/**
 * Flash runtime — singleton Stage, Canvas2D rendering, ENTER_FRAME dispatch,
 * and DOM → Flash event bridging.
 */

import {
  Stage,
  DisplayObject,
  DisplayObjectContainer,
  Sprite,
  Graphics,
} from "./flash_display";
import {
  Event,
  MouseEvent as FlashMouseEvent,
  KeyboardEvent as FlashKeyboardEvent,
} from "./flash_events";
import { Rectangle } from "./flash_geom";

// ---------------------------------------------------------------------------
// Canvas + rendering context
// ---------------------------------------------------------------------------

const canvas = document.getElementById("reincarnate-canvas") as HTMLCanvasElement;
const ctx = canvas.getContext("2d")!;

// ---------------------------------------------------------------------------
// Singleton Stage
// ---------------------------------------------------------------------------

export const stage = new Stage();
stage.stageWidth = canvas.width;
stage.stageHeight = canvas.height;
// Stage's own .stage points to itself (Flash behaviour).
stage.stage = stage;

// ---------------------------------------------------------------------------
// flashTick — called once per frame from the game loop
// ---------------------------------------------------------------------------

const DEG_TO_RAD = Math.PI / 180;

export function flashTick(): void {
  // Sync stage dimensions to canvas.
  stage.stageWidth = canvas.width;
  stage.stageHeight = canvas.height;

  // Clear canvas then render.
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  renderDisplayList(stage, ctx);

  // Dispatch ENTER_FRAME to entire tree.
  dispatchEnterFrame(stage);
}

// ---------------------------------------------------------------------------
// Display list renderer
// ---------------------------------------------------------------------------

function renderDisplayList(
  node: DisplayObject,
  c: CanvasRenderingContext2D,
): void {
  if (!node.visible || node.alpha <= 0) return;

  c.save();

  // Apply transforms.
  c.translate(node.x, node.y);
  if (node.rotation !== 0) {
    c.rotate(node.rotation * DEG_TO_RAD);
  }
  if (node.scaleX !== 1 || node.scaleY !== 1) {
    c.scale(node.scaleX, node.scaleY);
  }
  c.globalAlpha *= node.alpha;

  // scrollRect clipping.
  if (node.scrollRect) {
    const r = node.scrollRect;
    c.beginPath();
    c.rect(r.x, r.y, r.width, r.height);
    c.clip();
    c.translate(-r.x, -r.y);
  }

  // Render graphics if present (Sprite, MovieClip).
  if ((node as any).graphics) {
    renderGraphics((node as any).graphics as Graphics, c);
  }

  // Recurse into children.
  if (node instanceof DisplayObjectContainer) {
    const n = node.numChildren;
    for (let i = 0; i < n; i++) {
      renderDisplayList(node.getChildAt(i), c);
    }
  }

  c.restore();
}

// ---------------------------------------------------------------------------
// Graphics command replay
// ---------------------------------------------------------------------------

function renderGraphics(gfx: Graphics, c: CanvasRenderingContext2D): void {
  const cmds = gfx._commands;
  if (cmds.length === 0) return;

  let fillActive = false;
  let strokeActive = false;

  for (const cmd of cmds) {
    switch (cmd.kind) {
      case "beginFill": {
        if (fillActive) c.fill();
        c.beginPath();
        c.fillStyle = colorToCSS(cmd.args[0], cmd.args[1]);
        fillActive = true;
        break;
      }
      case "endFill": {
        if (fillActive) {
          c.fill();
          fillActive = false;
        }
        if (strokeActive) {
          c.stroke();
        }
        break;
      }
      case "lineStyle": {
        const [thickness, color, alpha] = cmd.args;
        if (thickness == null || thickness < 0) {
          strokeActive = false;
        } else {
          c.lineWidth = thickness;
          c.strokeStyle = colorToCSS(color, alpha);
          strokeActive = true;
        }
        break;
      }
      case "moveTo": {
        c.moveTo(cmd.args[0], cmd.args[1]);
        break;
      }
      case "lineTo": {
        c.lineTo(cmd.args[0], cmd.args[1]);
        break;
      }
      case "curveTo": {
        c.quadraticCurveTo(cmd.args[0], cmd.args[1], cmd.args[2], cmd.args[3]);
        break;
      }
      case "drawRect": {
        c.rect(cmd.args[0], cmd.args[1], cmd.args[2], cmd.args[3]);
        break;
      }
      case "drawCircle": {
        c.moveTo(cmd.args[0] + cmd.args[2], cmd.args[1]);
        c.arc(cmd.args[0], cmd.args[1], cmd.args[2], 0, Math.PI * 2);
        break;
      }
      case "drawEllipse": {
        const [ex, ey, ew, eh] = cmd.args;
        c.ellipse(ex + ew / 2, ey + eh / 2, ew / 2, eh / 2, 0, 0, Math.PI * 2);
        break;
      }
      case "drawRoundRect": {
        const [rx, ry, rw, rh, rew, reh] = cmd.args;
        c.roundRect(rx, ry, rw, rh, [rew / 2, reh / 2]);
        break;
      }
      case "beginBitmapFill":
      case "beginGradientFill":
      case "lineGradientStyle":
      case "drawPath":
      case "drawTriangles":
        // TODO: implement advanced fill/stroke modes
        break;
    }
  }

  // Close any open path.
  if (fillActive) c.fill();
  if (strokeActive) c.stroke();
}

