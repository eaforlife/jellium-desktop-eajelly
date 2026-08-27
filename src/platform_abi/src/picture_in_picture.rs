//! Shared picture-in-picture state and aspect-preserving sizing.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;

use crate::PhysicalSize;

pub const DEFAULT_SCREEN_FRACTION: f64 = 0.25;
pub const MINIMUM_SCREEN_FRACTION: f64 = 0.15;
pub const MAXIMUM_SCREEN_FRACTION: f64 = 0.50;

static ACTIVE: AtomicBool = AtomicBool::new(false);
type Subscriber = Arc<dyn Fn(bool) + Send + Sync>;
static SUBSCRIBERS: Mutex<Vec<Subscriber>> = Mutex::new(Vec::new());

#[must_use]
pub fn active() -> bool {
    ACTIVE.load(Ordering::Acquire)
}

/// Publish a platform-confirmed PiP mode change.
pub fn notify(active: bool) {
    if ACTIVE.swap(active, Ordering::AcqRel) == active {
        return;
    }
    let subscribers = SUBSCRIBERS.lock().clone();
    for subscriber in subscribers {
        subscriber(active);
    }
}

/// Register a process-lifetime PiP mode listener.
pub fn subscribe(f: impl Fn(bool) + Send + Sync + 'static) {
    SUBSCRIBERS.lock().push(Arc::new(f));
}

/// Largest aspect-preserving size inside `fraction` of both display axes.
#[must_use]
pub fn fit_display_fraction(display: PhysicalSize, aspect: f64, fraction: f64) -> PhysicalSize {
    let aspect = if aspect.is_finite() && aspect > 0.0 {
        aspect
    } else {
        16.0 / 9.0
    };
    let fraction = fraction.clamp(0.01, 1.0);
    let max_w = (f64::from(display.w.max(1)) * fraction).round().max(1.0);
    let max_h = (f64::from(display.h.max(1)) * fraction).round().max(1.0);
    let (w, h) = if max_w / max_h > aspect {
        (max_h * aspect, max_h)
    } else {
        (max_w, max_w / aspect)
    };
    PhysicalSize {
        w: w.round().max(1.0) as i32,
        h: h.round().max(1.0) as i32,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn landscape_video_fits_quarter_of_display() {
        assert_eq!(
            fit_display_fraction(
                PhysicalSize { w: 1920, h: 1080 },
                16.0 / 9.0,
                DEFAULT_SCREEN_FRACTION,
            ),
            PhysicalSize { w: 480, h: 270 }
        );
    }

    #[test]
    fn portrait_video_is_limited_by_display_height() {
        assert_eq!(
            fit_display_fraction(
                PhysicalSize { w: 1920, h: 1080 },
                9.0 / 16.0,
                DEFAULT_SCREEN_FRACTION,
            ),
            PhysicalSize { w: 152, h: 270 }
        );
    }

    #[test]
    fn invalid_aspect_uses_widescreen_fallback() {
        assert_eq!(
            fit_display_fraction(
                PhysicalSize { w: 3840, h: 2160 },
                f64::NAN,
                MINIMUM_SCREEN_FRACTION,
            ),
            PhysicalSize { w: 576, h: 324 }
        );
    }

    #[test]
    fn landscape_video_fits_half_of_display() {
        assert_eq!(
            fit_display_fraction(
                PhysicalSize { w: 1920, h: 1080 },
                16.0 / 9.0,
                MAXIMUM_SCREEN_FRACTION,
            ),
            PhysicalSize { w: 960, h: 540 }
        );
    }
}
