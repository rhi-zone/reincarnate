// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/**
 * flash.text.ime package — input method editor interfaces.
 */

import type { Rectangle } from "../geom";

/** AS3 `flash.text.ime.IIMEClient` — IME composition target. */
// HANDWRITTEN
export abstract class IIMEClient {
  abstract get compositionStartIndex(): number;
  abstract get compositionEndIndex(): number;
  abstract get verticalTextLayout(): boolean;
  abstract get selectionAnchorIndex(): number;
  abstract get selectionActiveIndex(): number;
  abstract confirmComposition(text: string, preserveSelection: boolean): void;
  abstract getTextBounds(startIndex: number, endIndex: number): Rectangle;
  abstract getTextInRange(startIndex: number, endIndex: number): string;
  abstract selectRange(anchorIndex: number, activeIndex: number): void;
  abstract updateComposition(text: string, attributes: unknown[], compositionStartIndex: number, compositionEndIndex: number): void;
}
