/** Harlowe navigation — passage registry, goto, display (include). */

import * as State from "./state";
import * as Output from "./output";
import type { ContentNode } from "./output";

/** Registry of passage name -> passage function. */
const passages: Map<string, () => ContentNode[]> = new Map();

/** Registry of passage name -> tags. */
const passageTags: Map<string, string[]> = new Map();

/** Current passage name. */
let currentPassage = "";

/** Register all passage functions and start the story.
 *
 * Follows the same init pattern as Harlowe:
 * 1. Register all passages
 * 2. Navigate to the start passage
 */
export function startStory(
  passageMap: Record<string, () => ContentNode[]>,
  startPassage?: string,
  tagMap?: Record<string, string[]>,
): void {
  for (const [name, fn] of Object.entries(passageMap)) {
    passages.set(name, fn);
  }
  if (tagMap) {
    for (const [name, tags] of Object.entries(tagMap)) {
      passageTags.set(name, tags);
    }
  }

  // Provide passage lookup to output.ts for (display:) rendering
  Output.setPassageLookup((name: string) => passages.get(name));

  // Navigate to the explicit start passage, or fall back to first registered
  const target = startPassage || Object.keys(passageMap)[0];
  if (target) {
    goto(target);
  }
}

/** Render a passage with full lifecycle. */
function renderPassage(target: string, fn: () => ContentNode[]): void {
  State.clearTemps();
  currentPassage = target;
  Output.clear();

  const container = document.getElementById("passages");
  if (!container) return;

  try {
    const nodes = fn();
    Output.render(container, nodes);
  } catch (e) {
    console.error(`[harlowe] error in passage "${target}":`, e);
    container.appendChild(document.createTextNode(`Error in passage "${target}": ${e}`));
  }
}

/** Navigate to a passage by name. */
export function goto(target: string): void {
  const fn = passages.get(target);
  if (!fn) {
    console.error(`[harlowe] passage not found: "${target}"`);
    return;
  }
  State.pushMoment(target);
  renderPassage(target, fn);
}

/** Include (embed) another passage inline without navigation.
 *  Returns the passage's content nodes for inline rendering. */
export function display(passage: string): ContentNode[] {
  const fn = passages.get(passage);
  if (!fn) {
    console.error(`[harlowe] passage not found for display: "${passage}"`);
    return [];
  }
  try {
    return fn();
  } catch (e) {
    console.error(`[harlowe] error in displayed passage "${passage}":`, e);
    return [`Error in passage "${passage}": ${e}`];
  }
}

/** Get the current passage name. */
export function current(): string {
  return currentPassage;
}

/** Check if a passage exists in the registry. */
export function has(name: string): boolean {
  return passages.has(name);
}

/** Get a passage function by name. */
export function getPassage(name: string): (() => ContentNode[]) | undefined {
  return passages.get(name);
}

/** Get the tags for a passage. */
export function getTags(name: string): string[] {
  return passageTags.get(name) || [];
}

/** Get all passage names in the registry. */
export function allPassages(): string[] {
  return Array.from(passages.keys());
}

/** Register commands for navigation. */
export function initCommands(
  registerCommand: (id: string, binding: string, handler: () => void) => void,
): void {
  registerCommand("go-back", "", () => {
    const title = State.popMoment();
    if (title) {
      const fn = passages.get(title);
      if (fn) renderPassage(title, fn);
    }
  });
  registerCommand("restart", "", () => location.reload());
}
