//! Animation math for the reader. Pure functions and small state machines with no gpui types, so
//! they can be tested with simulated frame rates.
//!
//! Everything here is driven by the *time elapsed since the previous frame* (`dt`, in seconds)
//! rather than by a frame count, so an animation takes the same wall-clock time at 60 Hz, 144 Hz
//! or 240 Hz. No timers and no animation queue: callers advance the state once per rendered frame
//! and stop asking for frames when nothing is moving.

use std::time::{Duration, Instant};

/// How long a page turn slides for.
pub const PAGE_TURN_SECS: f32 = 0.16;
/// Page turns that interrupt a running one (holding a key, mashing the wheel) are quicker so the
/// reader keeps up instead of lagging behind the input.
pub const PAGE_TURN_INTERRUPTED_SECS: f32 = 0.09;
/// Time constant of the smooth-scroll glide: the remaining distance shrinks to 1/e (about 37%)
/// every this many seconds.
pub const SCROLL_TAU_SECS: f32 = 0.06;
/// Frames that take longer than this (window dragged, system hiccup) are treated as this long so
/// an animation never jumps.
pub const MAX_FRAME_SECS: f32 = 0.05;
/// A scroll closer than this (in logical pixels) to its target snaps to it.
const SNAP_PX: f32 = 0.05;

pub fn clamp_frame_dt(dt: f32) -> f32 {
    if dt.is_finite() {
        dt.clamp(0.0, MAX_FRAME_SECS)
    } else {
        0.0
    }
}

pub fn ease_out_cubic(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

/// Move `current` toward `target` exponentially. Because the decay is exponential, applying it in
/// many small steps gives the same result as one big step of the same total duration: the glide
/// is identical at any refresh rate.
pub fn smooth_toward(current: f32, target: f32, dt: f32, tau: f32) -> f32 {
    let delta = target - current;
    if delta.abs() <= SNAP_PX {
        return target;
    }
    let k = 1.0 - (-dt.max(0.0) / tau.max(1e-4)).exp();
    current + delta * k
}

/// One frame of smooth scrolling for both axes. Returns the new offset and whether it is still
/// moving (i.e. whether another frame is needed).
pub fn step_scroll(current: (f32, f32), target: (f32, f32), dt: f32) -> ((f32, f32), bool) {
    let next = (
        smooth_toward(current.0, target.0, dt, SCROLL_TAU_SECS),
        smooth_toward(current.1, target.1, dt, SCROLL_TAU_SECS),
    );
    (next, next != target)
}

/// Time constant for inertial velocity decay (seconds).
///
/// Natural exponential friction: velocity decays to 1/e every ~0.32 seconds.
pub const INERTIA_TAU_SECS: f32 = 0.32;
/// Velocity below this (in logical px/s) is considered stopped.
pub const INERTIA_STOP_VELOCITY: f32 = 8.0;
/// Minimum initial flick velocity (in logical px/s) to trigger an inertial glide.
pub const INERTIA_MIN_VELOCITY: f32 = 80.0;

/// Rolling-window velocity estimator for touchpad and touchscreen gestures.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VelocityTracker {
    samples: [(f32, f32, f32); 8],
    count: usize,
}

impl Default for VelocityTracker {
    fn default() -> Self {
        Self::new()
    }
}

impl VelocityTracker {
    pub const fn new() -> Self {
        Self {
            samples: [(0.0, 0.0, 0.0); 8],
            count: 0,
        }
    }

    pub fn reset(&mut self) {
        self.count = 0;
    }

    /// Record a motion displacement `(dx, dy)` over `dt` seconds.
    pub fn push(&mut self, dt: f32, dx: f32, dy: f32) {
        if dt <= 0.0 || !dt.is_finite() {
            return;
        }
        let idx = self.count % 8;
        self.samples[idx] = (dt, dx, dy);
        self.count += 1;
    }

