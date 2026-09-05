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

use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO, MonitorFromWindow,
};
use windows::Win32::UI::HiDpi::{AdjustWindowRectExForDpi, GetDpiForSystem, GetDpiForWindow};
use windows::Win32::UI::WindowsAndMessaging::{
    CWPRETSTRUCT, CWPSTRUCT, CallNextHookEx, GWL_EXSTYLE, GWL_STYLE, GetWindowLongPtrW,
    GetWindowPlacement, GetWindowRect, GetWindowThreadProcessId, HHOOK, HWND_NOTOPMOST,
    HWND_TOPMOST, IsZoomed, MINMAXINFO, SET_WINDOW_POS_FLAGS, SIZE_MINIMIZED, SPI_GETWORKAREA,
    SW_RESTORE, SWP_FRAMECHANGED, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE,
    SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS, SetWindowLongPtrW, SetWindowPlacement, SetWindowPos,
    SetWindowsHookExW, ShowWindow, SystemParametersInfoW, UnhookWindowsHookEx, WH_CALLWNDPROC,
    WH_CALLWNDPROCRET, WINDOW_EX_STYLE, WINDOW_STYLE, WINDOWPLACEMENT, WM_ACTIVATE, WM_CLOSE,
    WM_DPICHANGED, WM_GETMINMAXINFO, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MBUTTONDBLCLK, WM_MBUTTONDOWN, WM_MBUTTONUP, WM_MOUSEACTIVATE, WM_MOVE, WM_RBUTTONDBLCLK,
    WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SETFOCUS, WM_SIZE, WM_SIZING, WM_STYLECHANGED, WM_XBUTTONDOWN,
    WM_XBUTTONUP, WMSZ_BOTTOM, WMSZ_BOTTOMLEFT, WMSZ_LEFT, WMSZ_RIGHT, WMSZ_TOP, WMSZ_TOPLEFT,
    WMSZ_TOPRIGHT, WS_CAPTION, WS_MAXIMIZEBOX, WS_MINIMIZEBOX, WS_SYSMENU, WS_THICKFRAME,
};

use jfn_mpv::api::{
    jfn_mpv_set_fullscreen, jfn_mpv_set_window_maximized, jfn_mpv_set_window_minimized,
    jfn_mpv_toggle_fullscreen,
};
use jfn_mpv::boot::jfn_mpv_handle_get;
use jfn_platform_abi::geometry::{Bounds, WindowGeometry, clamp_to_bounds};
use jfn_platform_abi::picture_in_picture::{
    DEFAULT_SCREEN_FRACTION, MAXIMUM_SCREEN_FRACTION, MINIMUM_SCREEN_FRACTION, fit_display_fraction,
};
use jfn_platform_abi::{PhysicalSize, picture_in_picture};
use jfn_playback::shutdown::jfn_shutdown_initiate;

use crate::input::{
    jfn_input_windows_resize_to_parent, jfn_input_windows_run_input_thread,
    jfn_input_windows_stop_input_thread,
};

struct WinState {
    mpv_hwnd_raw: usize,
    was_minimized: bool,
    restore_maximized_on_unfullscreen: bool,
    wndproc_pre_hook_raw: usize,
    wndproc_hook_raw: usize,
    input_thread: Option<JoinHandle<()>>,
    picture_in_picture: Option<PictureInPictureState>,
    pending_picture_in_picture_aspect: Option<f64>,
}

#[derive(Clone, Copy)]
struct PictureInPictureState {
    restore: WINDOWPLACEMENT,
    restore_rect: RECT,
    window_style: isize,
    minimum_client: PhysicalSize,
    maximum_client: PhysicalSize,
    frame: PhysicalSize,
    aspect_ratio: f64,
}

impl WinState {
    const fn new() -> Self {
        Self {
            mpv_hwnd_raw: 0,
            was_minimized: false,
            restore_maximized_on_unfullscreen: false,
            wndproc_pre_hook_raw: 0,
            wndproc_hook_raw: 0,
            input_thread: None,
            picture_in_picture: None,
            pending_picture_in_picture_aspect: None,
        }
    }
}

static STATE: Mutex<WinState> = Mutex::new(WinState::new());

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
    let raw = jfn_playback::ingest_driver::jfn_playback_window_id()? as usize;
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

/// The window's own DPI once it exists, the system DPI before it does.
pub(crate) fn win_get_scale() -> f32 {
    match crate::window::client_scale() {
        Some(scale) => scale.or_one().0,
        None => system_scale(),
    }
}

pub(crate) fn win_get_display_scale(_x: c_int, _y: c_int) -> f32 {
    system_scale()
}

fn system_scale() -> f32 {
    let dpi = unsafe { GetDpiForSystem() };
    if dpi > 0 { dpi as f32 / 96.0 } else { 1.0 }
}

