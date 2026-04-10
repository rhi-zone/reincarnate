// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Browser persistence — localStorage wrapper. */

import type { SaveBackend } from "./save";

// HANDWRITTEN
export function loadLocal(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}

// HANDWRITTEN
export function saveLocal(key: string, value: string): void {
  try {
    localStorage.setItem(key, value);
  } catch {
    // Storage full or unavailable — silent fail
  }
}

// HANDWRITTEN
export function removeLocal(key: string): void {
  try {
    localStorage.removeItem(key);
  } catch {
    // Unavailable — silent fail
  }
}

/** Default SaveBackend backed by localStorage. */
// HANDWRITTEN
export function localStorageBackend(): SaveBackend {
  return { load: loadLocal, save: saveLocal, remove: removeLocal };
}
