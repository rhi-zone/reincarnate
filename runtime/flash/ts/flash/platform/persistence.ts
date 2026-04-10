// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Browser persistence — key-value storage via localStorage. */

// HANDWRITTEN
export function loadLocal(key: string): string | null {
  if (typeof localStorage === "undefined") return null;
  return localStorage.getItem(key);
}

// HANDWRITTEN
export function saveLocal(key: string, value: string): void {
  if (typeof localStorage === "undefined") return;
  localStorage.setItem(key, value);
}

// HANDWRITTEN
export function removeLocal(key: string): void {
  if (typeof localStorage === "undefined") return;
  localStorage.removeItem(key);
}
