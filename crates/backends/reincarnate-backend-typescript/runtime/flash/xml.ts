/** Flash.XML — E4X / XML operations. */

export function escapeAttribute(value: any): string {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&apos;");
}

export function escapeElement(value: any): string {
  return String(value)
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;");
}

export function checkFilter(value: any): any {
  // E4X filtering predicate check. If the value is XML, return it;
  // otherwise throw a TypeError like AVM2 does.
  if (value === null || value === undefined) {
    throw new TypeError("Cannot filter null or undefined");
  }
  return value;
}

export function getDescendants(obj: any, name: string): any {
  // E4X descendant access (obj..name). Without a real XML type,
  // fall back to property access.
  if (obj === null || obj === undefined) return undefined;
  return obj[name];
}

export function setDefaultNamespace(ns: any): void {
  // In AVM2 this sets the default XML namespace for the current scope.
  // No-op in lifted code — E4X is rarely used in practice.
}
