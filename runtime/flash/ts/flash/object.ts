/** Flash.Object — AVM2 object model operations. */

export function typeOf(value: unknown): string {
  if (value === null || value === undefined) return "object";
  const t = typeof value;
  if (t === "function") return "function";
  if (t === "number") return "number";
  if (t === "boolean") return "boolean";
  if (t === "string") return "string";
  if (t === "undefined") return "undefined";
  if (t === "bigint") return "number";
  return "object";
}

export function deleteProperty(obj: unknown, name: string): boolean {
  try {
    return delete (obj as Record<string, unknown>)[name];
  } catch {
    return false;
  }
}

export function construct(ctor: Function, ...args: unknown[]): unknown {
  return new (ctor as new(...args: unknown[]) => unknown)(...args);
}

export function newObject(...pairs: unknown[]): Record<string, unknown> {
  const obj: Record<string, unknown> = {};
  for (let i = 0; i < pairs.length; i += 2) {
    obj[pairs[i] as string] = pairs[i + 1];
  }
  return obj;
}

export function applyType(base: unknown, ..._typeArgs: unknown[]): unknown {
  // AVM2 ApplyType creates a parameterized type (e.g., Vector.<int>).
  // In TypeScript land, just return the base — generics are erased.
  return base;
}

export function hasProperty(obj: unknown, name: string): boolean {
  if (obj === null || obj === undefined) return false;
  return name in Object(obj);
}