pub(crate) fn win_set_fullscreen(fullscreen: bool) {
    if jfn_mpv_handle_get().is_null() || win_is_fullscreen() == fullscreen {
        return;
    }
    let Some(hwnd) = win_hwnd() else {
        return;
    };

    if fullscreen {
        win_set_picture_in_picture(false, 1.0);
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
        win_set_picture_in_picture(false, 1.0);
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

fn monitor_work_area(hwnd: HWND) -> Option<RECT> {
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        ..Default::default()
    };
    unsafe { GetMonitorInfoW(monitor, &mut info) }
        .as_bool()
        .then_some(info.rcWork)
}

fn outer_size_for_client(hwnd: HWND, client: PhysicalSize) -> Option<PhysicalSize> {
    let style = WINDOW_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) } as u32);
    let ex_style = WINDOW_EX_STYLE(unsafe { GetWindowLongPtrW(hwnd, GWL_EXSTYLE) } as u32);
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: client.w,
        bottom: client.h,
    };
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    unsafe { AdjustWindowRectExForDpi(&mut rect, style, false, ex_style, dpi) }.ok()?;
    Some(PhysicalSize {
        w: rect.right - rect.left,
        h: rect.bottom - rect.top,
    })
}

fn enter_picture_in_picture(hwnd: HWND, aspect_ratio: f64) {
    if STATE.lock().picture_in_picture.is_some() {
        return;
    }
    let Some(work) = monitor_work_area(hwnd) else {
        return;
    };
    let display = PhysicalSize {
        w: work.right - work.left,
        h: work.bottom - work.top,
    };
    let default_client = fit_display_fraction(display, aspect_ratio, DEFAULT_SCREEN_FRACTION);
    let minimum_client = fit_display_fraction(display, aspect_ratio, MINIMUM_SCREEN_FRACTION);
    let maximum_client = fit_display_fraction(display, aspect_ratio, MAXIMUM_SCREEN_FRACTION);
    let mut restore = WINDOWPLACEMENT {
        length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
        ..Default::default()
    };
    if unsafe { GetWindowPlacement(hwnd, &mut restore) }.is_err() {
        return;
    }
    let mut restore_rect = RECT::default();
    if unsafe { GetWindowRect(hwnd, &mut restore_rect) }.is_err() {
        return;
    }

    // Preserve the placement, then remove the caption and system buttons. The
    // thick frame remains as an invisible resize target; with no caption or
    // maximize affordance Windows does not offer snap layouts for the PiP.
    let window_style = unsafe { GetWindowLongPtrW(hwnd, GWL_STYLE) };
    let _ = unsafe { ShowWindow(hwnd, SW_RESTORE) };
    let pip_style = window_style
        & !((WS_CAPTION.0 | WS_MINIMIZEBOX.0 | WS_MAXIMIZEBOX.0 | WS_SYSMENU.0) as isize);
    unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, pip_style) };
    let Some(default_outer) = outer_size_for_client(hwnd, default_client) else {
        unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, window_style) };
        let _ = unsafe { SetWindowPlacement(hwnd, &restore) };
        return;
    };
    let frame = PhysicalSize {
        w: default_outer.w - default_client.w,
        h: default_outer.h - default_client.h,
    };
    let dpi = unsafe { GetDpiForWindow(hwnd) }.max(96);
    let margin = ((16 * dpi) / 96) as i32;
    let x = work.right - default_outer.w - margin;
    let y = work.bottom - default_outer.h - margin;
    STATE.lock().picture_in_picture = Some(PictureInPictureState {
        restore,
        restore_rect,
        window_style,
        minimum_client,
        maximum_client,
        frame,
        aspect_ratio: f64::from(default_client.w) / f64::from(default_client.h),
    });
    if unsafe {
        SetWindowPos(
            hwnd,
            Some(HWND_TOPMOST),
            x,
            y,
            default_outer.w,
            default_outer.h,
            SET_WINDOW_POS_FLAGS(SWP_NOACTIVATE.0 | SWP_FRAMECHANGED.0),
        )
    }
    .is_err()
    {
        STATE.lock().picture_in_picture = None;
        unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, window_style) };
        let _ = unsafe { SetWindowPlacement(hwnd, &restore) };
        return;
    }
    picture_in_picture::notify(true);
}

fn reassert_picture_in_picture_topmost(hwnd: HWND) {
    if STATE.lock().picture_in_picture.is_none() {
        return;
    }
    let flags = SET_WINDOW_POS_FLAGS(SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOACTIVATE.0);
    let _ = unsafe { SetWindowPos(hwnd, Some(HWND_TOPMOST), 0, 0, 0, 0, flags) };
}

