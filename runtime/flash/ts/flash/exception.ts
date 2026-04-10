// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Flash.Exception — AVM2 exception handling. */

function throwValue(value: any): never {
  throw value;
}

// HANDWRITTEN
export function newCatchScope(exceptionIndex: number): Record<string, any> {
  // In AVM2, a catch scope is an activation that holds the caught
  // exception. Return a plain object; the catch block will assign
  // the exception value to it.
  return {};
}

// `throw` is a reserved keyword — re-export with the reserved name
// so namespace imports work: `Flash_Exception.throw(...)`.
// HANDWRITTEN
export { throwValue as throw };
