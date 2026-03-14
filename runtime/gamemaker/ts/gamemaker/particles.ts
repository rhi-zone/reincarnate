/**
 * Particle system types and helpers for GML runtime.
 *
 * Extracted from runtime.ts.
 */

export interface PartTypeConfig {
  life: [number, number];
  speed: [number, number, number, number];       // min, max, inc, wiggle
  direction: [number, number, number, number];   // min, max, inc, wiggle
  colors: number[];          // 1-3 color values for gradient
  alphas: number[];          // 1-3 alpha values for gradient
  hsvRange: { h1: number; h2: number; s1: number; s2: number; v1: number; v2: number } | null;
  size: [number, number, number, number];        // min, max, inc, wiggle
  sizeX: [number, number, number, number] | null;
  sizeY: [number, number, number, number] | null;
  orientation: [number, number, number, number, boolean];  // min, max, inc, wiggle, relative
  scale: [number, number];
  shape: number;             // 0=pixel, 1=disk, 2=square, 3=line, 4=star, 5=circle, 6=ring
  sprite: { spr: number; anim: boolean; stretch: boolean; random: boolean } | null;
  gravity: [number, number]; // gx, gy
}

export interface PartInst {
  x: number; y: number; vx: number; vy: number;
  life: number; maxLife: number;
  typeId: number;
  colorOverride: number | null;
  size: number; angle: number;
  sizeInc: number; angleInc: number; speedInc: number;
  sizeWiggle: number; angleWiggle: number; speedWiggle: number;
}

export interface PartEmitter {
  x1: number; y1: number; x2: number; y2: number; shape: number; dist: number;
  /** Stream config set by `part_emitter_stream`; emitted each step. */
  stream?: { typeId: number; num: number };
}

export interface PartSystem {
  particles: PartInst[];
  autoDraw: boolean;
  autoUpdate: boolean;
  /** Draw order: true = oldest first, false = newest first. */
  drawOrder?: boolean;
  depth: number;
  pos: [number, number];
  emitters: Map<number, PartEmitter>;
  nextEmitId: number;
}

export function defaultPartType(): PartTypeConfig {
  return {
    life: [1, 1], speed: [0, 0, 0, 0], direction: [0, 360, 0, 0],
    colors: [0xffffff], alphas: [1], hsvRange: null,
    size: [1, 1, 0, 0], sizeX: null, sizeY: null,
    orientation: [0, 0, 0, 0, false], scale: [1, 1],
    shape: 1, sprite: null, gravity: [0, 0],
  };
}

export function randf(min: number, max: number): number { return min + Math.random() * (max - min); }

export function hsv2rgb(h: number, s: number, v: number): number {
  const hi = Math.floor(h / 60) % 6, f = h / 60 - Math.floor(h / 60);
  const [p, q, t] = [v * (1 - s), v * (1 - f * s), v * (1 - (1 - f) * s)];
  const [r, g, b] = [[v, t, p], [q, v, p], [p, v, t], [p, q, v], [t, p, v], [v, p, q]][hi] as [number, number, number];
  return ((r * 255) << 16) | ((g * 255) << 8) | (b * 255 | 0);
}

export function lerpColor(c1: number, c2: number, t: number): number {
  const r = ((c1 & 0xff) + ((c2 & 0xff) - (c1 & 0xff)) * t) | 0;
  const g = (((c1 >> 8) & 0xff) + (((c2 >> 8) & 0xff) - ((c1 >> 8) & 0xff)) * t) | 0;
  const b = (((c1 >> 16) & 0xff) + (((c2 >> 16) & 0xff) - ((c1 >> 16) & 0xff)) * t) | 0;
  return r | (g << 8) | (b << 16);
}

/** Returns the byte size of a GML buffer type constant, or 0 for string types. */
export function bufferTypeSize(type: number): number {
  switch (type) {
    case 1: case 2: case 13: return 1;   // buffer_u8, buffer_s8, buffer_bool
    case 3: case 4: return 2;            // buffer_u16, buffer_s16
    case 5: case 6: case 7: return 4;    // buffer_u32, buffer_s32, buffer_f32
    case 8: case 16: return 8;           // buffer_f64, buffer_u64
    default: return 0;                   // buffer_string, buffer_text (variable)
  }
}

// PRNG constants (used by Math API methods)
export const _UINT32_MAX = 4294967295;
export const _UINT32_OFFSET = 2147483648;
