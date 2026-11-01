//! Windows platform impl: window init/cleanup, fullscreen toggle helpers,
//! scale + geometry queries, and the WndProc hook that resamples the window.
//!
//! All `g_win` state (HWND, the minimize edge, the maximize-restore flag, the
//! WndProc hook handle, the input thread JoinHandle) lives in this module
//! behind a `Mutex<WinState>`. Scale, position, and window mode are not stored
//! here: they come from Win32, through `crate::window`'s sample.

#![allow(non_snake_case)]

use parking_lot::Mutex;
use std::ffi::{c_int, c_void};
use std::thread::JoinHandle;

use windows::Win32::Foundation::{
    GetLastError, HWND, LPARAM, LRESULT, RECT, SetLastError, WIN32_ERROR, WPARAM,
};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::Accessibility::{HWINEVENTHOOK, SetWinEventHook, UnhookWinEvent};
use windows::Win32::UI::HiDpi::GetDpiForSystem;
use windows::Win32::UI::WindowsAndMessaging::{
    CWPRETSTRUCT, CallNextHookEx, CallWindowProcW, DefWindowProcW, EVENT_SYSTEM_FOREGROUND,
    GWL_STYLE, GWLP_WNDPROC, GetWindowLongPtrW, GetWindowThreadProcessId, HHOOK, HTCAPTION,
    HWND_NOTOPMOST, HWND_TOPMOST, IsZoomed, MINMAXINFO, PostMessageW, SC_MOVE, SIZE_MINIMIZED,
    SPI_GETWORKAREA, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_SHOWWINDOW,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowLongPtrW, SetWindowPos, SetWindowsHookExW,
    SystemParametersInfoW, UnhookWindowsHookEx, WH_CALLWNDPROCRET, WINDOWPOS,
    WINEVENT_OUTOFCONTEXT, WINEVENT_SKIPOWNPROCESS, WM_CLOSE, WM_DPICHANGED, WM_GETMINMAXINFO,
    WM_MOVE, WM_SIZE, WM_SIZING, WM_STYLECHANGED, WM_SYSCOMMAND, WM_WINDOWPOSCHANGING,
    WMSZ_BOTTOMLEFT, WMSZ_LEFT, WMSZ_TOP, WMSZ_TOPLEFT, WMSZ_TOPRIGHT, WNDPROC, WS_CAPTION,
    WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

use jfn_mpv::api::{
    jfn_mpv_set_fullscreen, jfn_mpv_set_window_maximized, jfn_mpv_set_window_minimized,
    jfn_mpv_toggle_fullscreen,
};
use jfn_mpv::boot::jfn_mpv_handle_get;
use jfn_platform_abi::PhysicalSize;
use jfn_platform_abi::geometry::{Bounds, WindowGeometry, clamp_to_bounds};
use jfn_platform_abi::picture_in_picture::{
    DEFAULT_SCREEN_FRACTION, fit_display_fraction, minimum_control_size,
};
use jfn_playback::shutdown::jfn_shutdown_initiate;

use crate::input::{
    jfn_input_windows_resize_to_parent, jfn_input_windows_run_input_thread,
    jfn_input_windows_stop_input_thread,
};

struct WinState {
    mpv_hwnd_raw: usize,
    was_minimized: bool,
    restore_maximized_on_unfullscreen: bool,
    wndproc_hook_raw: usize,
    input_thread: Option<JoinHandle<()>>,
    picture_in_picture_restore: Option<(RECT, isize)>,
    picture_in_picture_minimum: Option<PhysicalSize>,
    picture_in_picture_wndproc: Option<isize>,
    picture_in_picture_subtitles: Option<(f64, bool, bool)>,
    picture_in_picture_foreground_hook: Option<usize>,
}

impl WinState {
    const fn new() -> Self {
        Self {
            mpv_hwnd_raw: 0,
            was_minimized: false,
            restore_maximized_on_unfullscreen: false,
            wndproc_hook_raw: 0,
            input_thread: None,
            picture_in_picture_restore: None,
            picture_in_picture_minimum: None,
            picture_in_picture_wndproc: None,
            picture_in_picture_subtitles: None,
            picture_in_picture_foreground_hook: None,
        }
    }
}

static STATE: Mutex<WinState> = Mutex::new(WinState::new());
type WinWndProc = unsafe extern "system" fn(HWND, u32, WPARAM, LPARAM) -> LRESULT;

unsafe extern "system" fn win_pip_foreground_changed(
    _hook: HWINEVENTHOOK,
    event: u32,
    foreground: HWND,
    _object: i32,
    _child: i32,
    _thread: u32,
    _time: u32,
) {
    if event != EVENT_SYSTEM_FOREGROUND {
        return;
    }
    let pip = {
        let state = STATE.lock();
        state
            .picture_in_picture_restore
            .map(|_| state.mpv_hwnd_raw)
            .filter(|raw| *raw != 0)
    };
    let Some(pip) = pip.map(hwnd_from_raw) else {
        return;
    };
    if pip.0 == foreground.0 {
        return;
    }
    let _ = unsafe {
        SetWindowPos(
            pip,
            Some(HWND_TOPMOST),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE,
        )
    };
}

fn win_read_pip_subtitle_settings() -> Option<(f64, bool, bool)> {
    let mut scale = 0.0;
    let mut scale_by_window = 0;
    let mut scale_with_window = 0;
    let success = unsafe {
        jfn_mpv::api::jfn_mpv_get_property_double(c"sub-scale".as_ptr(), &mut scale) == 0
            && jfn_mpv::api::jfn_mpv_get_property_flag(
                c"sub-scale-by-window".as_ptr(),
                &mut scale_by_window,
            ) == 0
            && jfn_mpv::api::jfn_mpv_get_property_flag(
                c"sub-scale-with-window".as_ptr(),
                &mut scale_with_window,
            ) == 0
    };
    success.then_some((scale, scale_by_window != 0, scale_with_window != 0))
}

fn win_set_pip_subtitle_settings(scale: f64, scale_by_window: bool, scale_with_window: bool) {
    jfn_mpv::api::jfn_mpv_set_option_flag("sub-scale-by-window", scale_by_window);
    jfn_mpv::api::jfn_mpv_set_option_flag("sub-scale-with-window", scale_with_window);
    jfn_mpv::api::jfn_mpv_set_option_double("sub-scale", scale);
}

fn hwnd_from_raw(raw: usize) -> HWND {
    HWND(raw as *mut c_void)
}

/// mpv's HWND, or `None` before it has been resolved / after cleanup.
pub(crate) fn win_hwnd() -> Option<HWND> {
    let raw = STATE.lock().mpv_hwnd_raw;
    (raw != 0).then(|| hwnd_from_raw(raw))
}

/// The stored HWND, taken from mpv's observed `window-id` on first use. The
/// boot wait pulls the window snapshot before `win_init` runs, so resolution
/// cannot wait for init. `None` until mpv's VO has a window.
///
/// Reads a cached atomic — no mpv property read, so no thread that pulls the
/// snapshot can serialize against the mpv core or mpv's VO GUI thread.
pub(crate) fn win_ensure_hwnd() -> Option<HWND> {
    if let Some(hwnd) = win_hwnd() {
        return Some(hwnd);
    }
    // The property observation is asynchronous. During startup, platform
    // initialization can run before the first observation event reaches the
    // playback ingest state, even though mpv has already created its native
    // window. Read the property synchronously as a startup fallback; later
    // changes continue to arrive through the normal observation path.
    let raw = jfn_playback::ingest_driver::jfn_playback_window_id().or_else(|| {
        let mut id = 0_i64;
        let result =
            unsafe { jfn_mpv::api::jfn_mpv_get_property_int(c"window-id".as_ptr(), &mut id) };
        (result >= 0 && id > 0).then_some(id)
    })? as usize;
    if raw == 0 {
        return None;
    }
    STATE.lock().mpv_hwnd_raw = raw;
    Some(hwnd_from_raw(raw))
}

/// True when mpv's window has neither `WS_CAPTION` nor `WS_THICKFRAME`.
///
/// Exact for every style mpv sets: `update_style` in
/// `third_party/mpv/video/out/w32_common.c` keeps `WS_THICKFRAME` in its
/// borderless-windowed set (NO_FRAME) and clears it only for fullscreen, and
/// mpv owns a top-level window here (no `--wid`), so the early-out for
/// embedded windows never applies.
pub(crate) fn win_is_fullscreen() -> bool {
    let Some(hwnd) = win_hwnd() else {
        return false;
    };
    let style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32;
    (style & WS_CAPTION.0) == 0 && (style & WS_THICKFRAME.0) == 0
}

/// `GetDpiForWindow`/96 once the window exists, `GetDpiForSystem`/96 before
/// it does.
pub(crate) fn win_get_scale() -> jfn_platform_abi::Scale {
    match crate::window::client_scale() {
        Some(scale) => scale,
        None => win_display_scale(),
    }
}

pub(crate) fn win_display_scale() -> jfn_platform_abi::Scale {
    crate::scale::report_dpi("GetDpiForSystem", unsafe { GetDpiForSystem() })
}

pub(crate) fn win_set_fullscreen(fullscreen: bool) {
    if jfn_mpv_handle_get().is_null() || win_is_fullscreen() == fullscreen {
        return;
    }
    let Some(hwnd) = win_hwnd() else {
        return;
    };

    if fullscreen {
        STATE.lock().restore_maximized_on_unfullscreen = unsafe { IsZoomed(hwnd) }.as_bool();
        jfn_mpv_set_window_minimized(false);
        jfn_mpv_set_fullscreen(true);
        return;
    }

    let should_restore_maximized =
        std::mem::take(&mut STATE.lock().restore_maximized_on_unfullscreen);
    jfn_mpv_set_fullscreen(false);
    if should_restore_maximized {
        jfn_mpv_set_window_maximized(true);
    }
}

pub(crate) fn win_toggle_fullscreen() {
    if jfn_mpv_handle_get().is_null() {
        return;
    }
    let Some(hwnd) = win_hwnd() else {
        return;
    };

    if !win_is_fullscreen() {
        STATE.lock().restore_maximized_on_unfullscreen = unsafe { IsZoomed(hwnd) }.as_bool();
        jfn_mpv_set_window_minimized(false);
        jfn_mpv_toggle_fullscreen();
        return;
    }

    let should_restore_maximized =
        std::mem::take(&mut STATE.lock().restore_maximized_on_unfullscreen);
    jfn_mpv_toggle_fullscreen();
    if should_restore_maximized {
        jfn_mpv_set_window_maximized(true);
    }
}

pub(crate) fn win_start_window_move() {
    if !jfn_platform_abi::picture_in_picture::active() {
        return;
    }
    let Some(hwnd) = win_hwnd() else {
        return;
    };
    let _ = unsafe {
        PostMessageW(
            Some(hwnd),
            WM_SYSCOMMAND,
            WPARAM((SC_MOVE | HTCAPTION) as usize),
            LPARAM(0),
        )
    };
}

pub(crate) fn win_set_picture_in_picture(enabled: bool, aspect_ratio: f64) {
    let Some(hwnd) = win_hwnd() else {
        return;
    };
    if enabled {
        let mut restore = RECT::default();
        if unsafe { windows::Win32::UI::WindowsAndMessaging::GetWindowRect(hwnd, &mut restore) }
            .is_err()
        {
            return;
        }
        let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if !unsafe { GetMonitorInfoW(monitor, &mut info) }.as_bool() {
            return;
        }
        let display = PhysicalSize {
            w: info.rcWork.right - info.rcWork.left,
            h: info.rcWork.bottom - info.rcWork.top,
        };
        let size = fit_display_fraction(display, aspect_ratio, DEFAULT_SCREEN_FRACTION);
        let minimum_size = minimum_control_size(display);
        let subtitle_settings = win_read_pip_subtitle_settings();
        let x = info.rcWork.right - size.w;
        let y = info.rcWork.bottom - size.h;
        let original_style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
        {
            let mut state = STATE.lock();
            if state.picture_in_picture_restore.is_some() {
                return;
            }
            state.picture_in_picture_restore = Some((restore, original_style));
            state.picture_in_picture_minimum = Some(minimum_size);
            state.picture_in_picture_subtitles = subtitle_settings;
        }
        if !install_pip_wndproc(hwnd) {
            let mut state = STATE.lock();
            state.picture_in_picture_restore = None;
            state.picture_in_picture_minimum = None;
            state.picture_in_picture_subtitles = None;
            return;
        }
        let pip_style = (original_style as u32
            & !(WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0))
            | WS_THICKFRAME.0;
        unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, pip_style as isize) };
        if !unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_TOPMOST),
                x,
                y,
                size.w,
                size.h,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
            )
        }
        .is_ok()
        {
            restore_pip_wndproc(hwnd);
            unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, original_style) };
            let _ = unsafe {
                SetWindowPos(
                    hwnd,
                    Some(HWND_NOTOPMOST),
                    restore.left,
                    restore.top,
                    restore.right - restore.left,
                    restore.bottom - restore.top,
                    SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
                )
            };
            let mut state = STATE.lock();
            state.picture_in_picture_restore = None;
            state.picture_in_picture_minimum = None;
            state.picture_in_picture_subtitles = None;
            return;
        }
        if let Some(settings) = subtitle_settings {
            // Compensate for mpv's window-relative font shrink at the initial
            // PiP size; keep window-relative scaling enabled for user resizing.
            let subtitle_scale =
                (settings.0 * (display.h as f64 / size.h as f64) * 0.8 * 0.8).min(100.0);
            win_set_pip_subtitle_settings(subtitle_scale, true, true);
        }
        jfn_platform_abi::picture_in_picture::notify(true);
        let hook = unsafe {
            SetWinEventHook(
                EVENT_SYSTEM_FOREGROUND,
                EVENT_SYSTEM_FOREGROUND,
                None,
                Some(win_pip_foreground_changed),
                0,
                0,
                WINEVENT_OUTOFCONTEXT | WINEVENT_SKIPOWNPROCESS,
            )
        };
        if !hook.is_invalid() {
            STATE.lock().picture_in_picture_foreground_hook = Some(hook.0 as usize);
        }
    } else {
        let (restore, original_style, minimum) = {
            let mut state = STATE.lock();
            let Some((restore, original_style)) = state.picture_in_picture_restore else {
                return;
            };
            (
                restore,
                original_style,
                state.picture_in_picture_minimum.take(),
            )
        };
        restore_pip_wndproc(hwnd);
        unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, original_style) };
        if unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                restore.left,
                restore.top,
                restore.right - restore.left,
                restore.bottom - restore.top,
                SWP_NOACTIVATE | SWP_SHOWWINDOW | SWP_FRAMECHANGED,
            )
        }
        .is_ok()
        {
            let mut state = STATE.lock();
            let subtitle_settings = state.picture_in_picture_subtitles.take();
            let foreground_hook = state
                .picture_in_picture_foreground_hook
                .take()
                .map(|raw| HWINEVENTHOOK(raw as *mut c_void));
            state.picture_in_picture_restore = None;
            state.picture_in_picture_minimum = None;
            drop(state);
            if let Some(hook) = foreground_hook {
                let _ = unsafe { UnhookWinEvent(hook) };
            }
            if let Some(settings) = subtitle_settings {
                win_set_pip_subtitle_settings(settings.0, settings.1, settings.2);
            }
            jfn_platform_abi::picture_in_picture::notify(false);
        } else {
            STATE.lock().picture_in_picture_minimum = minimum;
            let current_style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
            let pip_style = (current_style as u32
                & !(WS_CAPTION.0 | WS_SYSMENU.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0))
                | WS_THICKFRAME.0;
            unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, pip_style as isize) };
            let _ = install_pip_wndproc(hwnd);
        }
    }
}

