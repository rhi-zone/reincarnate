/** Browser graphics — Canvas 2D initialization and context management. */

class GraphicsContext {
  canvas!: HTMLCanvasElement;
  ctx!: CanvasRenderingContext2D;
  tcanvas!: OffscreenCanvas | HTMLCanvasElement;
  tctx!: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
}

const gfx = new GraphicsContext();

export function initCanvas(id: string): { canvas: HTMLCanvasElement; ctx: CanvasRenderingContext2D } {
  gfx.canvas = document.getElementById(id) as HTMLCanvasElement;
  gfx.ctx = gfx.canvas.getContext("2d")!;
  gfx.ctx.imageSmoothingEnabled = false;
  gfx.tcanvas = "OffscreenCanvas" in window
    ? new OffscreenCanvas(0, 0)
    : document.createElement("canvas");
  gfx.tctx = gfx.tcanvas.getContext("2d")!;
  return { canvas: gfx.canvas, ctx: gfx.ctx };
}

export function getCanvas(): HTMLCanvasElement { return gfx.canvas; }
export function getCtx(): CanvasRenderingContext2D { return gfx.ctx; }
export function getTintCanvas(): OffscreenCanvas | HTMLCanvasElement { return gfx.tcanvas; }
export function getTintCtx(): CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D { return gfx.tctx; }

export function resizeCanvas(w: number, h: number): void {
  gfx.canvas.width = w;
  gfx.canvas.height = h;
}
