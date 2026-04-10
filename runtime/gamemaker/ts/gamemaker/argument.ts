// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/**
 * Out-of-range argument access stub.
 *
 * In GML, `argument[N]` on a script that received fewer than N+1 arguments
 * returns 0 (GMS1) or undefined (GMS2.3+, which GML treats as 0 numerically).
 * The `GameMaker.Argument.get` system call is emitted for such accesses when
 * the argument index exceeds the function's declared parameter count.
 */

// HANDWRITTEN
export function get(_name: string): number {
    return 0;
}