fn should_reassert_picture_in_picture_topmost(message: u32) -> bool {
    matches!(
        message,
        WM_ACTIVATE
            | WM_SETFOCUS
            | WM_MOUSEACTIVATE
            | WM_LBUTTONDOWN
            | WM_LBUTTONUP
            | WM_LBUTTONDBLCLK
            | WM_RBUTTONDOWN
            | WM_RBUTTONUP
            | WM_RBUTTONDBLCLK
            | WM_MBUTTONDOWN
            | WM_MBUTTONUP
            | WM_MBUTTONDBLCLK
            | WM_XBUTTONDOWN
            | WM_XBUTTONUP
            | WM_STYLECHANGED
    )
}

fn leave_picture_in_picture(hwnd: HWND, restore_placement: bool) {
    let state = STATE.lock().picture_in_picture.take();
    let Some(state) = state else {
        return;
    };
    unsafe { SetWindowLongPtrW(hwnd, GWL_STYLE, state.window_style) };
    if restore_placement {
        // Restore the exact outer rectangle as well as WINDOWPLACEMENT. mpv's
        // normal-window placement can be updated by the PiP SetWindowPos;
        // relying on SetWindowPlacement alone can therefore leave the window
        // at PiP dimensions after the caption is restored.
        let rect = state.restore_rect;
        let flags = SET_WINDOW_POS_FLAGS(SWP_NOACTIVATE.0 | SWP_FRAMECHANGED.0);
        let _ = unsafe {
            SetWindowPos(
                hwnd,
                Some(HWND_NOTOPMOST),
                rect.left,
                rect.top,
                rect.right - rect.left,
                rect.bottom - rect.top,
                flags,
            )
        };
        let _ = unsafe { SetWindowPlacement(hwnd, &state.restore) };
        crate::window::sample();
    } else {
        let flags = SET_WINDOW_POS_FLAGS(
            SWP_NOMOVE.0 | SWP_NOSIZE.0 | SWP_NOACTIVATE.0 | SWP_FRAMECHANGED.0,
        );
        let _ = unsafe { SetWindowPos(hwnd, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags) };
    }
    picture_in_picture::notify(false);
}

pub(crate) fn win_set_picture_in_picture(enabled: bool, aspect_ratio: f64) {
    let Some(hwnd) = win_hwnd() else {
        return;
    };
    if !enabled {
        STATE.lock().pending_picture_in_picture_aspect = None;
        leave_picture_in_picture(hwnd, true);
        return;
    }
    if STATE.lock().picture_in_picture.is_some() {
        return;
    }
    if win_is_fullscreen() {
        STATE.lock().pending_picture_in_picture_aspect = Some(aspect_ratio);
        win_set_fullscreen(false);
        return;
    }
    enter_picture_in_picture(hwnd, aspect_ratio);
}

fn constrain_picture_in_picture_rect(rect: &mut RECT, edge: usize, pip: PictureInPictureState) {
    let candidate_w = (rect.right - rect.left - pip.frame.w).max(1);
    let candidate_h = (rect.bottom - rect.top - pip.frame.h).max(1);
    let width_from_height = (f64::from(candidate_h) * pip.aspect_ratio).round() as i32;
    let height_from_width = (f64::from(candidate_w) / pip.aspect_ratio).round() as i32;
    let horizontal_edge = edge == WMSZ_LEFT as usize || edge == WMSZ_RIGHT as usize;
    let vertical_edge = edge == WMSZ_TOP as usize || edge == WMSZ_BOTTOM as usize;
    let corner_follows_height = !horizontal_edge
        && !vertical_edge
        && (width_from_height - candidate_w).abs() < (height_from_width - candidate_h).abs();
    let mut client_w = if vertical_edge || corner_follows_height {
        width_from_height
    } else {
        candidate_w
    };
    client_w = client_w.clamp(pip.minimum_client.w, pip.maximum_client.w);
    let client_h = (f64::from(client_w) / pip.aspect_ratio).round() as i32;
    let outer_w = client_w + pip.frame.w;
    let outer_h = client_h + pip.frame.h;

    if edge == WMSZ_LEFT as usize
        || edge == WMSZ_TOPLEFT as usize
        || edge == WMSZ_BOTTOMLEFT as usize
    {
        rect.left = rect.right - outer_w;
    } else {
        rect.right = rect.left + outer_w;
    }
    if edge == WMSZ_TOP as usize || edge == WMSZ_TOPLEFT as usize || edge == WMSZ_TOPRIGHT as usize
    {
        rect.top = rect.bottom - outer_h;
    } else {
        rect.bottom = rect.top + outer_h;
    }
}

