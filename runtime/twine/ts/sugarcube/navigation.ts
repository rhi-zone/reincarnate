/** SugarCube navigation — passage registry, goto/back/return/include. */

import * as State from "./state";
import * as Output from "./output";
import * as Events from "./events";
import type { RenderRoot } from "../../../shared/ts/render-root";

class SCNavigation {
  passages: Map<string, () => void> = new Map();
  passageTags: Map<string, string[]> = new Map();
  currentPassage = "";
}

export const nav = new SCNavigation();

// ---------------------------------------------------------------------------
// SugarCubeRuntime
// ---------------------------------------------------------------------------

export class SugarCubeRuntime {
  /**
   * Register all passage functions and start the story.
   *
   * Follows SugarCube's init order:
   * 1. Register all passages/widgets
   * 2. Run StoryInit passage (if it exists)
   * 3. Navigate to the explicit start passage (or first registered)
   */
  start(passageMap: Record<string, () => void>, startPassage?: string, tagMap?: Record<string, string[]>, opts?: { root?: RenderRoot }): void {
    if (opts?.root) {
      Output.output.doc = opts.root.doc;
      Output.output.container = opts.root.container as Element;
    }

    for (const [name, fn] of Object.entries(passageMap)) {
      nav.passages.set(name, fn);
    }
    if (tagMap) {
      for (const [name, tags] of Object.entries(tagMap)) {
        nav.passageTags.set(name, tags);
      }
    }

    // Run StoryInit if it exists (initializes story variables)
    const storyInit = nav.passages.get("StoryInit");
    if (storyInit) {
      try {
        storyInit();
      } catch (e) {
        console.error("[navigation] error in StoryInit:", e);
      }
    }

    // Navigate to the explicit start passage, or fall back to first registered
    const target = startPassage || Object.keys(passageMap)[0];
    if (target) {
      goto(target);
    }

    Events.trigger(":storyready");
  }
}

export function createSugarCubeRuntime(): SugarCubeRuntime {
  return new SugarCubeRuntime();
}

/** Run a special passage by name, if it exists. Errors are logged. */
function runSpecial(name: string): void {
  const fn = nav.passages.get(name);
  if (fn) {
    try {
      fn();
    } catch (e) {
      console.error(`[navigation] error in ${name}:`, e);
    }
  }
}

/** Render a passage with full event lifecycle and special passage support. */
function renderPassage(target: string, fn: () => void): void {
  State.clearTemps();
  nav.currentPassage = target;
  Output.clear();

  // Check for nobr tag
  const tags = nav.passageTags.get(target) || [];
  if (tags.includes("nobr")) {
    Output.setNobr(true);
  }

  // Build a passage object matching SugarCube's event data format.
  const passageObj = { title: target, tags };

  Events.trigger(":passageinit", { passage: passageObj });

  // PassageReady runs after :passageinit, before the main passage
  runSpecial("PassageReady");

  Events.trigger(":passagestart", { passage: passageObj });

  // PassageHeader content is prepended before the main passage
  runSpecial("PassageHeader");

  try {
    fn();
  } catch (e) {
    console.error(`[navigation] error in passage "${target}":`, e);
    Output.text(`Error in passage "${target}": ${e}`);
  }

  // PassageFooter content is appended after the main passage
  runSpecial("PassageFooter");

  Events.trigger(":passagerender", { passage: passageObj });

  // PassageDone runs after :passagerender, before flush
  runSpecial("PassageDone");

  Output.setNobr(false);
  Output.flush();

  Events.trigger(":passageend", { passage: passageObj });
  Events.trigger(":passagedisplay", { passage: passageObj });
}

/** Navigate to a passage by name. */
export function goto(target: string): void {
  const fn = nav.passages.get(target);
  if (!fn) {
    console.error(`[navigation] passage not found: "${target}"`);
    return;
  }
  State.pushMoment(target);
  renderPassage(target, fn);
}

/** Go back to the previous passage. */
export function back(): void {
  const title = State.popMoment();
  if (title === undefined) {
    console.warn("[navigation] no history to go back to");
    return;
  }
  const fn = nav.passages.get(title);
  if (!fn) {
    console.error(`[navigation] passage not found on back: "${title}"`);
    return;
  }
  renderPassage(title, fn);
}

/** Return to the previous passage (alias for back). */
// Using a wrapper to avoid JS reserved word in export.
export { returnNav as return };
function returnNav(): void {
  back();
}

/** Include (embed) another passage inline without navigation. */
export function include(passage: string): void {
  const fn = nav.passages.get(passage);
  if (!fn) {
    console.error(`[navigation] passage not found for include: "${passage}"`);
    return;
  }
  try {
    fn();
  } catch (e) {
    console.error(`[navigation] error in included passage "${passage}":`, e);
    Output.text(`Error in passage "${passage}": ${e}`);
  }
}

/** Get the current passage name. */
export function current(): string {
  return nav.currentPassage;
}

/** Check if a passage exists in the registry. */
export function has(name: string): boolean {
  return nav.passages.has(name);
}

/** Get a passage function by name (for widget/engine lookup). */
export function getPassage(name: string): (() => void) | undefined {
  return nav.passages.get(name);
}

/** Get the tags for a passage. */
export function getTags(name: string): string[] {
  return nav.passageTags.get(name) || [];
}

/** Get all passage names in the registry. */
export function allPassages(): string[] {
  return Array.from(nav.passages.keys());
}

/** Register commands for navigation. */
export function initCommands(registerCommand: (id: string, binding: string, handler: () => void) => void): void {
  registerCommand("go-back", "", () => back());
  registerCommand("restart", "", () => location.reload());
}
