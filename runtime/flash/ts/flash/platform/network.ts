// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Browser network — HTTP resource loading. */

// HANDWRITTEN
export function fetchResource(
  url: string,
  options?: { method?: string; signal?: AbortSignal },
): Promise<Response> {
  return globalThis.fetch(url, options);
}

// HANDWRITTEN
export function hasFetch(): boolean {
  return typeof globalThis.fetch === "function";
}
