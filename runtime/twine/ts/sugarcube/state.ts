/** SugarCube story variable state management.
 *
 * Two stores: storyVars ($-prefixed) persist across passages,
 * tempVars (_-prefixed) are cleared each passage transition.
 * The moment system tracks history for back/return navigation.
 */

import { type SaveableState, type HistoryStrategy, snapshotHistory } from "../platform";

export class SCState implements SaveableState {
  /** Story ($) variables — exposed for `State.variables` global alias. */
  readonly storyVars: Record<string, unknown> = {};
  /** Temporary (_) variables — exposed for `State.temporary` global alias. */
  readonly tempVars: Record<string, unknown> = {};
  private history: HistoryStrategy;

  constructor(history?: HistoryStrategy) {
    this.history = history ?? snapshotHistory();
  }

  /** Get a story or temp variable. */
  get(name: string): unknown {
    if (name.startsWith("_")) {
      return this.tempVars[name];
    }
    return this.storyVars[name];
  }

  /** Set a story or temp variable. */
  set(name: string, value: unknown): void {
    if (name.startsWith("_")) {
      this.tempVars[name] = value;
    } else {
      this.storyVars[name] = value;
    }
  }

  /** Delete a story or temp variable. */
  unset(name: string): void {
    if (name.startsWith("_")) {
      delete this.tempVars[name];
    } else {
      delete this.storyVars[name];
    }
  }

  /** Clear all temp variables (called at start of each passage). */
  clearTemps(): void {
    for (const key of Object.keys(this.tempVars)) {
      delete this.tempVars[key];
    }
  }

  // --- History (delegated to strategy) ---

  pushMoment(title: string): void {
    this.history.push(title, this.storyVars);
  }

  popMoment(): string | undefined {
    const restored = this.history.pop();
    if (!restored) return undefined;
    for (const key of Object.keys(this.storyVars)) {
      delete this.storyVars[key];
    }
    Object.assign(this.storyVars, restored.vars);
    return restored.title;
  }

  peekMoment(): string | undefined {
    return this.history.peek();
  }

  historyLength(): number {
    return this.history.length;
  }

  hasPlayed(title: string): boolean {
    return this.history.hasVisited(title);
  }

  visited(title: string): number {
    return this.history.countVisits(title);
  }

  /**
   * Return the number of turns since `title` was last visited.
   *
   * Returns -1 if the passage has never been visited.  The current passage
   * has a distance of 0 (it is the last entry in history).
   */
  sinceLastVisit(title: string): number {
    const all = this.history.titles();
    for (let i = all.length - 1; i >= 0; i--) {
      if (all[i] === title) {
        return all.length - 1 - i;
      }
    }
    return -1;
  }

  passages(): string[] {
    return this.history.titles();
  }

  // --- SaveableState implementation ---

  serialize(): string {
    return JSON.stringify({
      title: this.history.peek(),
      variables: this.storyVars,
    });
  }

  deserialize(data: string): string | undefined {
    const parsed = JSON.parse(data);
    for (const key of Object.keys(this.storyVars)) {
      delete this.storyVars[key];
    }
    Object.assign(this.storyVars, parsed.variables);
    // Reset history — a loaded save starts a fresh history from the restored passage
    this.history.forgetUndos(-1);
    if (parsed.title) {
      this.pushMoment(parsed.title);
    }
    return parsed.title;
  }
}
