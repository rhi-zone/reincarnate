// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Flash.Iterator — AVM2 for-in / for-each iteration. */

// HANDWRITTEN
export function hasNext(obj: unknown, index: number): boolean {
  const keys = Object.keys(obj as object);
  return index < keys.length;
}

/** AVM2 HasNext2 — advances index, returns whether there are more items. */
// HANDWRITTEN
export function hasNext2(obj: unknown, index: number): [unknown, number, boolean] {
  const keys = Object.keys(obj as object);
  if (index < keys.length) {
    return [obj, index + 1, true];
  }
  return [obj, 0, false];
}

// HANDWRITTEN
export function nextName(obj: unknown, index: number): string {
  const keys = Object.keys(obj as object);
  // AVM2 indices are 1-based during iteration.
  return keys[index - 1] ?? "";
}

// HANDWRITTEN
export function nextValue(obj: unknown, index: number): unknown {
  const keys = Object.keys(obj as object);
  return (obj as Record<string, unknown>)[keys[index - 1]!];
}
