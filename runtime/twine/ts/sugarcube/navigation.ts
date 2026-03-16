/** SugarCube navigation — passage registry, goto/back/return/include. */

import type { SugarCubeRuntime } from "./runtime";

/** Passage function signature: receives the runtime instance. */
export type PassageFn = (rt: SugarCubeRuntime) => void;

export class SCNavigation {
  passages: Map<string, PassageFn> = new Map();
  passageTags: Map<string, string[]> = new Map();
  currentPassage = "";

  private rt: SugarCubeRuntime;

  constructor(rt: SugarCubeRuntime) {
    this.rt = rt;
  }

  /** Run a special passage by name, if it exists. Errors are logged. */
  private runSpecial(name: string): void {
    const fn = this.passages.get(name);
    if (fn) {
      try {
        fn(this.rt);
      } catch (e) {
        console.error(`[navigation] error in ${name}:`, e);
      }
    }
  }

  /** Render a passage with full event lifecycle and special passage support. */
  renderPassage(target: string, fn: PassageFn): void {
    this.rt.State.clearTemps();
    this.currentPassage = target;
    this.rt.Output.clear();

    const tags = this.passageTags.get(target) || [];
    if (tags.includes("nobr")) {
      this.rt.Output.setNobr(true);
    }

    const passageObj = { title: target, tags };

    this.rt.Events.trigger(":passageinit", { passage: passageObj });
    this.runSpecial("PassageReady");
    this.rt.Events.trigger(":passagestart", { passage: passageObj });
    this.runSpecial("PassageHeader");

    try {
      fn(this.rt);
    } catch (e) {
      console.error(`[navigation] error in passage "${target}":`, e);
      this.rt.Output.text(`Error in passage "${target}": ${e}`);
    }

    this.runSpecial("PassageFooter");
    this.rt.Events.trigger(":passagerender", { passage: passageObj });
    this.runSpecial("PassageDone");

    this.rt.Output.setNobr(false);
    this.rt.Output.flush();

    this.rt.Events.trigger(":passageend", { passage: passageObj });
    this.rt.Events.trigger(":passagedisplay", { passage: passageObj });
  }

  /** Navigate to a passage by name. */
  goto(target: string): void {
    const fn = this.passages.get(target);
    if (!fn) {
      console.error(`[navigation] passage not found: "${target}"`);
      return;
    }
    this.rt.State.pushMoment(target);
    this.renderPassage(target, fn);
    this.rt.Platform.commitSave();
  }

  /** Go back to the previous passage, or to a named passage if provided. */
  back(passage?: string): void {
    if (passage !== undefined) {
      this.goto(passage);
      return;
    }
    const title = this.rt.State.popMoment();
    if (title === undefined) {
      console.warn("[navigation] no history to go back to");
      return;
    }
    const fn = this.passages.get(title);
    if (!fn) {
      console.error(`[navigation] passage not found on back: "${title}"`);
      return;
    }
    this.renderPassage(title, fn);
  }

  /** Return to the previous passage (alias for back). */
  return(passage?: string): void {
    this.back(passage);
  }

  /** Include (embed) another passage inline without navigation. */
  include(passage: string): void {
    const fn = this.passages.get(passage);
    if (!fn) {
      console.error(`[navigation] passage not found for include: "${passage}"`);
      return;
    }
    try {
      fn(this.rt);
    } catch (e) {
      console.error(`[navigation] error in included passage "${passage}":`, e);
      this.rt.Output.text(`Error in passage "${passage}": ${e}`);
    }
  }

  /** Get the current passage name. */
  current(): string {
    return this.currentPassage;
  }

  /** Check if a passage exists in the registry. */
  has(name: string): boolean {
    return this.passages.has(name);
  }

  /** Get a passage function by name (for widget/engine lookup). */
  getPassage(name: string): PassageFn | undefined {
    return this.passages.get(name);
  }

  /** Get the tags for a passage. */
  getTags(name: string): string[] {
    return this.passageTags.get(name) || [];
  }

  /** Get all passage names in the registry. */
  allPassages(): string[] {
    return Array.from(this.passages.keys());
  }

  /** Register commands for navigation. */
  initCommands(registerCommand: (id: string, binding: string, handler: () => void) => void): void {
    registerCommand("go-back", "", () => this.back());
    registerCommand("restart", "", () => location.reload());
  }

  // --- SugarCube stdlib functions exposed as typed SystemCall targets ---

  /**
   * Return the current passage name (`passage()` stdlib function).
   *
   * SugarCube's global `passage()` returns the title of the currently
   * displayed passage.
   */
  passage(): string {
    return this.currentPassage;
  }

  /**
   * Count the number of times any of the named passages has been visited
   * (`visited(...passages)` stdlib function).
   *
   * With no arguments: return the visit count for the current passage.
   * With one or more arguments: return the minimum visit count across all
   * named passages (0 if any of them has never been visited).
   */
  visited(...passageNames: string[]): number {
    const State = this.rt.State;
    if (passageNames.length === 0) {
      return State.visited(this.currentPassage);
    }
    return Math.min(...passageNames.map(p => State.visited(p)));
  }

  /**
   * Return the number of turns since a passage was last visited
   * (`lastVisited(...passages)` stdlib function).
   *
   * Returns -1 if the passage has never been visited.
   * With multiple arguments returns the minimum value across all named
   * passages (i.e. the most recently visited one).
   */
  lastVisited(...passageNames: string[]): number {
    const State = this.rt.State;
    const passages = passageNames.length > 0 ? passageNames : [this.currentPassage];
    const results = passages.map(p => State.sinceLastVisit(p));
    return Math.min(...results);
  }

  /**
   * Return all tags for the named passages (`tags(...passages)` stdlib function).
   *
   * With no arguments: return tags for the current passage.
   * With one or more arguments: return the union of tags for all named passages.
   */
  tags(...passageNames: string[]): string[] {
    if (passageNames.length === 0) {
      return this.passageTags.get(this.currentPassage) || [];
    }
    const result: string[] = [];
    for (const name of passageNames) {
      result.push(...(this.passageTags.get(name) || []));
    }
    return result;
  }

  /**
   * Return the number of moments in the history (`turns()` stdlib function).
   *
   * Equivalent to the number of passages visited so far (including the current one).
   */
  turns(): number {
    return this.rt.State.historyLength();
  }

  /**
   * Return the title of the most recently visited passage before the current
   * one (`previous()` stdlib function).
   *
   * Returns an empty string when there is no previous passage.
   */
  previous(): string {
    const all = this.rt.State.passages();
    return all.length >= 2 ? all[all.length - 2] : "";
  }

  /**
   * Return the number of passages whose tags include ALL of the given tags
   * (`visitedTags(...tags)` stdlib function).
   */
  visitedTags(...tagList: string[]): number {
    let count = 0;
    for (const title of this.rt.State.passages()) {
      const passageTags = this.passageTags.get(title) || [];
      if (tagList.every(t => passageTags.includes(t))) count++;
    }
    return count;
  }
}
