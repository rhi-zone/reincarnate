/** SugarCube widget invocation stubs.
 *
 * When the translator encounters an unknown macro name, it emits a
 * Widget.call(name, ...args) SystemCall. Content blocks within the
 * widget invocation are bracketed by content_start/content_end.
 */

/** Invoke a widget by name. */
export function call(name: string, ...args: any[]): void {
  console.log("[widget:call]", name, ...args);
}

/** Start a widget content block. */
export function content_start(): void {
  console.log("[widget:content_start]");
}

/** End a widget content block. */
export function content_end(): void {
  console.log("[widget:content_end]");
}
