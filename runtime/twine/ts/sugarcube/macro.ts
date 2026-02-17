/** SugarCube Macro system shim.
 *
 * Supports Macro.add(), Macro.has(), Macro.get(), Macro.delete() and provides
 * a MacroContext (the `this` inside a handler) with args, output, payload,
 * name, error(), addShadow(), createShadowWrapper(), createDebugView().
 *
 * Handlers are invoked via invokeMacro() which is called from widget.ts when
 * a widget lookup fails.
 */

import * as State from "./state";
import { output as outputState } from "./output";

export interface MacroDef {
  tags?: string[] | null;
  skipArgs?: boolean;
  handler: Function;
  [key: string]: any;
}

const macros: Map<string, MacroDef> = new Map();

/** Register one or more macros. */
export function add(nameOrNames: string | string[], definition: MacroDef): void {
  const names = Array.isArray(nameOrNames) ? nameOrNames : [nameOrNames];
  for (const name of names) {
    macros.set(name, definition);
  }
}

/** Check if a macro is registered. */
export function has(name: string): boolean {
  return macros.has(name);
}

/** Get a macro definition. */
export function get(name: string): MacroDef | null {
  return macros.get(name) || null;
}

/** Delete one or more macros. */
function deleteMacro(nameOrNames: string | string[]): void {
  const names = Array.isArray(nameOrNames) ? nameOrNames : [nameOrNames];
  for (const name of names) {
    macros.delete(name);
  }
}
export { deleteMacro as delete };

/** Build a MacroContext and invoke a macro handler.
 *
 * Called from widget.ts when a widget lookup fails and the name exists
 * in the Macro registry.
 */
export function invokeMacro(def: MacroDef, name: string, args: any[], output?: DocumentFragment): void {
  const argsArray: any[] = [...args];
  // SugarCube adds .full and .raw as properties on the args array
  (argsArray as any).full = args.join(" ");
  (argsArray as any).raw = args.join(" ");

  // Collect shadowed variable names for this invocation
  const shadows: string[] = [];

  const context = {
    name,
    args: argsArray,
    output: output || outputState.doc.createDocumentFragment(),
    payload: [] as { name: string; contents: string }[],
    error(msg: string): string {
      return `Error in macro <<${name}>>: ${msg}`;
    },
    addShadow(...varNames: string[]): void {
      for (const v of varNames) {
        // SugarCube accepts space/comma-separated names and bare names
        for (const part of v.split(/[,\s]+/)) {
          const trimmed = part.trim();
          if (trimmed) shadows.push(trimmed);
        }
      }
    },
    createShadowWrapper(fn: Function): Function {
      if (shadows.length === 0) return fn;
      // Capture current values of all shadowed variables
      const captured = new Map<string, any>();
      for (const varName of shadows) {
        captured.set(varName, State.get(varName));
      }
      return function(this: any, ...fnArgs: any[]) {
        // Save current values, restore captured values
        const saved = new Map<string, any>();
        for (const varName of shadows) {
          saved.set(varName, State.get(varName));
          State.set(varName, captured.get(varName));
        }
        try {
          return fn.apply(this, fnArgs);
        } finally {
          // Restore original values
          for (const varName of shadows) {
            State.set(varName, saved.get(varName));
          }
        }
      };
    },
    createDebugView(): void {},
    self: def,
    parser: { source: "", matchStart: 0 },
  };

  try {
    def.handler.call(context);
  } catch (e) {
    console.error(`[macro] error in <<${name}>>:`, e);
  }
}
