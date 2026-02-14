/** SugarCube DOM manipulation macro stubs.
 *
 * Block macros: <<replace>>...<<endreplace>>, <<append>>...<<endappend>>,
 * <<prepend>>...<<endprepend>>, <<copy>>...<<endcopy>>, <<remove>>.
 * Each produces a {method}_start() / {method}_end() pair.
 */

export function replace_start(...args: any[]): void {
  console.log("[dom:replace_start]", ...args);
}

export function replace_end(): void {
  console.log("[dom:replace_end]");
}

export function append_start(...args: any[]): void {
  console.log("[dom:append_start]", ...args);
}

export function append_end(): void {
  console.log("[dom:append_end]");
}

export function prepend_start(...args: any[]): void {
  console.log("[dom:prepend_start]", ...args);
}

export function prepend_end(): void {
  console.log("[dom:prepend_end]");
}

export function copy_start(...args: any[]): void {
  console.log("[dom:copy_start]", ...args);
}

export function copy_end(): void {
  console.log("[dom:copy_end]");
}

export function remove_start(...args: any[]): void {
  console.log("[dom:remove_start]", ...args);
}

export function remove_end(): void {
  console.log("[dom:remove_end]");
}

export function toggleclass_start(...args: any[]): void {
  console.log("[dom:toggleclass_start]", ...args);
}

export function toggleclass_end(): void {
  console.log("[dom:toggleclass_end]");
}

export function addclass_start(...args: any[]): void {
  console.log("[dom:addclass_start]", ...args);
}

export function addclass_end(): void {
  console.log("[dom:addclass_end]");
}

export function removeclass_start(...args: any[]): void {
  console.log("[dom:removeclass_start]", ...args);
}

export function removeclass_end(): void {
  console.log("[dom:removeclass_end]");
}
