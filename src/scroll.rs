//! Cross-platform momentum scrolling abstraction for GPUI and GPUI Kit.
//!
//! # Platform Momentum Scrolling Architecture
//!
//! - **macOS**: GPUI's AppKit backend translates macOS window server inertial events
//!   into an ongoing stream of `ScrollWheelEvent`s with `ScrollDelta::Pixels` and
//!   `TouchPhase::Moved` / `TouchPhase::Ended`. GPUI's `overflow_y_scroll()` and
//!   `ScrollHandle` therefore natively receive OS-level momentum without extra code.
//!   Layering a secondary software simulation on macOS causes duplicate acceleration
//!   and runaway inertia. Therefore, on macOS, this controller acts as a no-op
//!   pass-through, preserving native framework behavior.
//!
//! - **Linux (Wayland & X11) and Windows**: The host operating systems and window managers
//!   do not synthesize inertial scroll events after fingers lift from trackpads.
//!   GPUI's platform layers currently dispatch only raw physical deltas during active
//!   contact and do not generate momentum decay. To achieve parity with native desktop
//!   standards, this module provides a safe, minimal, framework-native fallback using
//!   standard GPUI public APIs (`ScrollHandle`, `ScrollWheelEvent`, and analytical
//!   exponential decay).
//!
//! - **Mouse Wheels vs Trackpads**: Conventional mouse-wheel input (`ScrollDelta::Lines`)
//!   is discrete and never receives trackpad inertia. Only continuous pixel deltas
//!   (`ScrollDelta::Pixels`) from trackpads or touch surfaces participate in velocity
//!   tracking and kinetic glide.
//!
//! # Upstream Tracking
//!
//! TODO: Replace and refine cross-platform momentum scrolling using official GPUI/GPUI Kit support.
//!
//! 1. Monitor official GPUI and GPUI Kit releases, documentation, and upstream changes
//!    for native momentum-scrolling support on Linux and Windows, as well as improvements
//!    to the existing macOS implementation.
//! 2. Reassess the temporary Linux/Windows fallback when official APIs become available.
//! 3. Replace the fallback with the official implementation wherever appropriate.
//! 4. Use the official API consistently across platforms while preserving platform-native
//!    behavior and user scrolling preferences.
//! 5. Remove obsolete fallback code and redundant abstractions after migration.
//! 6. Test mouse wheels, trackpads, momentum decay, scrolling responsiveness, and interaction
//!    with the library layout on all supported operating systems.
//! 7. Document the upstream capability, version or commit that enables the migration,
//!    implementation changes, and validation results.

#![forbid(unsafe_code)]

use std::time::Instant;

use gpui_kit::{ScrollDelta, ScrollHandle, ScrollWheelEvent, TouchPhase, point, px};

use crate::motion::{self, KineticScroll};

/// Assumed baseline frame interval (16ms ~ 60Hz) when no prior frame timestamp exists.
pub const DEFAULT_FRAME_DT: f32 = 0.016;

/// Central controller managing cross-platform momentum scrolling for a `ScrollHandle`.
#[derive(Debug, Clone, PartialEq)]
pub struct PlatformScrollController {
    /// Kinetic tracker state machine (velocity sampling + analytical exponential decay).
    pub kinetic: KineticScroll,
    /// Timestamp of the previous animation step frame.
    last_frame_time: Option<Instant>,
    /// Whether an inertial glide is actively stepping.
    is_animating: bool,
}

impl Default for PlatformScrollController {
    fn default() -> Self {
        Self::new()
    }
}

impl PlatformScrollController {
    /// Creates a new platform scroll controller.
    pub const fn new() -> Self {
        Self {
            kinetic: KineticScroll::new(),
            last_frame_time: None,
            is_animating: false,
        }
    }

    /// Whether this platform natively receives momentum scroll events from GPUI.
    #[inline(always)]
    pub const fn has_native_momentum() -> bool {
        cfg!(target_os = "macos")
    }

    /// Resets all momentum, velocity samples, and animation state.
    pub fn reset(&mut self) {
        self.kinetic.reset();
        self.last_frame_time = None;
        self.is_animating = false;
    }

    /// Cancels active inertia immediately (e.g. on finger touch-down or mouse press)
    /// without clearing historical velocity tracking.
    pub fn cancel_inertia(&mut self) {
        self.kinetic.cancel_inertia();
        self.last_frame_time = None;
        self.is_animating = false;
    }

    /// True if an inertial glide simulation is actively animating.
    #[inline]
    pub fn is_animating(&self) -> bool {
        self.is_animating
    }

    /// True if gesture velocity tracking is actively awaiting finger release/stillness timeout.
    #[inline]
    pub fn is_awaiting_release(&self) -> bool {
        self.kinetic.is_awaiting_release()
    }