fn constrain_pip_sizing_rect(rect: &mut RECT, edge: u32, minimum: PhysicalSize) {
    if rect.right - rect.left < minimum.w {
        match edge {
            WMSZ_LEFT | WMSZ_TOPLEFT | WMSZ_BOTTOMLEFT => rect.left = rect.right - minimum.w,
            _ => rect.right = rect.left + minimum.w,
        }
    }
    if rect.bottom - rect.top < minimum.h {
        match edge {
            WMSZ_TOP | WMSZ_TOPLEFT | WMSZ_TOPRIGHT => rect.top = rect.bottom - minimum.h,
            _ => rect.bottom = rect.top + minimum.h,
        }
    }
}

fn install_pip_wndproc(hwnd: HWND) -> bool {
    unsafe { SetLastError(WIN32_ERROR(0)) };
    let original = unsafe {
        SetWindowLongPtrW(
            hwnd,
            GWLP_WNDPROC,
            pip_wndproc as *const () as usize as isize,
        )
    };
    if original == 0 && unsafe { GetLastError() } != WIN32_ERROR(0) {
        tracing::error!("could not install the PiP window procedure");
        return false;
    }
    STATE.lock().picture_in_picture_wndproc = Some(original);
    true
}

fn restore_pip_wndproc(hwnd: HWND) {
    let original = STATE.lock().picture_in_picture_wndproc.take();
    if let Some(original) = original {
        unsafe { SetWindowLongPtrW(hwnd, GWLP_WNDPROC, original) };
    }
}