    /// Estimated exit velocity (pixels per second) based on recent samples.
    pub fn velocity(&self) -> (f32, f32) {
        let n = self.count.min(8);
        if n < 2 {
            return (0.0, 0.0);
        }
        let mut total_dt = 0.0;
        let mut weighted_dt = 0.0;
        let mut weighted_dx = 0.0;
        let mut weighted_dy = 0.0;
        for i in 0..n {
            let idx = (self.count + 8 - 1 - i) % 8;
            let (dt, dx, dy) = self.samples[idx];
            // Exponential recency weighting: recent samples (i=0) have full weight (1.0),
            // older samples decay smoothly so exit flick speed is preserved without drop cliff.
            let weight = (-(i as f32) * 0.5).exp();
            total_dt += dt;
            weighted_dt += dt * weight;
            weighted_dx += dx * weight;
            weighted_dy += dy * weight;
            if total_dt >= 0.12 {
                break;
            }
        }
        if weighted_dt > 0.001 {
            (weighted_dx / weighted_dt, weighted_dy / weighted_dt)
        } else {
            (0.0, 0.0)
        }
    }
}

/// Kinetic / inertial scroll momentum simulation with analytical exponential decay.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InertialScroll {
    pub velocity: (f32, f32),
}

impl InertialScroll {
    pub fn new(velocity: (f32, f32)) -> Option<Self> {
        let speed = (velocity.0 * velocity.0 + velocity.1 * velocity.1).sqrt();
        if speed >= INERTIA_MIN_VELOCITY {
            Some(Self { velocity })
        } else {
            None
        }
    }

    /// Step the simulation forward by `dt` seconds.
    ///
    /// Returns the exact analytical integral displacement `(dx, dy)` for this frame and
    /// whether inertia is still running.
    pub fn step(&mut self, dt: f32) -> ((f32, f32), bool) {
        let dt = clamp_frame_dt(dt);
        let decay = (-dt / INERTIA_TAU_SECS).exp();

        // Exact analytical integral of v(t) = v0 * exp(-t / tau):
        // displacement = v0 * tau * (1 - exp(-dt / tau))
        let delta_x = self.velocity.0 * INERTIA_TAU_SECS * (1.0 - decay);
        let delta_y = self.velocity.1 * INERTIA_TAU_SECS * (1.0 - decay);

        self.velocity.0 *= decay;
        self.velocity.1 *= decay;

        let speed = (self.velocity.0 * self.velocity.0 + self.velocity.1 * self.velocity.1).sqrt();
        let still_moving = speed >= INERTIA_STOP_VELOCITY;
        if !still_moving {
            self.velocity = (0.0, 0.0);
        }

        ((delta_x, delta_y), still_moving)
    }
}

/// Stillness duration (fingers lifted off touchpad) before initiating inertia if no release event arrived.
/// Set to 35ms to ensure normal touchpad packet intervals (10-25ms) never prematurely trigger inertia
/// during active multi-touch gestures, eliminating stuttering on high-refresh displays.
pub const INERTIA_STILLNESS_THRESHOLD: Duration = Duration::from_millis(35);
/// Maximum interval after touch stillness where release momentum is still valid.
/// If more than this time has passed, the fingers were brought to a stop before lifting.
pub const INERTIA_MAX_RELEASE_GAP: Duration = Duration::from_millis(100);
/// Maximum interval between touch moves before treating the next move as a new gesture.
pub const INERTIA_GESTURE_TIMEOUT: Duration = Duration::from_millis(150);

/// Unified touch and drag gesture tracker and kinetic inertia state machine.
#[derive(Debug, Clone, PartialEq)]
pub struct KineticScroll {
    pub tracker: VelocityTracker,
    pub last_motion_time: Option<Instant>,
    pub inertia: Option<InertialScroll>,
}

impl Default for KineticScroll {
    fn default() -> Self {
        Self::new()
    }
}

impl KineticScroll {
    pub const fn new() -> Self {
        Self {
            tracker: VelocityTracker::new(),
            last_motion_time: None,
            inertia: None,
        }
    }

    /// Reset all momentum, samples, and gesture state.
    pub fn reset(&mut self) {
        self.tracker.reset();
        self.last_motion_time = None;
        self.inertia = None;
    }

    /// Cancel only the actively running inertial glide without resetting tracked velocity.
    pub fn cancel_inertia(&mut self) {
        self.inertia = None;
    }

    /// True if an inertial glide simulation is actively running.
    pub fn has_active_inertia(&self) -> bool {
        self.inertia.is_some()
    }

    /// True if a touch gesture was active and is awaiting a release event or stillness timeout.
    pub fn is_awaiting_release(&self) -> bool {
        self.last_motion_time.is_some()
    }

    /// True if either an inertial glide is running or a touch gesture is awaiting stillness timeout.
    pub fn is_active(&self) -> bool {
        self.inertia.is_some() || self.last_motion_time.is_some()
    }