    /// Ingests a `ScrollWheelEvent` for a given `ScrollHandle`.
    ///
    /// Returns `true` if an animation frame should be requested to service the kinetic glide.
    pub fn handle_wheel(
        &mut self,
        event: &ScrollWheelEvent,
        _scroll_handle: &ScrollHandle,
        animations_enabled: bool,
    ) -> bool {
        // On macOS, native momentum is provided by AppKit/GPUI. Do not layer software inertia.
        if Self::has_native_momentum() {
            return false;
        }

        // Discrete mouse wheel notches (ScrollDelta::Lines) must never trigger trackpad inertia.
        let (dx, dy) = match event.delta {
            ScrollDelta::Pixels(delta) => (f32::from(delta.x), f32::from(delta.y)),
            ScrollDelta::Lines(_) => {
                self.reset();
                return false;
            }
        };

        let now = Instant::now();

        // New gesture begins or cancelled: reset tracker.
        if event.touch_phase == TouchPhase::Started || event.touch_phase == TouchPhase::Cancelled {
            self.reset();
            return false;
        }

        // Track displacement while moving.
        if event.touch_phase != TouchPhase::Cancelled && (dx.abs() > 0.001 || dy.abs() > 0.001) {
            self.kinetic.push_motion(now, dx, dy);
        }

        // Check if an explicit Ended phase arrived to launch momentum.
        if event.touch_phase == TouchPhase::Ended {
            if animations_enabled && self.kinetic.end_touch(true) {
                self.is_animating = true;
                self.last_frame_time = Some(now);
                return true;
            } else {
                self.reset();
                return false;
            }
        }

        // On Linux / Windows, TouchPhase::Ended is not sent by window compositors.
        // Returning animations_enabled wakes the host frame clock and triggers render so
        // check_stillness_timeout can detect fingers lifted from the touchpad and trigger inertia.
        animations_enabled
    }

    /// Checks if a stillness timeout elapsed after touch movement on platforms where
    /// `TouchPhase::Ended` is not dispatched (such as Wayland or X11).
    ///
    /// Returns `true` if inertia was launched from the detected stillness.
    pub fn check_stillness_timeout(&mut self, now: Instant, animations_enabled: bool) -> bool {
        if Self::has_native_momentum() || !animations_enabled {
            return false;
        }

        if self.kinetic.check_stillness_timeout(now, true) {
            self.is_animating = true;
            self.last_frame_time = Some(now);
            true
        } else {
            false
        }
    }

    /// Steps the inertial simulation by one animation frame for the target `ScrollHandle`.
    ///
    /// Updates the `ScrollHandle` offset using the exact analytical integral displacement.
    /// Returns `true` if inertia is still active and requires another frame; `false` when finished.
    pub fn step(&mut self, now: Instant, scroll_handle: &ScrollHandle) -> bool {
        if Self::has_native_momentum() || !self.is_animating {
            self.is_animating = false;
            self.last_frame_time = None;
            return false;
        }

        let dt = if let Some(last) = self.last_frame_time {
            now.duration_since(last).as_secs_f32()
        } else {
            DEFAULT_FRAME_DT
        };
        self.last_frame_time = Some(now);

        let dt = motion::clamp_frame_dt(dt);

        if let Some((_dx, dy)) = self.kinetic.step(dt) {
            let current_offset = scroll_handle.offset();
            // Scroll offset in GPUI for vertical scrolling is negative downwards:
            // pulling down (positive dy) scrolls up (offset becomes less negative).
            let max_y = scroll_handle.max_offset().y;
            if max_y > px(0.0) {
                let raw_y = current_offset.y + px(dy);
                let new_y = raw_y.clamp(-max_y, px(0.0));
                scroll_handle.set_offset(point(current_offset.x, new_y));

                // Stop inertia if we hit the scroll boundary
                if (raw_y >= px(0.0) && dy > 0.0) || (raw_y <= -max_y && dy < 0.0) {
                    self.kinetic.cancel_inertia();
                }
            } else {
                self.kinetic.cancel_inertia();
            }

            if self.kinetic.has_active_inertia() {
                self.is_animating = true;
                return true;
            }
        }

        self.is_animating = false;
        self.last_frame_time = None;
        false
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn test_platform_detection() {
        if cfg!(target_os = "macos") {
            assert!(PlatformScrollController::has_native_momentum());
        } else {
            assert!(!PlatformScrollController::has_native_momentum());
        }
    }

    #[test]
    fn test_controller_state_transitions() {
        let mut controller = PlatformScrollController::new();
        assert!(!controller.is_animating());

        let now = Instant::now();
        // Record motion
        controller.kinetic.push_motion(now, 0.0, 50.0);
        controller
            .kinetic
            .push_motion(now + Duration::from_millis(16), 0.0, 75.0);

        // Stillness timeout should trigger inertia on Linux/Windows
        let trigger_time = now + Duration::from_millis(60);
        if !PlatformScrollController::has_native_momentum() {
            assert!(controller.check_stillness_timeout(trigger_time, true));
            assert!(controller.is_animating());

            controller.cancel_inertia();
            assert!(!controller.is_animating());
        }
    }
}
