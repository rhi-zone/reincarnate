// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/**
 * flash.security package — security interfaces.
 */

import type { IDataInput } from "./utils";

/** AS3 `flash.security.IURIDereferencer` — resolves URIs in XML signatures. */
// HANDWRITTEN
export abstract class IURIDereferencer {
  abstract dereference(uri: string): IDataInput;
}