function colorToCSS(color: number, alpha: number): string {
  const r = (color >> 16) & 0xff;
  const g = (color >> 8) & 0xff;
  const b = color & 0xff;
  if (alpha >= 1) {
    return `rgb(${r},${g},${b})`;
  }
  return `rgba(${r},${g},${b},${alpha})`;
}

// ---------------------------------------------------------------------------
// ENTER_FRAME dispatch
// ---------------------------------------------------------------------------

function dispatchEnterFrame(node: DisplayObject): void {
  node.dispatchEvent(new Event(Event.ENTER_FRAME, false, false));
  if (node instanceof DisplayObjectContainer) {
    const n = node.numChildren;
    for (let i = 0; i < n; i++) {
      dispatchEnterFrame(node.getChildAt(i));
    }
  }
}

// ---------------------------------------------------------------------------
// DOM → Flash mouse event routing
// ---------------------------------------------------------------------------

function canvasCoords(e: MouseEvent): [number, number] {
  const rect = canvas.getBoundingClientRect();
  return [e.clientX - rect.left, e.clientY - rect.top];
}

function dispatchFlashMouse(type: string, e: MouseEvent): void {
  const [lx, ly] = canvasCoords(e);
  const evt = new FlashMouseEvent(
    type,
    true,
    false,
    lx,
    ly,
    null,
    e.ctrlKey,
    e.altKey,
    e.shiftKey,
    e.buttons > 0,
    0,
  );
  evt.stageX = lx;
  evt.stageY = ly;
  stage.dispatchEvent(evt);
}

canvas.addEventListener("click", (e) => dispatchFlashMouse(FlashMouseEvent.CLICK, e));
canvas.addEventListener("mousedown", (e) => dispatchFlashMouse(FlashMouseEvent.MOUSE_DOWN, e));
canvas.addEventListener("mouseup", (e) => dispatchFlashMouse(FlashMouseEvent.MOUSE_UP, e));
canvas.addEventListener("mousemove", (e) => dispatchFlashMouse(FlashMouseEvent.MOUSE_MOVE, e));
canvas.addEventListener("dblclick", (e) => dispatchFlashMouse(FlashMouseEvent.DOUBLE_CLICK, e));
canvas.addEventListener("mouseover", (e) => dispatchFlashMouse(FlashMouseEvent.MOUSE_OVER, e));
canvas.addEventListener("mouseout", (e) => dispatchFlashMouse(FlashMouseEvent.MOUSE_OUT, e));

canvas.addEventListener("wheel", (e) => {
  const [lx, ly] = canvasCoords(e);
  const evt = new FlashMouseEvent(
    FlashMouseEvent.MOUSE_WHEEL,
    true,
    false,
    lx,
    ly,
    null,
    e.ctrlKey,
    e.altKey,
    e.shiftKey,
    false,
    e.deltaY > 0 ? -1 : e.deltaY < 0 ? 1 : 0,
  );
  evt.stageX = lx;
  evt.stageY = ly;
  stage.dispatchEvent(evt);
});

// ---------------------------------------------------------------------------
// DOM → Flash keyboard event routing
// ---------------------------------------------------------------------------

function dispatchFlashKey(type: string, e: KeyboardEvent): void {
  const target = stage.focus ?? stage;
  const evt = new FlashKeyboardEvent(
    type,
    true,
    false,
    e.key.length === 1 ? e.key.charCodeAt(0) : 0,
    e.keyCode,
    0,
    e.ctrlKey,
    e.altKey,
    e.shiftKey,
  );
  target.dispatchEvent(evt);
}

document.addEventListener("keydown", (e) => dispatchFlashKey(FlashKeyboardEvent.KEY_DOWN, e));
document.addEventListener("keyup", (e) => dispatchFlashKey(FlashKeyboardEvent.KEY_UP, e));
