// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Twine runtime entry point.
 *
 * Twine is event-driven (click-to-navigate), not frame-based,
 * so timing is a no-op. The scaffold imports startStory from
 * navigation instead of running a frame loop.
 */
// HANDWRITTEN
export const timing = { tick() {} };
