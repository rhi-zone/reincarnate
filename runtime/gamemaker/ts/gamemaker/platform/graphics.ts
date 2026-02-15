/** Browser graphics — Canvas 2D initialization and context management. */

let canvas: HTMLCanvasElement;
let ctx: CanvasRenderingContext2D;
let tcanvas: OffscreenCanvas | HTMLCanvasElement;
let tctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;

export function initCanvas(id: string): { canvas: HTMLCanvasElement; ctx: CanvasRenderingContext2D } {
  canvas = document.getElementById(id) as HTMLCanvasElement;
  ctx = canvas.getContext("2d")!;
  ctx.imageSmoothingEnabled = false;
  tcanvas = "OffscreenCanvas" in window
    ? new OffscreenCanvas(0, 0)
    : document.createElement("canvas");
  tctx = tcanvas.getContext("2d")!;
  return { canvas, ctx };
}

export function getCanvas(): HTMLCanvasElement { return canvas; }
export function getCtx(): CanvasRenderingContext2D { return ctx; }
export function getTintCanvas(): OffscreenCanvas | HTMLCanvasElement { return tcanvas; }
export function getTintCtx(): CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D { return tctx; }

export function resizeCanvas(w: number, h: number): void {
  canvas.width = w;
  canvas.height = h;
}
