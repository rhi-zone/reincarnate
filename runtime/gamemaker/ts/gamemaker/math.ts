// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** GML math functions — PRNG, trig (degrees), standard math. */

import type { GameRuntime } from "./runtime";
import { currentWallTimeMs } from "../shared/platform/timing";

// ---- Seedable PRNG (xorshift128) ----

const UINT32_MAX = 4294967295;
const UINT32_OFFSET = 2147483648;

// HANDWRITTEN
export class XorGen {
  x: number[];
  i: number;

  constructor(seed: number) {
    this.x = [];
    this.i = 0;

    if (seed === (seed | 0)) {
      this.x[0] = seed;
    }
    while (this.x.length < 8) this.x.push(0);
    let found = false;
    for (let j = 0; j < 8; j++) {
      if (this.x[j] !== 0) { found = true; break; }
    }
    if (!found) this.x[7] = -1;

    // Warm up
    for (let j = 0; j < 256; j++) this.next();
  }

  next(): number {
    const X = this.x;
    let i = this.i;
    let t = X[i]!; t ^= (t >>> 7);
    let v = t ^ (t << 24);
    t = X[(i + 1) & 7]!; v ^= t ^ (t >>> 10);
    t = X[(i + 3) & 7]!; v ^= t ^ (t >>> 3);
    t = X[(i + 4) & 7]!; v ^= t ^ (t << 7);
    t = X[(i + 7) & 7]!; t = t ^ (t << 13); v ^= t ^ (t << 9);
    X[i] = v;
    this.i = (i + 1) & 7;
    return v;
  }
}

// HANDWRITTEN
export class MathState {
  prng = new XorGen(0);
}

// ---- PRNG API (stateful — needs runtime) ----

// HANDWRITTEN
export function createMathAPI(rt: GameRuntime) {
  function random_set_seed(seed: number): void {
    rt._math.prng = new XorGen(seed);
  }

  function randomize(): void {
    rt._math.prng = new XorGen(currentWallTimeMs());
  }

  function random(max: number): number {
    return (rt._math.prng.next() + UINT32_OFFSET) * max / UINT32_MAX;
  }

  function random_range(min: number, max: number): number {
    return min + (rt._math.prng.next() + UINT32_OFFSET) * (max - min) / UINT32_MAX;
  }

  function irandom(max: number): number {
    if (max < 0 || !isFinite(max)) return 0;
    const maxp1 = max + 1;
    let res: number;
    do {
      res = Math.floor((rt._math.prng.next() + UINT32_OFFSET) * maxp1 / UINT32_MAX);
    } while (res > max);
    return res;
  }

  function irandom_range(min: number, max: number): number {
    if (max < min || !isFinite(min) || !isFinite(max)) return 0;
    const deltap1 = max - min + 1;
    let res: number;
    do {
      res = min + Math.floor((rt._math.prng.next() + UINT32_OFFSET) * deltap1 / UINT32_MAX);
    } while (res > max);
    return res;
  }

  function choose(...args: unknown[]): unknown {
    return args[irandom(args.length - 1)];
  }

  return {
    random_set_seed, randomize, random, random_range,
    irandom, irandom_range, choose,
  };
}

// ---- Standard math (pure — no runtime needed) ----

// ln, max, min have no IR bodies — they remain handwritten.
// HANDWRITTEN
export const { log: ln, max, min } = Math;

// HANDWRITTEN
export function mean(...nums: number[]): number { return nums.reduce((p, c) => p + c, 0) / nums.length; }
// HANDWRITTEN
export function median(...nums: number[]): number {
  const sorted = nums.slice().sort((a, b) => a - b);
  const mid = sorted.length >> 1;
  return sorted.length % 2 === 0 ? (sorted[mid - 1]! + sorted[mid]!) / 2 : sorted[mid]!;
}

// HANDWRITTEN
export function int64(n: number): number { return n | 0; }

// ---- Type conversion ----

/** Truncate to 32-bit signed integer. */
// HANDWRITTEN
export function int(n: unknown): number { return Number(n) | 0; }
/** Truncate to 32-bit unsigned integer. */
// HANDWRITTEN
export function uint(n: unknown): number { return Number(n) >>> 0; }
/** Convert to number (GML real). */
// HANDWRITTEN
export function real(n: unknown): number { return Number(n); }
/** Convert to string (GML string). Optional second arg is decimal places; extra args ignored. */
// HANDWRITTEN
export function string(n: unknown, ..._rest: unknown[]): string { return String(n); }
// HANDWRITTEN
export function math_get_epsilon(): number { return 0.00001; }
// HANDWRITTEN
export function is_bool(val: unknown): val is boolean { return typeof val === "boolean"; }
