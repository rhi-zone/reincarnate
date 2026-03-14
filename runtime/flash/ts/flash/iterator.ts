/** Flash.Iterator — AVM2 for-in / for-each iteration. */

export function hasNext(obj: unknown, index: number): boolean {
  const keys = Object.keys(obj as object);
  return index < keys.length;
}

/** AVM2 HasNext2 — advances index, returns whether there are more items. */
export function hasNext2(obj: unknown, index: number): [unknown, number, boolean] {
  const keys = Object.keys(obj as object);
  if (index < keys.length) {
    return [obj, index + 1, true];
  }
  return [obj, 0, false];
}

export function nextName(obj: unknown, index: number): string {
  const keys = Object.keys(obj as object);
  // AVM2 indices are 1-based during iteration.
  return keys[index - 1] ?? "";
}

export function nextValue(obj: unknown, index: number): unknown {
  const keys = Object.keys(obj as object);
  return (obj as Record<string, unknown>)[keys[index - 1]!];
}
