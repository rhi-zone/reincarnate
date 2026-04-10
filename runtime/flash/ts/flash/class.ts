// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Flash.Class — AVM2 class hierarchy operations. */

// HANDWRITTEN
export function getSuper(obj: unknown, name: string): unknown {
  const proto = Object.getPrototypeOf(Object.getPrototypeOf(obj));
  if (proto === null) return undefined;
  const desc = Object.getOwnPropertyDescriptor(proto, name);
  if (desc && desc.get) return desc.get.call(obj);
  return (proto as Record<string, unknown>)[name];
}

// HANDWRITTEN
export function setSuper(obj: unknown, name: string, value: unknown): void {
  const proto = Object.getPrototypeOf(Object.getPrototypeOf(obj));
  if (proto === null) return;
  const desc = Object.getOwnPropertyDescriptor(proto, name);
  if (desc && desc.set) {
    desc.set.call(obj, value);
  } else {
    (obj as Record<string, unknown>)[name] = value;
  }
}

// HANDWRITTEN
export function callSuper(obj: unknown, name: string, ...args: unknown[]): unknown {
  const proto = Object.getPrototypeOf(Object.getPrototypeOf(obj));
  if (proto === null || typeof (proto as Record<string, unknown>)[name] !== "function") return undefined;
  return ((proto as Record<string, (...a: unknown[]) => unknown>)[name]).apply(obj, args);
}

// HANDWRITTEN
export function constructSuper(_obj: unknown, ..._args: unknown[]): void {
  // In AVM2, ConstructSuper calls the parent class's constructor.
  // With JS prototypes, this is handled by `super()` in constructors.
  // When we're emitting flat functions, this is a no-op — the super
  // constructor ran at allocation time.
}
