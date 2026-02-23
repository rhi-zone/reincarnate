/**
 * GML Runtime — game loop, GMLObject base class, room system.
 */

import { GraphicsContext, initCanvas, createCanvas, resizeCanvas, loadImage, scheduleFrame } from "./platform";
import type { RenderRoot } from "../../../shared/ts/render-root";
import { DrawState, createDrawAPI } from "./draw";
import { InputState, createInputAPI } from "./input";
import { StorageState, createStorageAPI } from "./storage";
import { MathState, createMathAPI } from "./math";
import { createGlobalAPI } from "./global";
import { createInstanceAPI } from "./instance";
import { ACTIVE, noop } from "./constants";
import type { Sprite } from "../../data/sprites";
import type { Texture } from "../../data/textures";
import type { Font } from "../../data/fonts";
import type { Room } from "../../data/rooms";

// Re-exports for class_preamble
export { Colors, HAligns, VAligns } from "./color";
export { ACTIVE } from "./constants";

// ---- GMLObject ----

const __baseproto = Object.getPrototypeOf(class {});

export class GMLObject {
  // GML objects are open — instance variables are set dynamically in event handlers.
  [key: string]: any;
  _rt!: GameRuntime;
  x = 0;
  y = 0;
  xstart = 0;
  ystart = 0;
  xprevious = 0;
  yprevious = 0;
  image_xscale = 1;
  image_yscale = 1;
  sprite_index: number | undefined = undefined;
  image_index = 0;
  image_alpha = 1;
  persistent = false;
  depth = 0;
  #alarm: number[] | null = null;
  [ACTIVE] = false;
  visible = true;

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

  create(): void {}
  destroy(): void {}