    /// Record a motion displacement `(dx, dy)` at `now`.
    pub fn push_motion(&mut self, now: Instant, dx: f32, dy: f32) {
        let is_new_gesture = self
            .last_motion_time
            .is_none_or(|t| now.duration_since(t) > INERTIA_GESTURE_TIMEOUT);

        if is_new_gesture {
            self.tracker.reset();
            self.inertia = None;
        }

        let dt = if is_new_gesture {
            0.016
        } else {
            self.last_motion_time.map_or(0.016, |t| {
                now.duration_since(t).as_secs_f32().clamp(0.001, 0.1)
            })
        };
        self.last_motion_time = Some(now);
        self.tracker.push(dt, dx, dy);
    }

    /// Handle gesture completion (touch lifted). Starts inertia if allowed by preferences.
    /// Returns true if inertia was successfully initiated.
    pub fn end_touch(&mut self, allow_inertia: bool) -> bool {
        let now = Instant::now();
        let was_recent = self
            .last_motion_time
            .is_some_and(|t| now.duration_since(t) <= INERTIA_MAX_RELEASE_GAP);
        self.last_motion_time = None;
        if allow_inertia && was_recent {
            let velocity = self.tracker.velocity();
            self.inertia = InertialScroll::new(velocity);
            self.inertia.is_some()
        } else {
            self.tracker.reset();
            self.inertia = None;
            false
        }
    }

    /// Check if stillness timeout has elapsed (e.g. on Linux where `Ended` was omitted).
    /// If elapsed, initiates inertia from tracked velocity. Returns true if inertia was started.
    pub fn check_stillness_timeout(&mut self, now: Instant, allow_inertia: bool) -> bool {
        if self.inertia.is_none()
            && let Some(last_time) = self.last_motion_time
        {
            let elapsed = now.duration_since(last_time);
            if elapsed >= INERTIA_STILLNESS_THRESHOLD {
                self.last_motion_time = None;
                if allow_inertia && elapsed <= INERTIA_MAX_RELEASE_GAP {
                    let velocity = self.tracker.velocity();
                    self.inertia = InertialScroll::new(velocity);
                    return self.inertia.is_some();
                } else {
                    self.tracker.reset();
                }
            }
        }
        false
    }

    /// Advance active inertia by `dt` seconds.
    /// Returns `Some((dx, dy))` if inertia is active and stepped, or `None` if inactive.
    pub fn step(&mut self, dt: f32) -> Option<(f32, f32)> {
        if let Some(mut inertia) = self.inertia.take() {
            let (disp, still) = inertia.step(dt);
            if still {
                self.inertia = Some(inertia);
            }
            Some(disp)
        } else {
            None
        }
    }

    /// Update the remaining velocity vector (e.g. when clamping against boundaries).
    /// If speed drops below the stopping threshold, inertia is automatically ended.
    pub fn clamp_velocity(&mut self, clamp_x: bool, clamp_y: bool) {
        if let Some(inertia) = self.inertia.as_mut() {
            if clamp_x {
                inertia.velocity.0 = 0.0;
            }
            if clamp_y {
                inertia.velocity.1 = 0.0;
            }
            let speed = (inertia.velocity.0 * inertia.velocity.0
                + inertia.velocity.1 * inertia.velocity.1)
                .sqrt();
            if speed < INERTIA_STOP_VELOCITY {
                self.inertia = None;
            }
        }
    }
}

/// Which way the page content travels during a turn: `-1.0` means it moves left (the new page
/// slides in from the right), `+1.0` means it moves right.
///
/// Going forward moves content left, going back moves it right.
pub fn travel(forward: bool) -> f32 {
    if forward { -1.0 } else { 1.0 }
}

/// A page-turn slide. Shifts are expressed in viewport widths so a window resize cannot leave the
/// animation in the wrong place.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PageTransition {
    travel: f32,
    /// Where the outgoing page starts, in viewport widths. Non-zero only when this transition
    /// interrupts another one and takes over the page that was still sliding in.
    out_start: f32,
    duration: f32,
    elapsed: f32,
}

impl PageTransition {
    pub fn new(travel: f32, out_start: f32, duration: f32) -> Self {
        Self {
            travel,
            out_start,
            duration: duration.max(0.001),
            elapsed: 0.0,
        }
    }

