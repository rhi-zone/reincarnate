// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Browser timing — setTimeout/setInterval wrapper. */

// HANDWRITTEN
export function scheduleTimeout(fn: () => void, ms: number): number {
  return window.setTimeout(fn, ms);
}

// HANDWRITTEN
export function cancelTimeout(id: number): void {
  window.clearTimeout(id);
}

// HANDWRITTEN
export function scheduleInterval(fn: () => void, ms: number): number {
  return window.setInterval(fn, ms);
}

// HANDWRITTEN
export function cancelInterval(id: number): void {
  window.clearInterval(id);
}
