/** SugarCube engine operations.
 *
 * Contains runtime helpers for JS constructs that can't be expressed as
 * direct rewrites (ushr, instanceof, clone, eval, etc.) plus iterator
 * protocol and control flow helpers.
 *
 * Methods like `new`, `typeof`, `delete`, `in`, `pow`, `def`, `ndef`,
 * `is_nullish`, and `to_string` are rewritten to native JS constructs by
 * the backend rewrite pass — they never reach this module at runtime.
 */

/** Resolve a bare name (used for function lookups in expression context). */
export function resolve(name: string): any {
  // At runtime, this would look up `name` in the SugarCube story scope.
  return (globalThis as any)[name];
}

/** Deep clone a value (SugarCube's clone() function). */
export function clone(value: any): any {
  if (value === null || value === undefined) return value;
  if (typeof value !== "object") return value;
  return JSON.parse(JSON.stringify(value));
}

/** Create an iterator over a collection (for <<for _v range collection>>). */
export function iterate(collection: any): { entries: [any, any][]; index: number } {
  const entries: [any, any][] = [];
  if (Array.isArray(collection)) {
    for (let i = 0; i < collection.length; i++) {
      entries.push([i, collection[i]]);
    }
  } else if (collection && typeof collection === "object") {
    for (const key of Object.keys(collection)) {
      entries.push([key, (collection as any)[key]]);
    }
  }
  return { entries, index: 0 };
}

/** Check if an iterator has more elements. */
export function iterator_has_next(iter: { entries: [any, any][]; index: number }): boolean {
  return iter.index < iter.entries.length;
}

/** Get the next value from an iterator. */
export function iterator_next_value(iter: { entries: [any, any][]; index: number }): any {
  const entry = iter.entries[iter.index];
  iter.index++;
  return entry ? entry[1] : undefined;
}

/** Get the next key from an iterator. */
export function iterator_next_key(iter: { entries: [any, any][]; index: number }): any {
  // index was already advanced by iterator_next_value
  const entry = iter.entries[iter.index - 1];
  return entry ? entry[0] : undefined;
}

/** Unsigned right shift (>>>). */
export function ushr(a: any, b: any): number {
  return (a as number) >>> (b as number);
}

/** instanceof check. */
export function instanceof_(value: any, type_: any): boolean {
  return value instanceof type_;
}
export { instanceof_ as instanceof };

/** Create an arrow function from parameter names and a body expression. */
export function arrow(params: string, body: any): any {
  // params is a comma-separated string of parameter names
  // body is the evaluated body expression
  // At runtime this would need proper JS evaluation
  console.log("[arrow]", params, body);
  return body;
}

/** Evaluate raw JavaScript code (<<script>> blocks). */
// Using a wrapper to avoid shadowing the global eval.
export { evalCode as eval };
function evalCode(code: string): void {
  console.log("[eval]", code);
}

/** Throw an error. */
export function error(message: string): never {
  throw new Error(message);
}

/** Start a <<done>> block (deferred execution). */
export function done_start(): void {
  console.log("[done_start]");
}

/** End a <<done>> block. */
export function done_end(): void {
  console.log("[done_end]");
}

/** Break out of a loop (<<break>>). */
// Using a wrapper to avoid JS reserved word.
export { breakLoop as break };
function breakLoop(): void {
  console.log("[break]");
}

/** Continue to next iteration (<<continue>>). */
// Using a wrapper to avoid JS reserved word.
export { continueLoop as continue };
function continueLoop(): void {
  console.log("[continue]");
}