    pub fn starting(travel: f32) -> Self {
        Self::new(travel, 0.0, PAGE_TURN_SECS)
    }

    /// The transition that replaces this one when another page turn arrives mid-slide. The page
    /// that was sliding in becomes the outgoing page, continuing from exactly where it is on
    /// screen right now. Nothing is queued: at most one transition ever exists.
    pub fn interrupted_by(&self, travel: f32) -> Self {
        Self::new(travel, self.incoming_shift(), PAGE_TURN_INTERRUPTED_SECS)
    }

    pub fn travel(&self) -> f32 {
        self.travel
    }

    pub fn advance(&mut self, dt: f32) {
        self.elapsed = (self.elapsed + dt.max(0.0)).min(self.duration);
    }

    pub fn is_done(&self) -> bool {
        self.elapsed >= self.duration
    }

    pub fn progress(&self) -> f32 {
        self.elapsed / self.duration
    }

    fn eased(&self) -> f32 {
        ease_out_cubic(self.progress())
    }

    /// Horizontal shift of the page that is leaving, in viewport widths.
    pub fn outgoing_shift(&self) -> f32 {
        self.out_start + (self.travel - self.out_start) * self.eased()
    }

    /// Horizontal shift of the page that is arriving, in viewport widths (0 once it has settled).
    pub fn incoming_shift(&self) -> f32 {
        -self.travel * (1.0 - self.eased())
    }
}

/// Parse `gsettings get org.gnome.desktop.interface enable-animations` output.
/// Returns `true` when the desktop asks for reduced motion (animations disabled).
pub fn gsettings_disables_animations(output: &str) -> bool {
    output.trim() == "false"
}

