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
  object_index = 0;
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
  /** GML built-in instance variable `image_blend` — sprite tint; default -1 (c_white). */
  image_blend = -1;

  // ---- GML built-in read-only instance properties (computed from sprite + transform) ----

  /** Number of sub-images in the assigned sprite (`sprite_get_number`). */
  get image_number(): number { return this._rt.sprite_get_number(this.sprite_index); }

  /** Sprite width scaled by `image_xscale` (`sprite_get_width * abs(image_xscale)`). */
  get sprite_width(): number {
    return this._rt.sprite_get_width(this.sprite_index) * Math.abs(this.image_xscale);
  }
  /** Sprite height scaled by `image_yscale` (`sprite_get_height * abs(image_yscale)`). */
  get sprite_height(): number {
    return this._rt.sprite_get_height(this.sprite_index) * Math.abs(this.image_yscale);
  }
  /** Sprite x-origin scaled by `image_xscale`. */
  get sprite_xoffset(): number {
    return this._rt.sprite_get_xoffset(this.sprite_index) * this.image_xscale;
  }
  /** Sprite y-origin scaled by `image_yscale`. */
  get sprite_yoffset(): number {
    return this._rt.sprite_get_yoffset(this.sprite_index) * this.image_yscale;
  }

  // Bounding box in room coordinates. World position of a local sprite coordinate is
  // `x + (local - origin) * image_xscale`; the AABB spans the scaled bbox edges.
  // Rotation (`image_angle`) is not applied, matching the runtime's axis-aligned
  // collision model (see `activateMouse`, `place_meeting`).
  private _bboxX(local: number): number {
    return this.x + (local - this._rt.sprite_get_xoffset(this.sprite_index)) * this.image_xscale;
  }
  private _bboxY(local: number): number {
    return this.y + (local - this._rt.sprite_get_yoffset(this.sprite_index)) * this.image_yscale;
  }
  /** Left edge of the instance bounding box, in room coordinates (read-only). */
  get bbox_left(): number {
    return Math.min(
      this._bboxX(this._rt.sprite_get_bbox_left(this.sprite_index)),
      this._bboxX(this._rt.sprite_get_bbox_right(this.sprite_index)),
    );
  }
  /** Right edge of the instance bounding box, in room coordinates (read-only). */
  get bbox_right(): number {
    return Math.max(
      this._bboxX(this._rt.sprite_get_bbox_left(this.sprite_index)),
      this._bboxX(this._rt.sprite_get_bbox_right(this.sprite_index)),
    );
  }
  /** Top edge of the instance bounding box, in room coordinates (read-only). */
  get bbox_top(): number {
    return Math.min(
      this._bboxY(this._rt.sprite_get_bbox_top(this.sprite_index)),
      this._bboxY(this._rt.sprite_get_bbox_bottom(this.sprite_index)),
    );
  }
  /** Bottom edge of the instance bounding box, in room coordinates (read-only). */
  get bbox_bottom(): number {
    return Math.max(
      this._bboxY(this._rt.sprite_get_bbox_top(this.sprite_index)),
      this._bboxY(this._rt.sprite_get_bbox_bottom(this.sprite_index)),
    );
  }

  // ---- GML built-in globals (shared via the runtime; readable from any instance) ----

  /** GML built-in global `room_width` — width of the current room. */
  get room_width(): number { return this._rt.room_width; }
  /** GML built-in global `room_height` — height of the current room. */
  get room_height(): number { return this._rt.room_height; }
  /** GML built-in global `current_time` — milliseconds since the game started. */
  get current_time(): number { return this._rt.current_time; }
  /** GML built-in global `current_year`. */
  get current_year(): number { return this._rt.current_year; }
  /** GML built-in global `current_month` (1-12). */
  get current_month(): number { return this._rt.current_month; }
  /** GML built-in global `current_day` (1-31). */
  get current_day(): number { return this._rt.current_day; }
  /** GML built-in global `current_weekday` (0-6). */
  get current_weekday(): number { return this._rt.current_weekday; }
  /** GML built-in global `current_hour` (0-23). */
  get current_hour(): number { return this._rt.current_hour; }
  /** GML built-in global `current_minute` (0-59). */
  get current_minute(): number { return this._rt.current_minute; }
  /** GML built-in global `current_second` (0-59). */
  get current_second(): number { return this._rt.current_second; }
  /** GML built-in global `view_camera` — per-viewport camera ID array (settable). */
  get view_camera(): number[] { return this._rt.view_camera; }
  set view_camera(v: number[]) { this._rt.view_camera = v; }
  /** GML built-in global `async_load` — DS Map handle during async events, else -1. */
  get async_load(): number { return this._rt.async_load; }
  set async_load(v: number) { this._rt.async_load = v; }

  get alarm(): number[] {
    if (this.#alarm === null) {
      this.#alarm = [];
    }
    return this.#alarm;
  }
  set alarm(val: number[]) {
    this.#alarm = val;
  }

  get id(): GMLObject { return this; }

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

const proto = GMLObject.prototype as unknown as Record<string, unknown>;
// Alarm stubs
for (let i = 0; i < 12; i++) {
  proto["alarm" + i] = noop;
}
// Key press / keyboard / key release stubs
for (let i = 0; i <= 0xff; i++) {
  proto["keypress" + i] = noop;
  proto["keyboard" + i] = noop;
  proto["keyrelease" + i] = noop;
}
// View event stubs
for (let i = 0; i < 8; i++) {
  proto["outsideview" + i] = noop;
  proto["boundaryview" + i] = noop;
}
// User event stubs
for (let i = 0; i < 16; i++) {
  proto["user" + i] = noop;
}
// Draw variant stubs
for (const ev of ["drawbegin", "drawend", "drawguibegin", "drawguiend", "drawpre", "drawpost", "drawresize"]) {
  proto[ev] = noop;
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
  proto[ev] = noop;
}

/** Sentinel prototype for detecting the root of the GMLObject hierarchy. */
// HANDWRITTEN
export { __baseproto };
