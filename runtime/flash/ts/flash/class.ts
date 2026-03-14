/** Flash.Class — AVM2 class hierarchy operations. */

export function getSuper(obj: unknown, name: string): unknown {
  const proto = Object.getPrototypeOf(Object.getPrototypeOf(obj));
  if (proto === null) return undefined;
  const desc = Object.getOwnPropertyDescriptor(proto, name);
  if (desc && desc.get) return desc.get.call(obj);
  return (proto as Record<string, unknown>)[name];
}

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

export function callSuper(obj: unknown, name: string, ...args: unknown[]): unknown {
  const proto = Object.getPrototypeOf(Object.getPrototypeOf(obj));
  if (proto === null || typeof (proto as Record<string, unknown>)[name] !== "function") return undefined;
  return ((proto as Record<string, (...a: unknown[]) => unknown>)[name]).apply(obj, args);
}

export function constructSuper(_obj: unknown, ..._args: unknown[]): void {
  // In AVM2, ConstructSuper calls the parent class's constructor.
  // With JS prototypes, this is handled by `super()` in constructors.
  // When we're emitting flat functions, this is a no-op — the super
  // constructor ran at allocation time.
}
