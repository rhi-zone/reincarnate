// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Browser input — DOM event binding and coordinate conversion. */

// HANDWRITTEN
export function addCanvasEventListener(
  canvas: HTMLCanvasElement,
  type: string,
  handler: (e: any) => void,
): void {
  canvas.addEventListener(type, handler);
}

// HANDWRITTEN
export function addDocumentEventListener(
  type: string,
  handler: (e: any) => void,
): void {
  document.addEventListener(type, handler);
}

// HANDWRITTEN
export function getCanvasBounds(canvas: HTMLCanvasElement): DOMRect {
  return canvas.getBoundingClientRect();
}
