// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Browser timing — interval scheduling. */

// HANDWRITTEN
export function scheduleInterval(
  callback: () => void,
  ms: number,
): number {
  return setInterval(callback, ms) as unknown as number;
}

// HANDWRITTEN
export function cancelScheduledInterval(id: number): void {
  clearInterval(id);
}
