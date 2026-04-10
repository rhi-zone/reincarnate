// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/**
 * flash.desktop package — AIR desktop interfaces.
 */

import type { IDataInput } from "./utils";

/** AS3 `flash.desktop.IFilePromise` — deferred file data for drag-and-drop. */
// HANDWRITTEN
export abstract class IFilePromise {
  abstract get isAsync(): boolean;
  abstract get relativePath(): string;
  abstract close(): void;
  abstract open(): IDataInput;
  abstract reportError(e: unknown): void;
}
