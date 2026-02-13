/**
 * GML Runtime — game loop, GMLObject base class, room system.
 */

import { initCanvas, getCanvas, getCtx, resizeCanvas, loadImage, scheduleFrame } from "./platform";
import { setupInput, mouse_x, mouse_y, resetFrameInput, dispatchKeyPress, activateMouse } from "./input";
import type { Sprite } from "../../data/sprites";
import type { Texture } from "../../data/textures";
import type { Font } from "../../data/fonts";
import type { Room } from "../../data/rooms";
import { drawSprite } from "./draw";

// Re-exports for class_preamble
export { Colors, HAligns, VAligns } from "./color";

// ---- Global state ----

const noop = function () {};

let drawHandle = 0;
let currentRoom: GMLRoom | null = null;
let isStepping = false;
let pendingStep: GMLObject[] = [];
let drawguiUsed = false;

export let room = 0;
export let room_speed = 60;
export let fps_real = 1;

/** All live instances in the current room. */
export const roomVariables: GMLObject[] = [];

/** Registered class constructors (indexed by OBJT order). */
let classes: (typeof GMLObject)[] = [];

/** Room data array. */
let roomDatas: Room[] = [];

/** Sprite data array. */
export let sprites: Sprite[] = [];

/** Texture data array. */
export let textures: Texture[] = [];

/** Loaded texture sheet images. */
export const textureSheets: HTMLImageElement[] = [];

/** Font data array. */
export let fonts: Font[] = [];

/** Classes enum (name→index). */
let classesEnum: Record<string, number> = {};

/** Room instances list. */
const roomInstances: GMLRoom[] = [];

/** Symbol for the internal "active" flag (mouse hover state). */
export const ACTIVE = Symbol("active");

// ---- Timing ----

export const timing = {
  tick() {
    // GameMaker uses its own setTimeout-based loop, not rAF
  },
};

// ---- GMLObject ----

const __baseproto = Object.getPrototypeOf(class {});

export class GMLObject {
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

  static __instances: GMLObject[] = [];

  static get instances(): GMLObject[] {
    if (!this.hasOwnProperty("__instances")) {
      (this as any).__instances = [];
    }
    return (this as any).__instances;
  }

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
    drawSprite(this.sprite_index, this.image_index, this.x, this.y, this);
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

// Sprites enum — populated by startGame
export const Sprites: Record<string, number> = {};

// ---- GMLRoom ----

class GMLRoom {
  draw(): void {
    const ctx = getCtx();
    ctx.fillStyle = "black";
    ctx.fillRect(0, 0, getCanvas().width, getCanvas().height);

    const oldRoom = room;
    isStepping = true;

    // Alarms
    for (const instance of roomVariables) {
      if (instance.alarm.length !== 0) {
        for (let i = 0; i < 12; i++) {
          if (instance.alarm[i]) {
            instance.alarm[i]--;
            if (instance.alarm[i] === 0) {
              delete instance.alarm[i];
              const method = (instance as any)["alarm" + i];
              if (method !== noop) method.call(instance);
              if (oldRoom !== room) break;
            }
          }
        }
      }
    }

    // Begin step
    let toStep: GMLObject[] = roomVariables;
    while (toStep.length !== 0) {
      pendingStep = [];
      for (const instance of toStep) {
        if ((instance as any).beginstep === noop) continue;
        instance.xprevious = instance.x;
        instance.yprevious = instance.y;
        instance.beginstep();
        if (oldRoom !== room) break;
      }
      toStep = pendingStep;
    }

    // Step
    toStep = roomVariables;
    while (toStep.length !== 0) {
      pendingStep = [];
      for (const instance of toStep) {
        if ((instance as any).step === noop) continue;
        instance.step();
        if (oldRoom !== room) break;
      }
      toStep = pendingStep;
    }

    // End step
    toStep = roomVariables;
    while (toStep.length !== 0) {
      pendingStep = [];
      for (const instance of toStep) {
        if ((instance as any).endstep === noop) continue;
        instance.endstep();
        if (oldRoom !== room) break;
      }
      toStep = pendingStep;
    }

    isStepping = false;

    // Draw (sorted by depth, descending)
    const sorted = roomVariables.slice().sort((a, b) => b.depth - a.depth);
    for (const instance of sorted) {
      if ((instance as any).draw === noop) continue;
      instance.draw();
      if (oldRoom !== room) break;
    }

    // Draw GUI
    if (drawguiUsed) {
      for (const instance of sorted) {
        if ((instance as any).drawgui === noop) continue;
        instance.drawgui();
        if (oldRoom !== room) break;
      }
    }

    resetFrameInput();
  }

  create(restart = false): void {
    const idx = roomInstances.indexOf(this);
    const data = roomDatas[idx];
    if (!data) return;

    const instances: GMLObject[] = [];
    for (const obj of data.objs) {
      const clazz = classes[obj.obj];
      if (!clazz) continue;
      const proto = clazz.prototype;
      if (!proto.persistent || instance_number_internal(clazz) === 0) {
        instances.push(instance_create_internal(obj.pos.x, obj.pos.y, clazz, true));
      }
    }
    for (const instance of instances) {
      instance.create();
    }
  }