/// Whether the user (or their desktop) asks for reduced motion.
///
/// * `YOMU_REDUCE_MOTION=1` forces it on, `=0` forces it off.
/// * Otherwise GNOME's "Animations" switch is honoured via `gsettings`, when available.
///   (Desktops without that setting simply report no preference.)
pub fn detect_reduced_motion() -> bool {
    if let Ok(value) = std::env::var("YOMU_REDUCE_MOTION") {
        return matches!(value.trim(), "1" | "true" | "yes" | "on");
    }
    std::process::Command::new("gsettings")
        .args(["get", "org.gnome.desktop.interface", "enable-animations"])
        .output()
        .ok()
        .filter(|out| out.status.success())
        .map(|out| gsettings_disables_animations(&String::from_utf8_lossy(&out.stdout)))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run `steps` frames of `dt` and return the state after them.
    fn run_transition(mut t: PageTransition, hz: f32) -> (f32, usize) {
        let dt = 1.0 / hz;
        let mut frames = 0;
        while !t.is_done() {
            t.advance(dt);
            frames += 1;
            assert!(frames < 10_000);
        }
        (frames as f32 * dt, frames)
    }

    #[test]
    fn page_turn_takes_the_same_time_at_any_refresh_rate() {
        for hz in [30.0, 60.0, 90.0, 120.0, 144.0, 240.0, 360.0] {
            let (seconds, frames) = run_transition(PageTransition::starting(-1.0), hz);
            assert!(
                (seconds - PAGE_TURN_SECS).abs() <= 1.0 / hz + 1e-4,
                "{hz} Hz took {seconds}s ({frames} frames)"
            );
        }
    }

    #[test]
    fn slide_positions_agree_across_refresh_rates_at_the_same_moment() {
        // 50 ms in, sampled with 60 Hz frames and with 240 Hz frames.
        let mut slow = PageTransition::starting(-1.0);
        let mut fast = PageTransition::starting(-1.0);
        for _ in 0..3 {
            slow.advance(1.0 / 60.0);
        }
        for _ in 0..12 {
            fast.advance(1.0 / 240.0);
        }
        assert!((slow.outgoing_shift() - fast.outgoing_shift()).abs() < 1e-4);
        assert!((slow.incoming_shift() - fast.incoming_shift()).abs() < 1e-4);
    }

    #[test]
    fn pages_start_and_end_in_the_right_places() {
        let mut t = PageTransition::starting(-1.0);
        assert_eq!(t.outgoing_shift(), 0.0, "the old page starts where it is");
        assert_eq!(
            t.incoming_shift(),
            1.0,
            "the new page starts one viewport to the right"
        );
        t.advance(10.0);
        assert!(t.is_done());
        assert_eq!(
            t.outgoing_shift(),
            -1.0,
            "the old page ends fully off screen to the left"
        );
        assert_eq!(t.incoming_shift(), 0.0, "the new page ends at rest");
    }

    #[test]
    fn pages_stay_exactly_one_viewport_apart() {
        // Moving together: the gap between the two pages is constant, so they read as one strip.
        let mut t = PageTransition::starting(-1.0);
        for _ in 0..30 {
            t.advance(0.006);
            let gap = t.incoming_shift() - t.outgoing_shift();
            assert!((gap - 1.0).abs() < 1e-5, "gap {gap}");
        }
    }

    #[test]
    fn motion_is_monotonic_and_eased_out() {
        let mut t = PageTransition::starting(-1.0);
        let mut last = t.incoming_shift();
        let mut last_step = f32::MAX;
        for _ in 0..16 {
            t.advance(0.01);
            let now = t.incoming_shift();
            assert!(now <= last + 1e-6, "incoming page only moves toward rest");
            let step = (last - now).abs();
            assert!(step <= last_step + 1e-6, "ease-out: steps never grow");
            last_step = step;
            last = now;
        }
        assert_eq!(t.incoming_shift(), 0.0);
    }

    #[test]
    fn travel_follows_navigation_action() {
        assert_eq!(travel(true), -1.0, "forward: content moves left");
        assert_eq!(travel(false), 1.0, "back: content moves right");
    }

    #[test]
    fn interruption_continues_from_where_the_page_is_and_never_queues() {
        let mut first = PageTransition::starting(-1.0);
        first.advance(0.04);
        let on_screen = first.incoming_shift();
        assert!(on_screen > 0.0 && on_screen < 1.0);

        let second = first.interrupted_by(-1.0);
        assert_eq!(
            second.outgoing_shift(),
            on_screen,
            "no jump at the moment of interruption"
        );
        assert_eq!(second.incoming_shift(), 1.0);
        assert!(!second.is_done());

        // pressing back while sliding forward reverses smoothly
        let reversed = first.interrupted_by(1.0);
        assert_eq!(reversed.outgoing_shift(), on_screen);
        assert_eq!(reversed.incoming_shift(), -1.0);

        // ... and it is quicker than a normal turn
        let (seconds, _) = run_transition(second, 120.0);
        assert!(seconds < PAGE_TURN_SECS);
    }

    #[test]
    fn mashing_the_key_converges_instead_of_lagging() {
        // 40 presses, one every 25 ms, at 144 Hz: each interrupts the last.
        let dt = 1.0 / 144.0;
        let mut transition: Option<PageTransition> = None;
        let mut presses = 0;
        let mut time = 0.0f32;
        let mut next_press = 0.0f32;
        while presses < 40 || transition.is_some() {
            if presses < 40 && time >= next_press {
                transition = Some(match transition {
                    Some(t) => t.interrupted_by(-1.0),
                    None => PageTransition::starting(-1.0),
                });
                presses += 1;
                next_press += 0.025;
            }
            if let Some(mut t) = transition {
                t.advance(dt);
                transition = if t.is_done() { None } else { Some(t) };
            }
            time += dt;
            assert!(time < 3.0, "must settle");
        }
        assert!(
            time < 40.0 * 0.025 + PAGE_TURN_INTERRUPTED_SECS + 0.05,
            "settled {time}s"
        );
    }

    #[test]
    fn smooth_scroll_is_identical_at_every_refresh_rate() {
        for (hz, expected_frames) in [(60.0, 12), (144.0, 29), (240.0, 48)] {
            let dt = 1.0 / hz;
            let mut off = 0.0;
            for _ in 0..expected_frames {
                off = smooth_toward(off, 300.0, dt, SCROLL_TAU_SECS);
            }
            // 0.2 s of gliding, however it is sliced
            let exact = 300.0 * (1.0 - (-0.2f32 / SCROLL_TAU_SECS).exp());
            assert!((off - exact).abs() < 0.5, "{hz} Hz: {off} vs {exact}");
        }
    }

    #[test]
    fn smooth_scroll_converges_and_snaps_without_overshoot() {
        let mut off = (0.0, 0.0);
        let target = (0.0, 480.0);
        let mut moving = true;
        let mut frames = 0;
        let mut last = 0.0;
        while moving {
            let (next, still) = step_scroll(off, target, 1.0 / 120.0);
            assert!(
                next.1 >= last && next.1 <= target.1,
                "no overshoot, no reversal"
            );
            last = next.1;
            off = next;
            moving = still;
            frames += 1;
            assert!(frames < 2_000, "must stop requesting frames");
        }
        assert_eq!(off, target, "ends exactly on the target");
        let seconds = frames as f32 / 120.0;
        assert!(seconds < 0.6, "glides, then is done: {seconds}s");
    }

    #[test]
    fn an_idle_scroll_asks_for_no_frames() {
        let (off, moving) = step_scroll((12.0, 34.0), (12.0, 34.0), 0.016);
        assert_eq!(off, (12.0, 34.0));
        assert!(!moving);
    }

    #[test]
    fn long_frames_are_clamped() {
        assert_eq!(clamp_frame_dt(0.2), MAX_FRAME_SECS);
        assert_eq!(clamp_frame_dt(-1.0), 0.0);
        assert_eq!(clamp_frame_dt(f32::NAN), 0.0);
        assert_eq!(clamp_frame_dt(0.008), 0.008);
    }

    #[test]
    fn easing_has_the_right_shape() {
        assert_eq!(ease_out_cubic(0.0), 0.0);
        assert_eq!(ease_out_cubic(1.0), 1.0);
        assert_eq!(ease_out_cubic(2.0), 1.0);
        assert!(ease_out_cubic(0.5) > 0.5, "fast start");
    }

    #[test]
    fn velocity_tracker_calculates_exit_velocity() {
        let mut tracker = VelocityTracker::new();
        // 3 motion events: 10px in 16ms each -> ~625 px/s
        tracker.push(0.016, 0.0, 10.0);
        tracker.push(0.016, 0.0, 10.0);
        tracker.push(0.016, 0.0, 10.0);
        let (vx, vy) = tracker.velocity();
        assert_eq!(vx, 0.0);
        assert!((vy - 625.0).abs() < 1.0, "velocity {vy}");
    }

    #[test]
    fn velocity_tracker_recency_weighting_favors_exit_speed() {
        let mut tracker = VelocityTracker::new();
        // Accelerating flick: 4px in 16ms (250px/s), then 8px in 16ms (500px/s), then 16px in 16ms (1000px/s)
        tracker.push(0.016, 0.0, 4.0);
        tracker.push(0.016, 0.0, 8.0);
        tracker.push(0.016, 0.0, 16.0);
        let (_, vy) = tracker.velocity();
        // Unweighted average would be 28 / 0.048 = 583.3 px/s.
        // Recency-weighted estimate is significantly closer to the 1000 px/s exit speed (> 680 px/s).
        assert!(vy > 680.0, "weighted exit velocity {vy} should be > 680");
    }

    #[test]
    fn inertial_scroll_decays_exponentially_to_stop() {
        let mut inertia = InertialScroll::new((0.0, 500.0)).expect("above threshold");
        let mut total_displacement = 0.0;
        let mut frames = 0;
        let mut moving = true;

        while moving {
            let ((_, dy), still) = inertia.step(1.0 / 120.0);
            total_displacement += dy;
            moving = still;
            frames += 1;
            assert!(frames < 500, "inertia must terminate cleanly");
        }

        // Theoretical exact total displacement = v0 * tau = 500 * 0.32 = 160 px
        assert!(
            (total_displacement - 160.0).abs() < 5.0,
            "total distance {total_displacement}"
        );
        assert_eq!(inertia.velocity, (0.0, 0.0));
    }

    #[test]
    fn inertial_scroll_distance_is_identical_at_different_refresh_rates() {
        let v0 = (0.0, 800.0);
        let mut distances = Vec::new();

        for hz in [60.0, 120.0, 144.0, 240.0] {
            let mut inertia = InertialScroll::new(v0).unwrap();
            let mut distance = 0.0;
            let mut moving = true;
            while moving {
                let ((_, dy), still) = inertia.step(1.0 / hz);
                distance += dy;
                moving = still;
            }
            distances.push(distance);
        }

        // All refresh rates must agree to within 0.5 pixels
        let first = distances[0];
        for (i, d) in distances.iter().enumerate() {
            assert!(
                (d - first).abs() < 0.5,
                "refresh rate {i} gave {d} vs {first}"
            );
        }
    }
}
