/** SugarCube story variable state management. */

const state: Record<string, any> = {};

/** Get a story variable ($name or _name). */
export function get(name: string): any {
  return state[name];
}

/** Set a story variable. */
export function set(name: string, value: any): void {
  state[name] = value;
}

/** Unset (delete) a story variable. */
export function unset(name: string): void {
  delete state[name];
}
