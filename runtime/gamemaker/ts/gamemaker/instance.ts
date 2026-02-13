/** GML instance helpers — field access on specific object types. */

import { roomVariables, GMLObject } from "./runtime";

// Classes array reference (set by runtime.ts startGame)
let classes: (typeof GMLObject)[] = [];

export function setClasses(c: (typeof GMLObject)[]): void {
  classes = c;
}

/** Get a field value from the first instance of a given object type. */
export function getInstanceField(objId: number, field: string): any {
  const clazz = classes[objId];
  if (!clazz) return undefined;
  const inst = roomVariables.find((o) => o instanceof clazz);
  return inst ? (inst as any)[field] : undefined;
}

/** Set a field value on the first instance of a given object type. */
export function setInstanceField(objId: number, field: string, value: any): void {
  const clazz = classes[objId];
  if (!clazz) return;
  const inst = roomVariables.find((o) => o instanceof clazz);
  if (inst) (inst as any)[field] = value;
}

/** Get a field value from ALL instances. */
export function getAllField(field: string): any {
  for (const inst of roomVariables) {
    return (inst as any)[field];
  }
  return undefined;
}

/** Set a field value on ALL instances. */
export function setAllField(field: string, value: any): void {
  for (const inst of roomVariables) {
    (inst as any)[field] = value;
  }
}

/** Execute a block for each instance of a given type (or all). */
export function withInstances(target: number, callback: (inst: GMLObject) => void): void {
  if (target === -1) {
    // all
    for (const inst of roomVariables.slice()) {
      callback(inst);
    }
  } else if (target === -2) {
    // other — handled by caller
  } else if (target >= 0) {
    const clazz = classes[target];
    if (!clazz) return;
    for (const inst of roomVariables.slice()) {
      if (inst instanceof clazz) callback(inst);
    }
  }
}
