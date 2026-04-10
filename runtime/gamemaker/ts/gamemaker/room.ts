// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/**
 * GMLRoom — room simulation and game loop frame orchestration.
 *
 * Extracted from runtime.ts. Each room manages instance lifecycle,
 * event dispatch, and frame rendering.
 */

import type { GameRuntime } from "./runtime";
import type { GMLObject } from "./object";
import { noop } from "./constants";

// HANDWRITTEN
export class GMLRoom {
  constructor(private rt: GameRuntime) {}

  draw(): void {
    const rt = this.rt;
    const ctx = rt._gfx.ctx;
    ctx.fillStyle = "black";
    ctx.fillRect(0, 0, rt._gfx.canvas.width, rt._gfx.canvas.height);

    const oldRoom = rt.room;
    rt._isStepping = true;

    const deact = rt._deactivatedInstances;

    // Alarms
    for (const instance of rt.roomVariables) {
      if (deact.has(instance)) continue;
      if (instance.alarm.length !== 0) {
        for (let i = 0; i < 12; i++) {
          const alarmVal = instance.alarm[i];
          if (alarmVal) {
            instance.alarm[i] = alarmVal - 1;
            if (alarmVal - 1 === 0) {
              delete instance.alarm[i];
              const method = (instance as any)["alarm" + i];
              if (method !== noop) { rt._self = instance; method.call(instance); rt._self = null; }
              if (oldRoom !== rt.room) break;
            }
          }
        }
      }
    }

    // Built-in motion model (applied before begin-step, per GML semantics)
    for (const instance of rt.roomVariables) {
      if (deact.has(instance)) continue;
      const { speed, direction, hspeed, vspeed, friction, gravity, gravity_direction } = instance;
      if (speed === 0 && gravity === 0 && hspeed === 0 && vspeed === 0) continue;
      // Apply gravity: decompose along gravity_direction (GML: 0=right,90=up,180=left,270=down; y-axis flipped)
      const gravRad = gravity_direction * Math.PI / 180;
      let hs = hspeed + Math.cos(gravRad) * gravity;
      let vs = vspeed + (-Math.sin(gravRad)) * gravity;
      // Recompute speed/direction from hs/vs
      let spd = Math.sqrt(hs * hs + vs * vs);
      // Apply friction: reduce speed (clamped to 0)
      if (friction !== 0 && spd > 0) {
        spd = Math.max(0, spd - friction);
        if (spd === 0) { hs = 0; vs = 0; }
        else { hs = Math.cos(Math.atan2(-vs, hs)) * spd; vs = -Math.sin(Math.atan2(-vs, hs)) * spd; }
      }
      // Apply motion
      instance.x += hs;
      instance.y += vs;
      // Keep speed/direction/hspeed/vspeed in sync
      instance.hspeed = hs;
      instance.vspeed = vs;
      instance.speed = spd;
      instance.direction = spd > 0 ? Math.atan2(-vs, hs) * 180 / Math.PI : instance.direction;
    }

    // Begin step
    let toStep: GMLObject[] = rt.roomVariables;
    while (toStep.length !== 0) {
      rt._pendingStep = [];
      for (const instance of toStep) {
        if (deact.has(instance)) continue;
        if ((instance as any).beginstep === noop) continue;
        instance.xprevious = instance.x;
        instance.yprevious = instance.y;
        rt._self = instance; instance.beginstep(); rt._self = null;
        if (oldRoom !== rt.room) break;
      }
      toStep = rt._pendingStep;
    }

    // Step
    toStep = rt.roomVariables;
    while (toStep.length !== 0) {
      rt._pendingStep = [];
      for (const instance of toStep) {
        if (deact.has(instance)) continue;
        if ((instance as any).step === noop) continue;
        rt._self = instance; instance.step(); rt._self = null;
        if (oldRoom !== rt.room) break;
      }
      toStep = rt._pendingStep;
    }

    // End step
    toStep = rt.roomVariables;
    while (toStep.length !== 0) {
      rt._pendingStep = [];
      for (const instance of toStep) {
        if (deact.has(instance)) continue;
        if ((instance as any).endstep === noop) continue;
        rt._self = instance; instance.endstep(); rt._self = null;
        if (oldRoom !== rt.room) break;
      }
      toStep = rt._pendingStep;
    }

    rt._isStepping = false;

    // Draw (sorted by depth, descending; skip deactivated)
    const sorted = rt.roomVariables.slice().sort((a, b) => b.depth - a.depth);
    for (const instance of sorted) {
      if (deact.has(instance)) continue;
      if ((instance as any).draw === noop) continue;
      rt._self = instance; instance.draw(); rt._self = null;
      if (oldRoom !== rt.room) break;
    }

    // Draw GUI
    if (rt._drawguiUsed) {
      for (const instance of sorted) {
        if (deact.has(instance)) continue;
        if ((instance as any).drawgui === noop) continue;
        rt._self = instance; instance.drawgui(); rt._self = null;
        if (oldRoom !== rt.room) break;
      }
    }

    // Particle systems: update + auto-draw
    for (const [, sys] of rt._partSystems) {
      if (sys.autoUpdate) rt._partUpdate(sys);
      if (sys.autoDraw) rt._partDraw(sys);
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
      rt._self = instance; instance.create(); rt._self = null;
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
