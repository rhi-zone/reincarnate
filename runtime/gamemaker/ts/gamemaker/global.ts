// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** GML global variable object and helpers. */

import type { GameRuntime } from "./runtime";

// HANDWRITTEN
export function createGlobalAPI(rt: GameRuntime) {
  return {
    variable_global_exists(key: string): boolean {
      return key in rt.global;
    },
    variable_global_get(key: string): unknown {
      return rt.global[key];
    },
    variable_global_set(key: string, value: unknown): void {
      rt.global[key] = value;
    },
  };
}
