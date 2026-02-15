/** Twine runtime entry point.
 *
 * Twine is event-driven (click-to-navigate), not frame-based,
 * so timing is a no-op. The scaffold imports startStory from
 * navigation instead of running a frame loop.
 */
export const timing = { tick() {} };
