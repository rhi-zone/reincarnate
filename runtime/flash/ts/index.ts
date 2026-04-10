// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

import "./flash/vector"; // AS3 Vector compat (Array prototype patches)

// HANDWRITTEN
export { TimingShim } from "./timing";
// HANDWRITTEN
export { InputShim } from "./input";
// HANDWRITTEN
export { RendererShim } from "./renderer";
// HANDWRITTEN
export { AudioShim } from "./audio";
// HANDWRITTEN
export { SaveShim } from "./save";
// HANDWRITTEN
export { UiShim } from "./ui";
// HANDWRITTEN
export { FlashMemory } from "./flash/memory";

import { TimingShim } from "./timing";
import { InputShim } from "./input";
import { RendererShim } from "./renderer";
import { AudioShim } from "./audio";
import { SaveShim } from "./save";
import { UiShim } from "./ui";
import { FlashMemory } from "./flash/memory";

/** Holds all Flash shim state for one game instance. */
// HANDWRITTEN
export class FlashShims {
  constructor(
    public readonly timing: TimingShim,
    public readonly input: InputShim,
    public readonly renderer: RendererShim,
    public readonly audio: AudioShim,
    public readonly save: SaveShim,
    public readonly ui: UiShim,
    public readonly memory: FlashMemory,
  ) {}

  static create(canvas: HTMLCanvasElement, savePrefix = "reincarnate:"): FlashShims {
    return new FlashShims(
      new TimingShim(),
      new InputShim(canvas),
      new RendererShim(canvas),
      new AudioShim(),
      new SaveShim(savePrefix),
      new UiShim(),
      new FlashMemory(),
    );
  }
}