unsafe extern "system" fn mpv_wndproc_pre_hook(n_code: c_int, wp: WPARAM, lp: LPARAM) -> LRESULT {
    if n_code >= 0 {
        let msg = unsafe { &*(lp.0 as *const CWPSTRUCT) };
        let state = STATE.lock();
        if (msg.hwnd.0 as usize) == state.mpv_hwnd_raw
            && let Some(pip) = state.picture_in_picture
        {
            match msg.message {
                WM_GETMINMAXINFO => {
                    let minmax = unsafe { &mut *(msg.lParam.0 as *mut MINMAXINFO) };
                    minmax.ptMinTrackSize = POINT {
                        x: pip.minimum_client.w + pip.frame.w,
                        y: pip.minimum_client.h + pip.frame.h,
                    };
                    minmax.ptMaxTrackSize = POINT {
                        x: pip.maximum_client.w + pip.frame.w,
                        y: pip.maximum_client.h + pip.frame.h,
                    };
                }
                WM_SIZING => {
                    let rect = unsafe { &mut *(msg.lParam.0 as *mut RECT) };
                    constrain_picture_in_picture_rect(rect, msg.wParam.0, pip);
                }
                _ => {}
            }
        }
    }
    let hook_raw = STATE.lock().wndproc_pre_hook_raw;
    let hook = HHOOK(hook_raw as *mut c_void);
    unsafe { CallNextHookEx(Some(hook), n_code, wp, lp) }
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
                    let pending_aspect = if msg.message == WM_STYLECHANGED && !win_is_fullscreen() {
                        STATE.lock().pending_picture_in_picture_aspect.take()
                    } else {
                        None
                    };
                    if let Some(aspect) = pending_aspect {
                        enter_picture_in_picture(msg.hwnd, aspect);
                    }
                }
                WM_CLOSE => jfn_shutdown_initiate(),
                _ => {}
            }
            if should_reassert_picture_in_picture_topmost(msg.message) {
                reassert_picture_in_picture_topmost(msg.hwnd);
            }
        }
    }
    let hook_raw = STATE.lock().wndproc_hook_raw;
    let hook = HHOOK(hook_raw as *mut c_void);
    unsafe { CallNextHookEx(Some(hook), n_code, wp, lp) }
}

pub(crate) fn win_early_init() {}

pub(crate) fn win_init(_mpv: *mut c_void) -> bool {
    let Some(hwnd) = win_ensure_hwnd() else {
        tracing::error!("mpv window handle unresolved; no observed window-id");
        return false;
    };
    let hwnd_raw = hwnd.0 as usize;
    crate::window::republish();

    if !crate::render::init(hwnd_from_raw(hwnd_raw)) {
        return false;
    }

    crate::window::start_notifier();
    let mpv_tid = unsafe { GetWindowThreadProcessId(hwnd_from_raw(hwnd_raw), None) };
    let pre_hook =
        unsafe { SetWindowsHookExW(WH_CALLWNDPROC, Some(mpv_wndproc_pre_hook), None, mpv_tid) };
    match pre_hook {
        Ok(h) => STATE.lock().wndproc_pre_hook_raw = h.0 as usize,
        Err(e) => {
            tracing::error!("SetWindowsHookExW(WH_CALLWNDPROC) failed: {e:?}");
            return false;
        }
    }
    let hook =
        unsafe { SetWindowsHookExW(WH_CALLWNDPROCRET, Some(mpv_wndproc_hook), None, mpv_tid) };
    match hook {
        Ok(h) => STATE.lock().wndproc_hook_raw = h.0 as usize,
        Err(e) => {
            tracing::error!("SetWindowsHookExW(WH_CALLWNDPROCRET) failed: {e:?}");
            let pre_hook = HHOOK(STATE.lock().wndproc_pre_hook_raw as *mut c_void);
            unsafe {
                let _ = UnhookWindowsHookEx(pre_hook);
            }
            STATE.lock().wndproc_pre_hook_raw = 0;
            return false;
        }
    }
    let mpv_hwnd_for_thread = hwnd_raw;
    let join = std::thread::spawn(move || {
        jfn_input_windows_run_input_thread(mpv_hwnd_for_thread as *mut c_void);
    });
    STATE.lock().input_thread = Some(join);

    crate::window::republish();
    tracing::info!("Windows DirectComposition compositor initialized");
    true
}

pub(crate) fn win_cleanup() {
    win_set_picture_in_picture(false, 1.0);
    jfn_input_windows_stop_input_thread();
    let join = STATE.lock().input_thread.take();
    if let Some(j) = join {
        let _ = j.join();
    }
    let pre_hook_raw = STATE.lock().wndproc_pre_hook_raw;
    if pre_hook_raw != 0 {
        let hook = HHOOK(pre_hook_raw as *mut c_void);
        unsafe {
            let _ = UnhookWindowsHookEx(hook);
        }
        STATE.lock().wndproc_pre_hook_raw = 0;
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
