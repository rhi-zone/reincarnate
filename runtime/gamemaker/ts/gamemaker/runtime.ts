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

class GameRuntime {
  drawHandle = 0;
  currentRoom: GMLRoom | null = null;
  isStepping = false;
  pendingStep: GMLObject[] = [];
  drawguiUsed = false;
  room = 0;
  room_speed = 60;
  fps_real = 1;
  /** All live instances in the current room. */
  roomVariables: GMLObject[] = [];
  /** Registered class constructors (indexed by OBJT order). */
  classes: (typeof GMLObject)[] = [];
  /** Room data array. */
  roomDatas: Room[] = [];
  /** Per-room creation code functions (sparse, indexed by room index). */
  roomCreationCode: (() => void)[] = [];
  /** Sprite data array. */
  sprites: Sprite[] = [];
  /** Texture data array. */
  textures: Texture[] = [];
  /** Loaded texture sheet images. */
  textureSheets: HTMLImageElement[] = [];
  /** Font data array. */
  fonts: Font[] = [];
  /** Classes enum (name→index). */
  classesEnum: Record<string, number> = {};
  /** Room instances list. */
  roomInstances: GMLRoom[] = [];
}

export const rt = new GameRuntime();


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

    const oldRoom = rt.room;
    rt.isStepping = true;

    // Alarms
    for (const instance of rt.roomVariables) {
      if (instance.alarm.length !== 0) {
        for (let i = 0; i < 12; i++) {
          if (instance.alarm[i]) {
            instance.alarm[i]--;
            if (instance.alarm[i] === 0) {
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
      rt.pendingStep = [];
      for (const instance of toStep) {
        if ((instance as any).beginstep === noop) continue;
        instance.xprevious = instance.x;
        instance.yprevious = instance.y;
        instance.beginstep();
        if (oldRoom !== rt.room) break;
      }
      toStep = rt.pendingStep;
    }

    // Step
    toStep = rt.roomVariables;
    while (toStep.length !== 0) {
      rt.pendingStep = [];
      for (const instance of toStep) {
        if ((instance as any).step === noop) continue;
        instance.step();
        if (oldRoom !== rt.room) break;
      }
      toStep = rt.pendingStep;
    }

    // End step
    toStep = rt.roomVariables;
    while (toStep.length !== 0) {
      rt.pendingStep = [];
      for (const instance of toStep) {
        if ((instance as any).endstep === noop) continue;
        instance.endstep();
        if (oldRoom !== rt.room) break;
      }
      toStep = rt.pendingStep;
    }

    rt.isStepping = false;

    // Draw (sorted by depth, descending)
    const sorted = rt.roomVariables.slice().sort((a, b) => b.depth - a.depth);
    for (const instance of sorted) {
      if ((instance as any).draw === noop) continue;
      instance.draw();
      if (oldRoom !== rt.room) break;
    }

    // Draw GUI
    if (rt.drawguiUsed) {
      for (const instance of sorted) {
        if ((instance as any).drawgui === noop) continue;
        instance.drawgui();
        if (oldRoom !== rt.room) break;
      }
    }

    resetFrameInput();
  }

  create(restart = false): void {
    const idx = rt.roomInstances.indexOf(this);
    const data = rt.roomDatas[idx];
    if (!data) return;

    const instances: GMLObject[] = [];
    for (const obj of data.objs) {
      const clazz = rt.classes[obj.obj];
      if (!clazz) continue;
      const proto = clazz.prototype;
      if (!proto.persistent || instance_number_internal(clazz) === 0) {
        instances.push(instance_create_internal(obj.pos.x, obj.pos.y, clazz, true));
      }
    }
    for (const instance of instances) {
      instance.create();
    }
    // Room creation code runs after all instance creation events (GML semantics).
    const creationCode = rt.roomCreationCode[idx];
    if (creationCode) creationCode();
  }

  destroy(restart = false): void {
    for (const obj of rt.roomVariables.slice()) {
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
  rt.roomVariables.push(instance);
  if (!roomStart) {
    instance.create();
  }
  if (!rt.drawguiUsed && (instance as any).drawgui !== noop) {
    rt.drawguiUsed = true;
  }
  if (rt.isStepping) {
    rt.pendingStep.push(instance);
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
  const idx = rt.roomVariables.indexOf(instance);
  if (idx > -1) rt.roomVariables.splice(idx, 1);
}

function instance_number_internal(clazz: typeof GMLObject): number {
  return clazz.instances.reduce(
    (p: number, c: GMLObject) => p + (c.constructor === clazz ? 1 : 0), 0,
  );
}

// ---- Public instance API (called from emitted code) ----

export function instance_create(x: number, y: number, classIndex: number): GMLObject {
  const clazz = rt.classes[classIndex];
  return instance_create_internal(x, y, clazz);
}

export function instance_destroy(instance?: GMLObject): void {
  if (instance === undefined) return;
  instance_destroy_internal(instance);
}

export function instance_exists(classIndex: number): boolean {
  const clazz = rt.classes[classIndex];
  if (!clazz) return false;
  return clazz.instances.length > 0;
}

export function instance_number(classIndex: number): number {
  const clazz = rt.classes[classIndex];
  if (!clazz) return 0;
  return instance_number_internal(clazz);
}

// ---- Room navigation ----

export function room_goto(id: number, restart = false): void {
  const oldRoom = rt.currentRoom;
  if (oldRoom !== null) {
    for (const instance of rt.roomVariables) {
      instance.roomend();
    }
    oldRoom.destroy(restart);
  }
  const newRoom = rt.roomInstances[id];
  rt.currentRoom = newRoom;
  rt.room = id;
  rt.room_speed = rt.roomDatas[id]?.speed ?? 60;
  resizeCanvas(rt.roomDatas[id]?.size.width ?? 800, rt.roomDatas[id]?.size.height ?? 600);
  newRoom.create(restart);
  for (const instance of rt.roomVariables) {
    instance.roomstart();
  }
  activateMouse(mouse_x(), mouse_y(), true);
}

export function room_goto_next(): void { room_goto(rt.room + 1); }
export function room_goto_previous(): void { room_goto(rt.room - 1); }
export function room_restart(): void { room_goto(rt.room); }
export function game_restart(): void { room_goto(0, true); }

// ---- Game loop ----

function runFrame(): void {
  const start = performance.now();
  if (rt.currentRoom) rt.currentRoom.draw();
  const end = performance.now();
  const elapsed = end - start;
  const newfps = 1000 / Math.max(0.01, elapsed);
  rt.fps_real = 0.9 * rt.fps_real + 0.1 * newfps;
  rt.drawHandle = scheduleFrame(runFrame, Math.max(0, 1000 / rt.room_speed - elapsed));
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
  roomCreationCode?: (() => void)[];
}

export async function startGame(config: GameConfig): Promise<void> {
  rt.roomDatas = config.rooms;
  rt.sprites = config.sprites;
  rt.textures = config.textures;
  rt.fonts = config.fonts;
  rt.classes = config.classes;
  rt.classesEnum = config.Classes;
  rt.roomCreationCode = config.roomCreationCode ?? [];

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
    rt.roomInstances.push(new GMLRoom());
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
  rt.textureSheets.push(...sheets);

  // Start
  room_goto(config.initialRoom);
  runFrame();
}
