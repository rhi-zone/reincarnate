/** GML instance helpers — field access on specific object types. */

import type { GameRuntime } from "./runtime";
import { GMLObject } from "./object";

export function createInstanceAPI(rt: GameRuntime) {
  /** Get a field value from the first instance of a given object type. */
  function getInstanceField(cls: typeof GMLObject | number, field: string): unknown {
    const clazz = typeof cls === 'function' ? cls : rt.classes[cls];
    if (!clazz) return undefined;
    const inst = rt.roomVariables.find((o) => o instanceof clazz);
    return inst ? (inst as unknown as Record<string, unknown>)[field] : undefined;
  }

  /** Set a field value on the first instance of a given object type. */
  function setInstanceField(cls: typeof GMLObject | number, field: string, value: unknown): void {
    const clazz = typeof cls === 'function' ? cls : rt.classes[cls];
    if (!clazz) return;
    const inst = rt.roomVariables.find((o) => o instanceof clazz);
    if (inst) (inst as unknown as Record<string, unknown>)[field] = value;
  }

  /** Set an indexed element of a field on the first instance of a given object type. */
  function setInstanceFieldIndex(cls: typeof GMLObject | number, field: string, index: number, value: unknown): void {
    const clazz = typeof cls === 'function' ? cls : rt.classes[cls];
    if (!clazz) return;
    const inst = rt.roomVariables.find((o) => o instanceof clazz);
    if (inst) (inst as unknown as Record<string, unknown[]>)[field]![index] = value;
  }

  /** Get a field value from ALL instances. */
  function getAllField(field: string): unknown {
    for (const inst of rt.roomVariables) {
      return (inst as unknown as Record<string, unknown>)[field];
    }
    return undefined;
  }

  /** Set a field value on ALL instances. */
  function setAllField(field: string, value: unknown): void {
    for (const inst of rt.roomVariables) {
      (inst as unknown as Record<string, unknown>)[field] = value;
    }
  }

  /** Execute a block for each instance of a given type (or all).
   * Sets rt._self to the current with-target so alarm_set/event_user work correctly.
   * Returns the last callback return value (supports GML `return X` inside `with`). */
  function withInstances<T extends GMLObject>(
    target: (new(...args: unknown[]) => T) | T | number,
    callback: (inst: T) => unknown,
  ): unknown {
    const prevSelf = rt._self;
    let result: unknown;
    if (typeof target === 'function') {
      // class constructor — iterate all instances of this class
      for (const inst of rt.roomVariables.slice()) {
        if (inst instanceof (target as Function)) {
          rt._self = inst; result = callback(inst as T);
        }
      }
    } else if (target === -1) {
      for (const inst of rt.roomVariables.slice()) {
        rt._self = inst; result = callback(inst as T);
      }
    } else if (target === -2) {
      // other — handled by caller
    } else if (target instanceof GMLObject) {
      // specific instance
      rt._self = target; result = callback(target as T);
    }
    rt._self = prevSelf;
    return result;
  }

  return {
    getInstanceField, setInstanceField, setInstanceFieldIndex,
    getAllField, setAllField, withInstances,
  };
}