unsafe extern "system" fn pip_wndproc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let (target, minimum, original) = {
        let state = STATE.lock();
        (
            state.mpv_hwnd_raw == hwnd.0 as usize,
            state.picture_in_picture_minimum,
            state.picture_in_picture_wndproc,
        )
    };
    if target && let Some(minimum) = minimum {
        match message {
            WM_GETMINMAXINFO if lparam.0 != 0 => {
                let minmax = unsafe { &mut *(lparam.0 as *mut MINMAXINFO) };
                minmax.ptMinTrackSize.x = minmax.ptMinTrackSize.x.max(minimum.w);
                minmax.ptMinTrackSize.y = minmax.ptMinTrackSize.y.max(minimum.h);
            }
            WM_SIZING if lparam.0 != 0 => {
                let rect = unsafe { &mut *(lparam.0 as *mut RECT) };
                constrain_pip_sizing_rect(rect, wparam.0 as u32, minimum);
            }
            WM_WINDOWPOSCHANGING if lparam.0 != 0 => {
                let pos = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
                if !pos.flags.contains(SWP_NOSIZE) {
                    pos.cx = pos.cx.max(minimum.w);
                    pos.cy = pos.cy.max(minimum.h);
                }
            }
            _ => {}
        }
    }
    let Some(original) = original else {
        return unsafe { DefWindowProcW(hwnd, message, wparam, lparam) };
    };
    let original: WNDPROC = Some(unsafe { std::mem::transmute::<isize, WinWndProc>(original) });
    unsafe { CallWindowProcW(original, hwnd, message, wparam, lparam) }
}

