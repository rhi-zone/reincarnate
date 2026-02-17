/** Harlowe navigation — passage registry, goto, display (include). */

import * as State from "./state";
import { HarloweContext, cancelTimers, departOldPassage } from "./context";
import type { RenderRoot, DocumentFactory } from "../../../shared/ts/render-root";

/** Passage function type — receives h context, returns void. */
export type PassageFn = (h: HarloweContext) => void;

class HarloweNavigation {
  passages: Map<string, PassageFn> = new Map();
  passageTags: Map<string, string[]> = new Map();
  currentPassage = "";
  lastDepart: { name: string; duration?: string } | undefined;
  /** Document factory — defaults to global document. */
  doc: DocumentFactory = document;
  /** Container element for rendering passages (tw-story equivalent). */
  container: Element | ShadowRoot | null = null;
}

export const nav = new HarloweNavigation();

// ---------------------------------------------------------------------------
// HarloweRuntime
// ---------------------------------------------------------------------------

export class HarloweRuntime {
  /**
   * Register all passage functions and start the story.
   *
   * Follows the same init pattern as Harlowe:
   * 1. Register all passages
   * 2. Navigate to the start passage
   */
  start(
    passageMap: Record<string, PassageFn>,
    startPassage?: string,
    tagMap?: Record<string, string[]>,
    opts?: { root?: RenderRoot },
  ): void {
    if (opts?.root) {
      nav.doc = opts.root.doc;
      nav.container = opts.root.container;
    }

    for (const [name, fn] of Object.entries(passageMap)) {
      nav.passages.set(name, fn);
    }
    if (tagMap) {
      for (const [name, tags] of Object.entries(tagMap)) {
        nav.passageTags.set(name, tags);
      }
    }

    // Navigate to the explicit start passage, or fall back to first registered
    const target = startPassage || Object.keys(passageMap)[0];
    if (target) {
      goto(target);
    }
  }
}

export function createHarloweRuntime(): HarloweRuntime {
  return new HarloweRuntime();
}

/** Render a passage with full lifecycle. */
function renderPassage(target: string, fn: PassageFn): void {
  State.clearTemps();
  nav.currentPassage = target;
  cancelTimers();

  const doc = nav.doc;
  const story = nav.container ?? document.querySelector("tw-story");
  if (!story) return;

  // Animate out old passage (or remove immediately if no depart transition).
  departOldPassage(story as Element, nav.lastDepart, doc);
  nav.lastDepart = undefined;

  // Create <tw-passage> with tags attribute
  const passage = doc.createElement("tw-passage");
  const tags = nav.passageTags.get(target);
  if (tags && tags.length > 0) {
    passage.setAttribute("tags", tags.join(" "));
  }

  // Create <tw-sidebar> with undo/redo icons
  const sidebar = doc.createElement("tw-sidebar");
  const undoIcon = doc.createElement("tw-icon");
  undoIcon.setAttribute("tabindex", "0");
  undoIcon.setAttribute("title", "Undo");
  undoIcon.textContent = "\u21A9";
  undoIcon.addEventListener("click", () => {
    const title = State.popMoment();
    if (title) {
      const pfn = nav.passages.get(title);
      if (pfn) renderPassage(title, pfn);
    }
  });
  const redoIcon = doc.createElement("tw-icon");
  redoIcon.setAttribute("tabindex", "0");
  redoIcon.setAttribute("title", "Redo");
  redoIcon.textContent = "\u21AA";
  sidebar.appendChild(undoIcon);
  sidebar.appendChild(redoIcon);
  passage.appendChild(sidebar);

  story.appendChild(passage);

  const h = new HarloweContext(passage, doc);
  try {
    fn(h);
  } catch (e) {
    console.error(`[harlowe] error in passage "${target}":`, e);
    passage.appendChild(doc.createTextNode(`Error in passage "${target}": ${e}`));
  } finally {
    h.closeAll();
  }
  // Capture depart transition for use when navigating away from this passage.
  nav.lastDepart = h.departTransition;
}

/** Navigate to a passage by name. */
export function goto(target: string): void {
  const fn = nav.passages.get(target);
  if (!fn) {
    console.error(`[harlowe] passage not found: "${target}"`);
    return;
  }
  State.pushMoment(target);
  renderPassage(target, fn);
}

/** Include (embed) another passage inline using the provided context. */
export function display(passage: string, h: HarloweContext): void {
  const fn = nav.passages.get(passage);
  if (!fn) {
    console.error(`[harlowe] passage not found for display: "${passage}"`);
    h.text(`[passage not found: "${passage}"]`);
    return;
  }
  try {
    fn(h);
  } catch (e) {
    console.error(`[harlowe] error in displayed passage "${passage}":`, e);
    h.text(`Error in passage "${passage}": ${e}`);
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

/** Get a passage function by name. */
export function getPassage(name: string): PassageFn | undefined {
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
export function initCommands(
  registerCommand: (id: string, binding: string, handler: () => void) => void,
): void {
  registerCommand("go-back", "", () => {
    const title = State.popMoment();
    if (title) {
      const fn = nav.passages.get(title);
      if (fn) renderPassage(title, fn);
    }
  });
  registerCommand("restart", "", () => location.reload());
}
