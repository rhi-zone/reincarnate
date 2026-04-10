// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/**
 * GMLObject — base class for all GML game objects.
 *
 * Extracted from runtime.ts. All emitted game classes extend this.
 */

import type { GameRuntime } from "./runtime";
import { ACTIVE, noop } from "./constants";

const __baseproto = Object.getPrototypeOf(class {});

// HANDWRITTEN
export class GMLObject {
  static instances: GMLObject[];
  // GML objects are open — instance variables are set dynamically in event handlers.
  [key: string]: any;
  _rt!: GameRuntime;
  x = 0;
  y = 0;
  z = 0;
  xstart = 0;
  ystart = 0;
  xprevious = 0;
  yprevious = 0;
  image_xscale = 1;
  image_yscale = 1;
  sprite_index = -1;
  image_index = 0;
  image_alpha = 1;
  mask_index = -1;
  persistent: number | boolean = 0;
  depth = 0;
  #alarm: number[] | null = null;
  [ACTIVE] = false;
  visible: number | boolean = 1;
  speed = 0;
  direction = 0;
  hspeed = 0;
  vspeed = 0;
  friction = 0;
  gravity = 0;
  gravity_direction = 270;
  image_speed = 1;
  image_angle = 0;

  get alarm(): number[] {
    if (this.#alarm === null) {
      this.#alarm = [];
    }
    return this.#alarm;
  }
  set alarm(val: number[]) {
    this.#alarm = val;
  }

  // In GML, `id` is the numeric instance ID.  In the TypeScript runtime the
  // GMLObject itself serves as its own ID (field access and identity checks work
  // directly on the object).  Typing as `GMLObject & number` lets TypeScript
  // accept `this.id` in both object-field and numeric-argument contexts without
  // casting — which matches GML semantics where `id` is both.
  get id(): GMLObject & number { return this as unknown as GMLObject & number; }

  /** GML built-in global `room` — current room index from the runtime. */
  get room(): number { return this._rt.room; }
  set room(val: number) { this._rt.room_goto(val); }

  /** GML built-in global `fps_real` — measured frames per second from the runtime. */
  get fps_real(): number { return this._rt.fps_real; }

  create(): void {}
  destroy(): void {}

  draw(): void {
    if (this.sprite_index < 0 || !this.visible) return;
    this._rt.drawSprite(this.sprite_index, this.image_index, this.x, this.y, this);
  }

  mouseenter(): void {}
  mouseleave(): void {}
  roomstart(): void {}
  roomend(): void {}

  // Event stubs — overridden by subclasses
  beginstep(): void {}
  step(): void {}
  endstep(): void {}
  drawgui(): void {}
}

// Alarm stubs
for (let i = 0; i < 12; i++) {
  (GMLObject.prototype as any)["alarm" + i] = noop;
}
// Key press / keyboard / key release stubs
for (let i = 0; i <= 0xff; i++) {
  (GMLObject.prototype as any)["keypress" + i] = noop;
  (GMLObject.prototype as any)["keyboard" + i] = noop;
  (GMLObject.prototype as any)["keyrelease" + i] = noop;
}
// View event stubs
for (let i = 0; i < 8; i++) {
  (GMLObject.prototype as any)["outsideview" + i] = noop;
  (GMLObject.prototype as any)["boundaryview" + i] = noop;
}
// User event stubs
for (let i = 0; i < 16; i++) {
  (GMLObject.prototype as any)["user" + i] = noop;
}
// Draw variant stubs
for (const ev of ["drawbegin", "drawend", "drawguibegin", "drawguiend", "drawpre", "drawpost", "drawresize"]) {
  (GMLObject.prototype as any)[ev] = noop;
}
// Mouse button stubs
for (const ev of [
  "mouseleftbutton", "mouserightbutton", "mousemiddlebutton", "mousenobutton",
  "mouseleftpressed", "mouserightpressed", "mousemiddlepressed",
  "mouseleftreleased", "mouserightreleased", "mousemiddlereleased",
  "globalleftbutton", "globalrightbutton", "globalmiddlebutton",
  "globalleftpressed", "globalrightpressed", "globalmiddlepressed",
  "globalleftreleased", "globalrightreleased", "globalmiddlereleased",
]) {
  (GMLObject.prototype as any)[ev] = noop;
}

/** Sentinel prototype for detecting the root of the GMLObject hierarchy. */
// HANDWRITTEN
export { __baseproto };