unsafe extern "system" fn mpv_wndproc_hook(n_code: c_int, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if n_code >= 0 {
        let msg = unsafe { &*(lp.0 as *const CWPRETSTRUCT) };
        let target_hwnd_raw = STATE.lock().mpv_hwnd_raw;
        if (msg.hwnd.0 as usize) == target_hwnd_raw {
            match msg.message {
                WM_SIZE if msg.wParam.0 == SIZE_MINIMIZED as usize => {
                    if !std::mem::replace(&mut STATE.lock().was_minimized, true) {
                        jfn_playback::lifecycle::jfn_lifecycle_set_visible(false);
                    }
                }
                WM_SIZE => {
                    let restored = std::mem::replace(&mut STATE.lock().was_minimized, false);
                    if let Some(client) = crate::window::publish_deferred() {
                        jfn_input_windows_resize_to_parent(client.w, client.h);
                    }
                    if restored {
                        jfn_playback::lifecycle::jfn_lifecycle_set_visible(true);
                    }
                }
                WM_MOVE => {
                    crate::window::sample();
                }
                WM_DPICHANGED | WM_STYLECHANGED => {
                    crate::window::publish_deferred();
                }
                WM_CLOSE => jfn_shutdown_initiate(),
                _ => {}
            }
        }
    }
    let hook_raw = STATE.lock().wndproc_hook_raw;
    let hook = HHOOK(hook_raw as *mut c_void);
    unsafe { CallNextHookEx(Some(hook), n_code, wp, lp) }
}