  draw(): void {
    if (this.sprite_index === undefined || !this.visible) return;
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

// ---- GMLRoom ----

class GMLRoom {
  constructor(private rt: GameRuntime) {}

  draw(): void {
    const rt = this.rt;
    const ctx = rt._gfx.ctx;
    ctx.fillStyle = "black";
    ctx.fillRect(0, 0, rt._gfx.canvas.width, rt._gfx.canvas.height);

    const oldRoom = rt.room;
    rt._isStepping = true;

    // Alarms
    for (const instance of rt.roomVariables) {
      if (instance.alarm.length !== 0) {
        for (let i = 0; i < 12; i++) {
          const alarmVal = instance.alarm[i];
          if (alarmVal) {
            instance.alarm[i] = alarmVal - 1;
            if (alarmVal - 1 === 0) {
              delete instance.alarm[i];
              const method = (instance as any)["alarm" + i];
              if (method !== noop) method.call(instance);
              if (oldRoom !== rt.room) break;
            }
          }
        }
      }
    }

    // Begin step
    let toStep: GMLObject[] = rt.roomVariables;
    while (toStep.length !== 0) {
      rt._pendingStep = [];
      for (const instance of toStep) {
        if ((instance as any).beginstep === noop) continue;
        instance.xprevious = instance.x;
        instance.yprevious = instance.y;
        instance.beginstep();
        if (oldRoom !== rt.room) break;
      }
      toStep = rt._pendingStep;
    }

    // Step
    toStep = rt.roomVariables;
    while (toStep.length !== 0) {
      rt._pendingStep = [];
      for (const instance of toStep) {
        if ((instance as any).step === noop) continue;
        instance.step();
        if (oldRoom !== rt.room) break;
      }
      toStep = rt._pendingStep;
    }

    // End step
    toStep = rt.roomVariables;
    while (toStep.length !== 0) {
      rt._pendingStep = [];
      for (const instance of toStep) {
        if ((instance as any).endstep === noop) continue;
        instance.endstep();
        if (oldRoom !== rt.room) break;
      }
      toStep = rt._pendingStep;
    }

    rt._isStepping = false;

    // Draw (sorted by depth, descending)
    const sorted = rt.roomVariables.slice().sort((a, b) => b.depth - a.depth);
    for (const instance of sorted) {
      if ((instance as any).draw === noop) continue;
      instance.draw();
      if (oldRoom !== rt.room) break;
    }

    // Draw GUI
    if (rt._drawguiUsed) {
      for (const instance of sorted) {
        if ((instance as any).drawgui === noop) continue;
        instance.drawgui();
        if (oldRoom !== rt.room) break;
      }
    }

    rt.resetFrameInput();
  }

  create(restart = false): void {
    const rt = this.rt;
    const idx = rt._roomInstances.indexOf(this);
    const data = rt._roomDatas[idx];
    if (!data) return;

    const instances: GMLObject[] = [];
    for (const obj of data.objs) {
      const clazz = rt.classes[obj.obj];
      if (!clazz) continue;
      const proto = clazz.prototype;
      if (!proto.persistent || rt._instanceNumber(clazz) === 0) {
        instances.push(rt._instanceCreate(obj.pos.x, obj.pos.y, clazz, true));
      }
    }
    for (const instance of instances) {
      instance.create();
    }
    // Room creation code runs after all instance creation events (GML semantics).
    const creationCode = rt.roomCreationCode[idx];
    if (creationCode) creationCode(rt);
  }

  destroy(restart = false): void {
    const rt = this.rt;
    for (const obj of rt.roomVariables.slice()) {
      if (restart || !obj.persistent) {
        rt._instanceDestroy(obj);
      }
    }
  }
}

// ---- GameRuntime ----

export class GameRuntime {
  // Sub-state containers
  _draw = new DrawState();
  _input = new InputState();
  _storage = new StorageState();
  _math = new MathState();
  _gfx = new GraphicsContext();
  _root?: RenderRoot;

  // Runtime state
  _drawHandle = 0;
  _currentRoom: GMLRoom | null = null;
  _isStepping = false;
  _pendingStep: GMLObject[] = [];
  _drawguiUsed = false;
  room = 0;
  room_speed = 60;
  fps_real = 1;
  roomVariables: GMLObject[] = [];
  classes: (typeof GMLObject)[] = [];
  _roomDatas: Room[] = [];
  roomCreationCode: (((_rt: GameRuntime) => void) | undefined)[] = [];
  sprites: Sprite[] = [];
  textures: Texture[] = [];
  textureSheets: HTMLImageElement[] = [];
  fonts: Font[] = [];
  _classesEnum: Record<string, number> = {};
  _roomInstances: GMLRoom[] = [];
  _instancesByClass = new Map<Function, GMLObject[]>();

  // Global variable object
  global: Record<string, any> = { score: 0, health: 0, lives: 0, async_load: -1 };

  // Sprites enum (per-runtime)
  Sprites: Record<string, number> = {};

  // ---- API functions populated by factories ----
  // Explicitly declare internally-used factory functions for type safety.
  getInstanceField!: (objId: number, field: string) => any;
  setInstanceField!: (objId: number, field: string, value: any) => void;
  setInstanceFieldIndex!: (objId: number, field: string, index: number, value: any) => void;
  getAllField!: (field: string) => any;
  setAllField!: (field: string, value: any) => void;
  withInstances!: (target: number, callback: (inst: GMLObject) => void) => void;
  drawSprite!: (
    spriteIndex: number, imageIndex: number, x: number, y: number,
    opts?: { image_alpha?: number; image_xscale?: number; image_yscale?: number },
  ) => void;
  resetFrameInput!: () => void;
  activateMouse!: (ax: number, ay: number, override?: boolean) => void;
  setupInput!: () => void;
  mouse_x!: () => number;
  mouse_y!: () => number;

  // Math API (from createMathAPI)
  random!: (max: number) => number;
  randomize!: () => void;
  random_range!: (min: number, max: number) => number;
  irandom!: (max: number) => number;
  irandom_range!: (min: number, max: number) => number;
  choose!: (...args: any[]) => any;

  // Draw API (from createDrawAPI)
  draw_set_color!: (color: number) => void;
  draw_set_font!: (font: number) => void;
  draw_set_halign!: (halign: number) => void;
  draw_set_valign!: (valign: number) => void;
  draw_set_alpha!: (alpha: number) => void;
  draw_get_alpha!: () => number;
  draw_sprite!: (spriteIndex: number, imageIndex: number, x: number, y: number) => void;
  draw_sprite_ext!: (spriteIndex: number, imageIndex: number, x: number, y: number, xscale: number, yscale: number, rot: number, color: number, alpha: number) => void;
  draw_self!: () => void;
  draw_rectangle!: (x1: number, y1: number, x2: number, y2: number, outline: boolean) => void;
  draw_text!: (x: number, y: number, text: string) => void;
  draw_text_color!: (x: number, y: number, text: string, c1: number, c2: number, c3: number, c4: number, alpha: number) => void;
  draw_text_transformed!: (x: number, y: number, text: string, xscale: number, yscale: number, angle: number) => void;
  draw_text_ext!: (x: number, y: number, text: string, sep: number, w: number) => void;
  draw_text_ext_color!: (x: number, y: number, text: string, sep: number, w: number, c1: number, c2: number, c3: number, c4: number, alpha: number) => void;
  draw_text_ext_transformed!: (x: number, y: number, text: string, sep: number, w: number, xscale: number, yscale: number, angle: number) => void;
  draw_text_transformed_color!: (x: number, y: number, text: string, xscale: number, yscale: number, angle: number, c1: number, c2: number, c3: number, c4: number, alpha: number) => void;
  draw_text_ext_transformed_color!: (x: number, y: number, text: string, sep: number, w: number, xscale: number, yscale: number, angle: number, c1: number, c2: number, c3: number, c4: number, alpha: number) => void;
  sprite_get_width!: (spriteIndex: number) => number;
  sprite_get_height!: (spriteIndex: number) => number;
  string_height_ext!: (text: string, sep: number, w: number) => number;

  // Input API (from createInputAPI)
  mouse_check_button!: (button: number) => boolean;
  mouse_check_button_pressed!: (button: number) => boolean;
  mouse_check_button_released!: (button: number) => boolean;

  // Storage API (from createStorageAPI)
  ini_open!: (path: string) => void;
  ini_close!: () => string;
  ini_write_real!: (section: string, key: string, value: number) => void;

  // Global API (from createGlobalAPI)
  variable_global_exists!: (key: string) => boolean;
  variable_global_get!: (key: string) => any;
  variable_global_set!: (key: string, value: any) => void;

  constructor() {
    Object.assign(this, createDrawAPI(this));
    Object.assign(this, createInputAPI(this));
    Object.assign(this, createStorageAPI(this));
    Object.assign(this, createMathAPI(this));
    Object.assign(this, createGlobalAPI(this));
    Object.assign(this, createInstanceAPI(this));

    // Bind core methods for destructuring
    this.instance_create = this.instance_create.bind(this);
    this.instance_destroy = this.instance_destroy.bind(this);
    this.instance_exists = this.instance_exists.bind(this);
    this.instance_number = this.instance_number.bind(this);
    this.room_goto = this.room_goto.bind(this);
    this.room_goto_next = this.room_goto_next.bind(this);
    this.room_goto_previous = this.room_goto_previous.bind(this);
    this.room_restart = this.room_restart.bind(this);
    this.game_restart = this.game_restart.bind(this);
  }

  // ---- Per-runtime instance tracking ----

  _getInstances(clazz: Function): GMLObject[] {
    let arr = this._instancesByClass.get(clazz);
    if (!arr) {
      arr = [];
      this._instancesByClass.set(clazz, arr);
    }
    return arr;
  }

  // ---- Instance management (internal) ----

  _instanceCreate(x: number, y: number, clazz: typeof GMLObject, roomStart = false): GMLObject {
    const instance = new (clazz as any)();
    instance._rt = this;
    // Walk prototype chain and push to per-runtime instance tracking
    let c: any = instance.constructor;
    while (c !== __baseproto) {
      this._getInstances(c).push(instance);
      c = Object.getPrototypeOf(c);
    }
    instance.xstart = instance.x = x;
    instance.ystart = instance.y = y;
    this.roomVariables.push(instance);
    if (!roomStart) {
      instance.create();
    }
    if (!this._drawguiUsed && (instance as any).drawgui !== noop) {
      this._drawguiUsed = true;
    }
    if (this._isStepping) {
      this._pendingStep.push(instance);
    }
    return instance;
  }

  _instanceDestroy(instance: GMLObject): void {
    instance.destroy();
    let c: any = instance.constructor;
    while (c !== __baseproto) {
      const arr = this._getInstances(c);
      const idx = arr.indexOf(instance);
      if (idx > -1) arr.splice(idx, 1);
      c = Object.getPrototypeOf(c);
    }
    const idx = this.roomVariables.indexOf(instance);
    if (idx > -1) this.roomVariables.splice(idx, 1);
  }

  _instanceNumber(clazz: typeof GMLObject): number {
    return this._getInstances(clazz).reduce(
      (p: number, c: GMLObject) => p + (c.constructor === clazz ? 1 : 0), 0,
    );
  }

  // ---- Public instance API (called from emitted code) ----

  /** GML `other` — the "other" instance in collision/with events. Set by withInstances. */
  other: any = null;

  instance_create(x: number, y: number, classIndex: number): GMLObject {
    const clazz = this.classes[classIndex]!;
    return this._instanceCreate(x, y, clazz);
  }

  instance_create_depth(x: number, y: number, depth: number, classIndex: number): GMLObject {
    const clazz = this.classes[classIndex]!;
    const inst = this._instanceCreate(x, y, clazz);
    inst.depth = depth;
    return inst;
  }

  instance_create_layer(x: number, y: number, _layer: any, classIndex: number): GMLObject {
    const clazz = this.classes[classIndex]!;
    return this._instanceCreate(x, y, clazz);
  }

  instance_nearest(x: number, y: number, classIndex: number): GMLObject | null {
    const clazz = this.classes[classIndex];
    if (!clazz) return null;
    const instances = this._getInstances(clazz);
    if (instances.length === 0) return null;
    let nearest = instances[0]!;
    let minDist = Math.hypot(nearest.x - x, nearest.y - y);
    for (let i = 1; i < instances.length; i++) {
      const inst = instances[i]!;
      const d = Math.hypot(inst.x - x, inst.y - y);
      if (d < minDist) { minDist = d; nearest = inst; }
    }
    return nearest;
  }

  object_is_ancestor(classIndex: number, parentIndex: number): boolean {
    const clazz = this.classes[classIndex];
    const parent = this.classes[parentIndex];
    if (!clazz || !parent) return false;
    let proto = Object.getPrototypeOf(clazz);
    while (proto && proto !== __baseproto) {
      if (proto === parent) return true;
      proto = Object.getPrototypeOf(proto);
    }
    return false;
  }

  layer_get_id(_name: string): number { return -1; }

  // ---- Instance creation extensions ----

  instance_create_depth(x: number, y: number, depth: number, classIndex: number): GMLObject {
    const clazz = this.classes[classIndex]!;
    const inst = this._instanceCreate(x, y, clazz);
    inst.depth = depth;
    return inst;
  }

  instance_create_layer(x: number, y: number, _layer: any, classIndex: number): GMLObject {
    const clazz = this.classes[classIndex]!;
    return this._instanceCreate(x, y, clazz);
  }

  instance_nearest(x: number, y: number, classIndex: number): GMLObject | null {
    const clazz = this.classes[classIndex];
    if (!clazz) return null;
    const instances = this._getInstances(clazz);
    if (instances.length === 0) return null;
    let nearest = instances[0]!;
    let minDist = Math.hypot(nearest.x - x, nearest.y - y);
    for (let i = 1; i < instances.length; i++) {
      const inst = instances[i]!;
      const d = Math.hypot(inst.x - x, inst.y - y);
      if (d < minDist) { minDist = d; nearest = inst; }
    }
    return nearest;
  }

  object_is_ancestor(classIndex: number, parentIndex: number): boolean {
    const clazz = this.classes[classIndex];
    const parent = this.classes[parentIndex];
    if (!clazz || !parent) return false;
    let proto = Object.getPrototypeOf(clazz);
    while (proto && proto !== __baseproto) {
      if (proto === parent) return true;
      proto = Object.getPrototypeOf(proto);
    }
    return false;
  }

  layer_get_id(_name: string): number { return -1; }

  // ---- Sprite API extensions ----

  sprite_get_xoffset(spr: number): number { return this.sprites[spr]?.frames[0]?.originX ?? 0; }
  sprite_get_yoffset(spr: number): number { return this.sprites[spr]?.frames[0]?.originY ?? 0; }
  sprite_get_number(spr: number): number { return this.sprites[spr]?.frames.length ?? 1; }
  sprite_get_speed(spr: number): number { return this.sprites[spr]?.animSpeed ?? 1; }
  sprite_get_bbox_left(spr: number): number { return this.sprites[spr]?.bbox?.left ?? 0; }
  sprite_get_bbox_right(spr: number): number { return this.sprites[spr]?.bbox?.right ?? 0; }
  sprite_get_bbox_top(spr: number): number { return this.sprites[spr]?.bbox?.top ?? 0; }
  sprite_get_bbox_bottom(spr: number): number { return this.sprites[spr]?.bbox?.bottom ?? 0; }
  sprite_exists(spr: number): boolean { return spr >= 0 && spr < this.sprites.length; }

  // ---- Alarm API ----

  alarm_set(inst: any, alarm: number, steps: number): void {
    if (inst && typeof inst === 'object') inst.alarm[alarm] = steps;
  }
  alarm_get(inst: any, alarm: number): number {
    return inst?.alarm?.[alarm] ?? -1;
  }

  // ---- Misc pure utility ----

  angle_difference(a: number, b: number): number {
    // +540 normalizes JS's sign-preserving modulo for negative values.
    return ((((a - b) % 360) + 540) % 360) - 180;
  }

  approach(value: number, target: number, amount: number): number {
    if (value < target) return Math.min(value + amount, target);
    return Math.max(value - amount, target);
  }

  asset_get_index(_name: string): number { return -1; }
  asset_get_tags(_asset: number): string[] { return []; }
  asset_has_tags(_asset: number, _tags: string | string[]): boolean { return false; }

  // ---- Array API (GMS2 style) ----

  array_create(size: number, defaultVal: any = 0): any[] { return new Array(size).fill(defaultVal); }
  array_copy(dest: any[], destIndex: number, src: any[], srcIndex: number, count: number): void {
    for (let i = 0; i < count; i++) dest[destIndex + i] = src[srcIndex + i];
  }
  array_equals(a: any[], b: any[]): boolean {
    if (a.length !== b.length) return false;
    for (let i = 0; i < a.length; i++) if (a[i] !== b[i]) return false;
    return true;
  }
  array_concat(...arrs: any[]): any[] { return ([] as any[]).concat(...arrs); }
  array_delete(arr: any[], index: number, count: number): void { arr.splice(index, count); }
  array_insert(arr: any[], index: number, ...vals: any[]): void { arr.splice(index, 0, ...vals); }
  array_get(arr: any[], index: number): any { return arr[index]; }
  array_set(arr: any[], index: number, val: any): void { arr[index] = val; }
  array_sort(arr: any[], ascending: boolean): void { arr.sort((a, b) => ascending ? a - b : b - a); }
  array_shuffle(arr: any[]): void {
    for (let i = arr.length - 1; i > 0; i--) {
      const j = Math.floor(Math.random() * (i + 1));
      [arr[i], arr[j]] = [arr[j], arr[i]];
    }
  }
  array_shuffle_ext(arr: any[]): void { this.array_shuffle(arr); }

  // ---- Type-check functions ----

  is_array(val: any): boolean { return Array.isArray(val); }
  is_string(val: any): boolean { return typeof val === 'string'; }
  is_real(val: any): boolean { return typeof val === 'number'; }
  is_undefined(val: any): boolean { return val === undefined; }
  is_nan(val: any): boolean { return typeof val === 'number' && isNaN(val); }
  is_infinity(val: any): boolean { return val === Infinity || val === -Infinity; }
  is_numeric(val: any): boolean { return !isNaN(Number(val)); }

  // ---- Surface API (unimplemented — requires WebGL offscreen rendering) ----

  surface_exists(_surf: number): boolean { throw new Error("surface_exists: surfaces require WebGL implementation"); }
  surface_create(_w: number, _h: number): number { throw new Error("surface_create: surfaces require WebGL implementation"); }
  surface_free(_surf: number): void { throw new Error("surface_free: surfaces require WebGL implementation"); }
  surface_set_target(_surf: number): void { throw new Error("surface_set_target: surfaces require WebGL implementation"); }
  surface_reset_target(): void { throw new Error("surface_reset_target: surfaces require WebGL implementation"); }
  draw_surface(_surf: number, _x: number, _y: number): void { throw new Error("draw_surface: surfaces require WebGL implementation"); }
  draw_surface_ext(_surf: number, _x: number, _y: number, _xs: number, _ys: number, _rot: number, _col: number, _alpha: number): void { throw new Error("draw_surface_ext: surfaces require WebGL implementation"); }
  draw_surface_part(_surf: number, _left: number, _top: number, _w: number, _h: number, _x: number, _y: number): void { throw new Error("draw_surface_part: surfaces require WebGL implementation"); }
  surface_get_width(_surf: number): number { throw new Error("surface_get_width: surfaces require WebGL implementation"); }
  surface_get_height(_surf: number): number { throw new Error("surface_get_height: surfaces require WebGL implementation"); }
  surface_getpixel(_surf: number, _x: number, _y: number): number { throw new Error("surface_getpixel: surfaces require WebGL implementation"); }

  // ---- Shader API (unimplemented — requires WebGL shaders) ----

  shader_is_compiled(_sh: number): boolean { throw new Error("shader_is_compiled: shaders require WebGL implementation"); }
  shader_set(_sh: number): void { throw new Error("shader_set: shaders require WebGL implementation"); }
  shader_reset(): void { throw new Error("shader_reset: shaders require WebGL implementation"); }
  shader_get_uniform(_sh: number, _name: string): number { throw new Error("shader_get_uniform: shaders require WebGL implementation"); }
  shader_set_uniform_f(_handle: number, ..._vals: number[]): void { throw new Error("shader_set_uniform_f: shaders require WebGL implementation"); }
  shader_set_uniform_i(_handle: number, ..._vals: number[]): void { throw new Error("shader_set_uniform_i: shaders require WebGL implementation"); }
  shader_get_sampler_index(_sh: number, _name: string): number { throw new Error("shader_get_sampler_index: shaders require WebGL implementation"); }

  // ---- GPU state API (unimplemented — requires WebGL) ----

  gpu_set_colorwriteenable(_r: boolean, _g: boolean, _b: boolean, _a: boolean): void { throw new Error("gpu_set_colorwriteenable: requires WebGL implementation"); }
  gpu_get_colorwriteenable(): [boolean, boolean, boolean, boolean] { throw new Error("gpu_get_colorwriteenable: requires WebGL implementation"); }
  gpu_set_fog(_enabled: boolean, _color: number, _start: number, _end: number): void { throw new Error("gpu_set_fog: requires WebGL implementation"); }
  gpu_set_blendmode(_mode: number): void { throw new Error("gpu_set_blendmode: requires WebGL implementation"); }
  gpu_set_blendmode_ext(_src: number, _dst: number): void { throw new Error("gpu_set_blendmode_ext: requires WebGL implementation"); }
  gpu_set_alphatestenable(_enabled: boolean): void { throw new Error("gpu_set_alphatestenable: requires WebGL implementation"); }
  gpu_set_alphatestref(_ref: number): void { throw new Error("gpu_set_alphatestref: requires WebGL implementation"); }
  gpu_set_ztestenable(_enabled: boolean): void { throw new Error("gpu_set_ztestenable: requires WebGL implementation"); }
  gpu_set_zwriteenable(_enabled: boolean): void { throw new Error("gpu_set_zwriteenable: requires WebGL implementation"); }
  gpu_set_cullmode(_mode: number): void { throw new Error("gpu_set_cullmode: requires WebGL implementation"); }

  // ---- Audio API (unimplemented — requires platform audio layer) ----
  //
  // Audio belongs in the platform layer (platform/audio.ts), not here.
  // These methods need to be wired to a platform audio implementation
  // that abstracts HTMLAudioElement or Web Audio API.
  // See: docs/architecture.md "Runtime Architecture" → Platform Interface.

  audio_play_sound(_sound: number, _priority: number, _loop: boolean, _gain?: number, _offset?: number, _pitch?: number): number { throw new Error("audio_play_sound: implement in platform/audio.ts"); }
  audio_play_sound_at(_sound: number, _x: number, _y: number, _z: number, _falloff: number, _min: number, _max: number, _priority: number, _loop: boolean): number { throw new Error("audio_play_sound_at: implement in platform/audio.ts"); }
  audio_is_playing(_handle: number): boolean { throw new Error("audio_is_playing: implement in platform/audio.ts"); }
  audio_stop_sound(_handle: number): void { throw new Error("audio_stop_sound: implement in platform/audio.ts"); }
  audio_stop_all(): void { throw new Error("audio_stop_all: implement in platform/audio.ts"); }
  audio_pause_sound(_handle: number): void { throw new Error("audio_pause_sound: implement in platform/audio.ts"); }
  audio_resume_sound(_handle: number): void { throw new Error("audio_resume_sound: implement in platform/audio.ts"); }
  audio_resume_all(): void { throw new Error("audio_resume_all: implement in platform/audio.ts"); }
  audio_exists(_sound: number): boolean { throw new Error("audio_exists: implement in platform/audio.ts"); }
  audio_get_name(_sound: number): string { throw new Error("audio_get_name: implement in platform/audio.ts"); }
  audio_sound_gain(_handle: number, _gain: number, _time: number): void { throw new Error("audio_sound_gain: implement in platform/audio.ts"); }
  audio_sound_get_gain(_handle: number): number { throw new Error("audio_sound_get_gain: implement in platform/audio.ts"); }
  audio_sound_pitch(_handle: number, _pitch: number): void { throw new Error("audio_sound_pitch: implement in platform/audio.ts"); }
  audio_master_gain(_gain: number): void { throw new Error("audio_master_gain: implement in platform/audio.ts"); }
  audio_group_load(_group: number): void { throw new Error("audio_group_load: implement in platform/audio.ts"); }
  audio_group_stop_all(_group: number): void { throw new Error("audio_group_stop_all: implement in platform/audio.ts"); }
  audio_group_set_gain(_group: number, _gain: number, _time: number): void { throw new Error("audio_group_set_gain: implement in platform/audio.ts"); }

  // ---- Particle API (unimplemented — requires particle simulation) ----

  part_system_create(): number { throw new Error("part_system_create: particle system not yet implemented"); }
  part_system_destroy(_sys: number): void { throw new Error("part_system_destroy: particle system not yet implemented"); }
  part_type_create(): number { throw new Error("part_type_create: particle system not yet implemented"); }
  part_type_destroy(_type: number): void { throw new Error("part_type_destroy: particle system not yet implemented"); }
  part_type_life(_type: number, _min: number, _max: number): void { throw new Error("part_type_life: particle system not yet implemented"); }
  part_type_direction(_type: number, _dir1: number, _dir2: number, _inc: number, _wiggle: number): void { throw new Error("part_type_direction: particle system not yet implemented"); }
  part_type_speed(_type: number, _min: number, _max: number, _inc: number, _wiggle: number): void { throw new Error("part_type_speed: particle system not yet implemented"); }
  part_type_sprite(_type: number, _spr: number, _anim: boolean, _stretch: boolean, _random: boolean): void { throw new Error("part_type_sprite: particle system not yet implemented"); }
  part_type_color1(_type: number, _col: number): void { throw new Error("part_type_color1: particle system not yet implemented"); }
  part_type_color2(_type: number, _col1: number, _col2: number): void { throw new Error("part_type_color2: particle system not yet implemented"); }
  part_type_color3(_type: number, _col1: number, _col2: number, _col3: number): void { throw new Error("part_type_color3: particle system not yet implemented"); }
  part_type_alpha1(_type: number, _alpha: number): void { throw new Error("part_type_alpha1: particle system not yet implemented"); }
  part_type_alpha2(_type: number, _a1: number, _a2: number): void { throw new Error("part_type_alpha2: particle system not yet implemented"); }
  part_type_alpha3(_type: number, _a1: number, _a2: number, _a3: number): void { throw new Error("part_type_alpha3: particle system not yet implemented"); }
  part_type_scale(_type: number, _xs: number, _ys: number): void { throw new Error("part_type_scale: particle system not yet implemented"); }
  part_type_size(_type: number, _minSize: number, _maxSize: number, _inc: number, _wiggle: number): void { throw new Error("part_type_size: particle system not yet implemented"); }
  part_type_orientation(_type: number, _min: number, _max: number, _inc: number, _wiggle: number, _relative: boolean): void { throw new Error("part_type_orientation: particle system not yet implemented"); }
  part_particles_create(_sys: number, _x: number, _y: number, _type: number, _count: number): void { throw new Error("part_particles_create: particle system not yet implemented"); }
  part_particles_create_color(_sys: number, _x: number, _y: number, _type: number, _color: number, _count: number): void { throw new Error("part_particles_create_color: particle system not yet implemented"); }
  part_emitter_create(_sys: number): number { throw new Error("part_emitter_create: particle system not yet implemented"); }
  part_emitter_destroy(_sys: number, _emit: number): void { throw new Error("part_emitter_destroy: particle system not yet implemented"); }
  part_emitter_region(_sys: number, _emit: number, _x1: number, _y1: number, _x2: number, _y2: number, _shape: number, _dist: number): void { throw new Error("part_emitter_region: particle system not yet implemented"); }
  part_emitter_burst(_sys: number, _emit: number, _type: number, _count: number): void { throw new Error("part_emitter_burst: particle system not yet implemented"); }

  // ---- DS (Data Structure) API — backed by JS Map / Array ----

  private _dsLists = new Map<number, any[]>();
  private _dsMaps = new Map<number, Map<any, any>>();
  private _dsGrids = new Map<number, { w: number; h: number; data: any[] }>();
  private _dsStacks = new Map<number, any[]>();
  private _dsQueues = new Map<number, any[]>();
  private _dsNextId = 1;

  ds_list_create(): number { const id = this._dsNextId++; this._dsLists.set(id, []); return id; }
  ds_list_destroy(list: number): void { this._dsLists.delete(list); }
  ds_list_add(list: number, ...vals: any[]): void { this._dsLists.get(list)?.push(...vals); }
  ds_list_size(list: number): number { return this._dsLists.get(list)?.length ?? 0; }
  ds_list_find_value(list: number, pos: number): any { return this._dsLists.get(list)?.[pos]; }
  ds_list_set(list: number, pos: number, val: any): void { const l = this._dsLists.get(list); if (l) l[pos] = val; }
  ds_list_delete(list: number, pos: number): void { this._dsLists.get(list)?.splice(pos, 1); }
  ds_list_clear(list: number): void { const l = this._dsLists.get(list); if (l) l.length = 0; }
  ds_list_exists(list: number): boolean { return this._dsLists.has(list); }
  ds_list_find_index(list: number, val: any): number { return this._dsLists.get(list)?.indexOf(val) ?? -1; }
  ds_list_sort(list: number, ascending: boolean): void { this._dsLists.get(list)?.sort((a, b) => ascending ? a - b : b - a); }

  ds_map_create(): number { const id = this._dsNextId++; this._dsMaps.set(id, new Map()); return id; }
  ds_map_destroy(map: number): void { this._dsMaps.delete(map); }
  ds_map_add(map: number, key: any, val: any): void { this._dsMaps.get(map)?.set(key, val); }
  ds_map_set(map: number, key: any, val: any): void { this._dsMaps.get(map)?.set(key, val); }
  ds_map_find_value(map: number, key: any): any { return this._dsMaps.get(map)?.get(key); }
  ds_map_exists(map: number, key: any): boolean { return this._dsMaps.get(map)?.has(key) ?? false; }
  ds_map_delete(map: number, key: any): void { this._dsMaps.get(map)?.delete(key); }
  ds_map_size(map: number): number { return this._dsMaps.get(map)?.size ?? 0; }
  ds_map_clear(map: number): void { this._dsMaps.get(map)?.clear(); }
  ds_map_is_map(map: number): boolean { return this._dsMaps.has(map); }
  ds_map_find_first(map: number): any { return this._dsMaps.get(map)?.keys().next().value; }
  ds_map_find_next(map: number, key: any): any {
    const m = this._dsMaps.get(map);
    if (!m) return undefined;
    let found = false;
    for (const k of m.keys()) { if (found) return k; if (k === key) found = true; }
    return undefined;
  }

  ds_grid_create(w: number, h: number): number {
    const id = this._dsNextId++;
    this._dsGrids.set(id, { w, h, data: new Array(w * h).fill(0) });
    return id;
  }
  ds_grid_destroy(grid: number): void { this._dsGrids.delete(grid); }
  ds_grid_get(grid: number, x: number, y: number): any {
    const g = this._dsGrids.get(grid);
    return g ? g.data[y * g.w + x] : undefined;
  }
  ds_grid_set(grid: number, x: number, y: number, val: any): void {
    const g = this._dsGrids.get(grid);
    if (g) g.data[y * g.w + x] = val;
  }
  ds_grid_width(grid: number): number { return this._dsGrids.get(grid)?.w ?? 0; }
  ds_grid_height(grid: number): number { return this._dsGrids.get(grid)?.h ?? 0; }
  ds_grid_clear(grid: number, val: any): void { const g = this._dsGrids.get(grid); if (g) g.data.fill(val); }
  ds_grid_add(grid: number, x: number, y: number, val: any): void {
    const g = this._dsGrids.get(grid);
    if (g) g.data[y * g.w + x] = (g.data[y * g.w + x] ?? 0) + val;
  }

  ds_stack_create(): number { const id = this._dsNextId++; this._dsStacks.set(id, []); return id; }
  ds_stack_destroy(stack: number): void { this._dsStacks.delete(stack); }
  ds_stack_push(stack: number, val: any): void { this._dsStacks.get(stack)?.push(val); }
  ds_stack_pop(stack: number): any { return this._dsStacks.get(stack)?.pop(); }
  ds_stack_top(stack: number): any { const s = this._dsStacks.get(stack); return s?.[s.length - 1]; }
  ds_stack_size(stack: number): number { return this._dsStacks.get(stack)?.length ?? 0; }
  ds_stack_empty(stack: number): boolean { return (this._dsStacks.get(stack)?.length ?? 0) === 0; }

  ds_queue_create(): number { const id = this._dsNextId++; this._dsQueues.set(id, []); return id; }
  ds_queue_destroy(queue: number): void { this._dsQueues.delete(queue); }
  ds_queue_enqueue(queue: number, val: any): void { this._dsQueues.get(queue)?.push(val); }
  ds_queue_dequeue(queue: number): any { return this._dsQueues.get(queue)?.shift(); }
  ds_queue_head(queue: number): any { return this._dsQueues.get(queue)?.[0]; }
  ds_queue_size(queue: number): number { return this._dsQueues.get(queue)?.length ?? 0; }
  ds_queue_empty(queue: number): boolean { return (this._dsQueues.get(queue)?.length ?? 0) === 0; }

  // ---- Steam API (platform-provided or no-op) ----

  steam_current_game_language(): string { return "english"; }
  steam_inventory_result_destroy(_result: number): void {}

  // ---- Instance position/collision with DS list ----

  instance_place_list(_x: number, _y: number, _classIndex: number, _list: number, _notme: boolean): number { return 0; }
  instance_position_list(_x: number, _y: number, _classIndex: number, _list: number, _notme: boolean): number { return 0; }

  instance_destroy(instance: GMLObject): void {
    this._instanceDestroy(instance);
  }

  instance_exists(classIndex: number): boolean {
    const clazz = this.classes[classIndex];
    if (!clazz) return false;
    return this._getInstances(clazz).length > 0;
  }

  instance_number(classIndex: number): number {
    const clazz = this.classes[classIndex];
    if (!clazz) return 0;
    return this._instanceNumber(clazz);
  }

  // ---- Room navigation ----

  room_goto(id: number, restart = false): void {
    const oldRoom = this._currentRoom;
    if (oldRoom !== null) {
      for (const instance of this.roomVariables) {
        instance.roomend();
      }
      oldRoom.destroy(restart);
    }
    const newRoom = this._roomInstances[id]!;
    this._currentRoom = newRoom;
    this.room = id;
    this.room_speed = this._roomDatas[id]?.speed ?? 60;
    resizeCanvas(this._gfx, this._roomDatas[id]?.size.width ?? 800, this._roomDatas[id]?.size.height ?? 600);
    newRoom.create(restart);
    for (const instance of this.roomVariables) {
      instance.roomstart();
    }
    this.activateMouse(this.mouse_x(), this.mouse_y(), true);
  }

  room_goto_next(): void { this.room_goto(this.room + 1); }
  room_goto_previous(): void { this.room_goto(this.room - 1); }
  room_restart(): void { this.room_goto(this.room); }
  game_restart(): void { this.room_goto(0, true); }

  // ---- Game loop ----

  /** Called once per frame. Override to hook pause/resume, speed control, etc. */
  onTick?: () => void;

  private _runFrame(): void {
    const start = performance.now();
    this.onTick?.();
    if (this._currentRoom) this._currentRoom.draw();
    const end = performance.now();
    const elapsed = end - start;
    const newfps = 1000 / Math.max(0.01, elapsed);
    this.fps_real = 0.9 * this.fps_real + 0.1 * newfps;
    this._drawHandle = scheduleFrame(
      () => this._runFrame(),
      Math.max(0, 1000 / this.room_speed - elapsed),
    );
  }

  // ---- Game startup ----

  async start(config: GameConfig): Promise<void> {
    this._roomDatas = config.rooms;
    this.sprites = config.sprites;
    this.textures = config.textures;
    this.fonts = config.fonts;
    this.classes = config.classes;
    this._classesEnum = config.Classes;
    this.roomCreationCode = config.roomCreationCode ?? [];

    // Populate Sprites enum from sprite data
    for (let i = 0; i < config.sprites.length; i++) {
      this.Sprites[config.sprites[i]!.name] = i;
    }
    // Set up collision stubs (need class count)
    for (let i = 0; i < config.classes.length; i++) {
      (GMLObject.prototype as any)["collision" + i] = noop;
    }

    // Create room instances
    for (let i = 0; i < config.rooms.length; i++) {
      this._roomInstances.push(new GMLRoom(this));
    }

    // Init canvas and input
    if (this._root) {
      const canvas = createCanvas(this._root.doc);
      canvas.id = "reincarnate-canvas";
      this._root.container.appendChild(canvas);
      initCanvas(this._gfx, "reincarnate-canvas", this._root.doc);
    } else {
      initCanvas(this._gfx, "reincarnate-canvas");
    }
    this._gfx.canvas.tabIndex = 0;
    this._gfx.canvas.focus();
    this.setupInput();

    // Load texture sheets
    const sheetCount = Math.max(0, ...config.textures.map((t) => t.sheetId)) + 1;
    const sheetPromises: Promise<HTMLImageElement>[] = [];
    for (let i = 0; i < sheetCount; i++) {
      sheetPromises.push(loadImage(`assets/textures/texture_${i}.png`));
    }
    const sheets = await Promise.all(sheetPromises);
    this.textureSheets.push(...sheets);

    // Start
    this.room_goto(config.initialRoom);
    this._runFrame();
  }
}

// ---- Game config ----

export interface GameConfig {
  rooms: Room[];
  sprites: Sprite[];
  textures: Texture[];
  fonts: Font[];
  classes: (typeof GMLObject)[];
  Classes: Record<string, number>;
  initialRoom: number;
  roomCreationCode?: (((_rt: GameRuntime) => void) | undefined)[];
}

// ---- Factory function ----

export function createGameRuntime(opts?: { root?: RenderRoot }): GameRuntime {
  const rt = new GameRuntime();
  if (opts?.root) rt._root = opts.root;
  return rt;
}

