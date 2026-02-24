/// Timing system trait — handles frame pacing and time tracking.
///
/// Note: The TypeScript platform interface uses a callback model
/// (`schedule_timeout(cb, delay_ms) → handle` / `cancel_timeout(handle)`).
/// Rust game loops are typically structured around a winit event loop rather
/// than deferred callbacks, so this trait captures the frame-timing query
/// surface instead. Reconcile when a Rust runtime backend is built.
pub trait Timing {
    /// Seconds elapsed since last frame.
    fn delta_time(&self) -> f64;

    /// Total seconds elapsed since start.
    fn elapsed(&self) -> f64;

    /// Current frame number.
    fn frame_count(&self) -> u64;

    /// Target frames per second (0 = uncapped).
    fn target_fps(&self) -> u32;

    /// Set target frames per second.
    fn set_target_fps(&mut self, fps: u32);

    /// Called once per frame to advance timing state.
    fn tick(&mut self);
}