pub(crate) fn win_early_init() {}

pub(crate) fn win_init(_mpv: *mut c_void) -> Result<(), jfn_platform_abi::PlatformInitError> {
    let Some(hwnd) = win_ensure_hwnd() else {
        return Err(jfn_platform_abi::PlatformInitError::backend(
            "Windows window acquisition",
            "no observed mpv window-id",
        ));
    };
    let hwnd_raw = hwnd.0 as usize;
    crate::window::republish();

    crate::render::init(hwnd_from_raw(hwnd_raw))?;

    crate::window::start_notifier();
    let mpv_tid = unsafe { GetWindowThreadProcessId(hwnd_from_raw(hwnd_raw), None) };
    let hook =
        unsafe { SetWindowsHookExW(WH_CALLWNDPROCRET, Some(mpv_wndproc_hook), None, mpv_tid) };
    match hook {
        Ok(h) => STATE.lock().wndproc_hook_raw = h.0 as usize,
        Err(e) => {
            return Err(jfn_platform_abi::PlatformInitError::backend(
                "SetWindowsHookExW(WH_CALLWNDPROCRET)",
                e,
            ));
        }
    }
    let mpv_hwnd_for_thread = hwnd_raw;
    let join = std::thread::Builder::new()
        .name("jfn-input".into())
        .spawn(move || {
            jfn_input_windows_run_input_thread(mpv_hwnd_for_thread as *mut c_void);
        })
        .map_err(|e| {
            jfn_platform_abi::PlatformInitError::backend("Windows input thread spawn", e)
        })?;
    STATE.lock().input_thread = Some(join);

    crate::window::republish();
    tracing::info!("Windows DirectComposition compositor initialized");
    Ok(())
}

