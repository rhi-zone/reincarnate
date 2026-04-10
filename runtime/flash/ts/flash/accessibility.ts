// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/**
 * flash.accessibility package — accessibility interfaces.
 */

/** AS3 `flash.accessibility.ISearchableText` — searchable text content. */
// HANDWRITTEN
export abstract class ISearchableText {
  abstract get searchText(): string;
}

/** AS3 `flash.accessibility.ISimpleTextSelection` — text selection state. */
// HANDWRITTEN
export abstract class ISimpleTextSelection {
  abstract get selectionActiveIndex(): number;
  abstract get selectionAnchorIndex(): number;
}