  destroy(restart = false): void {
    for (const obj of roomVariables.slice()) {
      if (restart || !obj.persistent) {
        instance_destroy_internal(obj);
      }
    }
  }
}

// ---- Instance management (internal) ----

function instance_create_internal(x: number, y: number, clazz: typeof GMLObject, roomStart = false): GMLObject {
  const instance = new (clazz as any)();
  // Walk prototype chain and push to each class's instances array
  let c: any = instance.constructor;
  while (c !== __baseproto) {
    c.instances.push(instance);
    c = Object.getPrototypeOf(c);
  }
  instance.xstart = instance.x = x;
  instance.ystart = instance.y = y;
  roomVariables.push(instance);
  if (!roomStart) {
    instance.create();
  }
  if (!drawguiUsed && (instance as any).drawgui !== noop) {
    drawguiUsed = true;
  }
  if (isStepping) {
    pendingStep.push(instance);
  }
  return instance;
}

function instance_destroy_internal(instance: GMLObject): void {
  instance.destroy();
  let c: any = instance.constructor;
  while (c !== __baseproto) {
    const arr: GMLObject[] = c.instances;
    const idx = arr.indexOf(instance);
    if (idx > -1) arr.splice(idx, 1);
    c = Object.getPrototypeOf(c);
  }
  const idx = roomVariables.indexOf(instance);
  if (idx > -1) roomVariables.splice(idx, 1);
}

function instance_number_internal(clazz: typeof GMLObject): number {
  return clazz.instances.reduce(
    (p: number, c: GMLObject) => p + (c.constructor === clazz ? 1 : 0), 0,
  );
}

// ---- Public instance API (called from emitted code) ----

export function instance_create(x: number, y: number, classIndex: number): GMLObject {
  const clazz = classes[classIndex];
  return instance_create_internal(x, y, clazz);
}

export function instance_destroy(instance?: GMLObject): void {
  if (instance === undefined) return;
  instance_destroy_internal(instance);
}

export function instance_exists(classIndex: number): boolean {
  const clazz = classes[classIndex];
  if (!clazz) return false;
  return clazz.instances.length > 0;
}

export function instance_number(classIndex: number): number {
  const clazz = classes[classIndex];
  if (!clazz) return 0;
  return instance_number_internal(clazz);
}

// ---- Room navigation ----

export function room_goto(id: number, restart = false): void {
  const oldRoom = currentRoom;
  if (oldRoom !== null) {
    for (const instance of roomVariables) {
      instance.roomend();
    }
    oldRoom.destroy(restart);
  }
  const newRoom = roomInstances[id];
  currentRoom = newRoom;
  room = id;
  room_speed = roomDatas[id]?.speed ?? 60;
  resizeCanvas(roomDatas[id]?.size.width ?? 800, roomDatas[id]?.size.height ?? 600);
  newRoom.create(restart);
  for (const instance of roomVariables) {
    instance.roomstart();
  }
  activateMouse(mouse_x(), mouse_y(), true);
}

export function room_goto_next(): void { room_goto(room + 1); }
export function room_goto_previous(): void { room_goto(room - 1); }
export function room_restart(): void { room_goto(room); }
export function game_restart(): void { room_goto(0, true); }

// ---- Game loop ----

function drawit(): void {
  const start = performance.now();
  if (currentRoom) currentRoom.draw();
  const end = performance.now();
  const elapsed = end - start;
  const newfps = 1000 / Math.max(0.01, elapsed);
  fps_real = 0.9 * fps_real + 0.1 * newfps;
  drawHandle = scheduleFrame(drawit, Math.max(0, 1000 / room_speed - elapsed));
}

// ---- Game startup ----

export interface GameConfig {
  rooms: Room[];
  sprites: Sprite[];
  textures: Texture[];
  fonts: Font[];
  classes: (typeof GMLObject)[];
  Classes: Record<string, number>;
  initialRoom: number;
}

export async function startGame(config: GameConfig): Promise<void> {
  roomDatas = config.rooms;
  sprites = config.sprites;
  textures = config.textures;
  fonts = config.fonts;
  classes = config.classes;
  classesEnum = config.Classes;

  // Populate Sprites enum from sprite data
  for (let i = 0; i < config.sprites.length; i++) {
    Sprites[config.sprites[i].name] = i;
  }

  // Set up collision stubs (need class count)
  for (let i = 0; i < config.classes.length; i++) {
    (GMLObject.prototype as any)["collision" + i] = noop;
  }

  // Create room instances
  for (let i = 0; i < config.rooms.length; i++) {
    roomInstances.push(new GMLRoom());
  }

  // Init canvas and input
  const { canvas } = initCanvas("reincarnate-canvas");
  canvas.tabIndex = 0;
  canvas.focus();
  setupInput();

  // Load texture sheets
  const sheetCount = Math.max(0, ...config.textures.map((t) => t.sheetId)) + 1;
  const sheetPromises: Promise<HTMLImageElement>[] = [];
  for (let i = 0; i < sheetCount; i++) {
    sheetPromises.push(loadImage(`assets/textures/texture_${i}.png`));
  }
  const sheets = await Promise.all(sheetPromises);
  textureSheets.push(...sheets);

  // Start
  room_goto(config.initialRoom);
  drawit();
}