pub(crate) fn win_cleanup() {
    if let Some(hwnd) = win_hwnd() {
        restore_pip_wndproc(hwnd);
    }
    let foreground_hook = STATE
        .lock()
        .picture_in_picture_foreground_hook
        .take()
        .map(|raw| HWINEVENTHOOK(raw as *mut c_void));
    if let Some(hook) = foreground_hook {
        let _ = unsafe { UnhookWinEvent(hook) };
    }
    jfn_input_windows_stop_input_thread();
    let join = STATE.lock().input_thread.take();
    if let Some(j) = join {
        let _ = j.join();
    }
    let hook_raw = STATE.lock().wndproc_hook_raw;
    if hook_raw != 0 {
        let hook = HHOOK(hook_raw as *mut c_void);
        unsafe {
            let _ = UnhookWindowsHookEx(hook);
        }
        STATE.lock().wndproc_hook_raw = 0;
    }
    crate::window::stop_notifier();

    crate::render::cleanup();
    crate::window::clear();
    STATE.lock().mpv_hwnd_raw = 0;
}

/// Resolve saved geometry against the primary monitor's working area so the
/// window never opens larger than the screen or off-screen, and center any
/// unset axis.
pub(crate) fn win_clamp_window_geometry(
    w: &mut c_int,
    h: &mut c_int,
    x: &mut c_int,
    y: &mut c_int,
) {
    let mut work = RECT::default();
    let ok = unsafe {
        SystemParametersInfoW(
            SPI_GETWORKAREA,
            0,
            Some(&mut work as *mut RECT as *mut c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
    };
    if ok.is_err() {
        return;
    }
    let vw = work.right - work.left;
    let vh = work.bottom - work.top;
    let mut g = WindowGeometry::from_raw(*w, *h, *x, *y);
    clamp_to_bounds(&mut g, Bounds { w: vw, h: vh });
    *w = g.w;
    *h = g.h;
    let (nx, ny) = g.raw_position();
    *x = nx;
    *y = ny;
}
