// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Browser graphics — canvas creation and 2D context. */

import type { DocumentFactory } from "../../shared/render-root";

// HANDWRITTEN
export function initCanvas(id: string, doc: DocumentFactory = document): {
  canvas: HTMLCanvasElement;
  ctx: CanvasRenderingContext2D;
} {
  const canvas = (doc as Document).getElementById
    ? (doc as Document).getElementById(id) as HTMLCanvasElement
    : (doc as any).querySelector(`#${id}`) as HTMLCanvasElement;
  const ctx = canvas.getContext("2d")!;
  return { canvas, ctx };
}

// HANDWRITTEN
export function createCanvas(doc: DocumentFactory = document): HTMLCanvasElement {
  return doc.createElement("canvas") as HTMLCanvasElement;
}

// HANDWRITTEN
export function createMeasureContext(doc: DocumentFactory = document): CanvasRenderingContext2D {
  const c = createCanvas(doc);
  return c.getContext("2d")!;
}
