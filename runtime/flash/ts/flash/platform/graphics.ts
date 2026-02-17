/** Browser graphics — canvas creation and 2D context. */

import type { DocumentFactory } from "../../../../shared/ts/render-root";

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

export function createCanvas(doc: DocumentFactory = document): HTMLCanvasElement {
  return doc.createElement("canvas") as HTMLCanvasElement;
}

export function createMeasureContext(doc: DocumentFactory = document): CanvasRenderingContext2D {
  const c = createCanvas(doc);
  return c.getContext("2d")!;
}
