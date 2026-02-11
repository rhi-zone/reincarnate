/**
 * AS3 Vector compatibility patches for Array.
 *
 * Flash Vector.<T> maps to TypeScript Array<T>. These prototype patches
 * cover the Vector methods that have no native Array equivalent.
 */

declare global {
  interface Array<T> {
    removeAt(index: number): T;
    insertAt(index: number, element: T): void;
  }
}

if (!Array.prototype.removeAt) {
  Array.prototype.removeAt = function (index: number) {
    return this.splice(index, 1)[0];
  };
}

if (!Array.prototype.insertAt) {
  Array.prototype.insertAt = function (index: number, element: unknown) {
    this.splice(index, 0, element);
  };
}
