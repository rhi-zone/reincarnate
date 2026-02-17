/** Harlowe story variable state management.
 *
 * Two stores: storyVars ($-prefixed) persist across passages,
 * tempVars (_-prefixed) are cleared each passage transition.
 * Harlowe also tracks `it` — the result of the most recent expression.
 */

import { loadLocal, saveLocal, removeLocal, type SaveSlotInfo, showSaveUI } from "../platform";

// --- State class ---

interface Moment {
  title: string;
  variables: Record<string, any>;
}

class HarloweState {
  storyVars: Record<string, any> = {};
  tempVars: Record<string, any> = {};
  itValue: any = undefined;
  history: Moment[] = [];
  visitedSet: Set<string> = new Set();
}

export const state = new HarloweState();

// --- Variable accessors ---

/** Get a story variable by name (without the $ prefix).
 *  Uninitialized variables default to 0, matching Harlowe 2.x behavior. */
export function get(name: string): any {
  return name in state.storyVars ? state.storyVars[name] : 0;
}

/** Set a story variable by name (without the $ prefix). */
export function set(name: string, value: any): void {
  state.storyVars[name] = value;
  state.itValue = value;
}

/** Get the `it` keyword value (result of most recent expression). */
export function get_it(): any {
  return state.itValue;
}

/** Clear all temp variables (called at start of each passage). */
export function clearTemps(): void {
  for (const key of Object.keys(state.tempVars)) {
    delete state.tempVars[key];
  }
}

// --- History ---

/** Deep-clone story variables and push onto history. */
export function pushMoment(title: string): void {
  state.visitedSet.add(title);
  state.history.push({
    title,
    variables: JSON.parse(JSON.stringify(state.storyVars)),
  });
}

/** Pop the most recent moment and restore story variables. */
export function popMoment(): string | undefined {
  state.history.pop();
  const prev = state.history[state.history.length - 1];
  if (!prev) return undefined;
  for (const key of Object.keys(state.storyVars)) {
    delete state.storyVars[key];
  }
  Object.assign(state.storyVars, JSON.parse(JSON.stringify(prev.variables)));
  return prev.title;
}

/** Get the number of moments in history. */
export function historyLength(): number {
  return state.history.length;
}

/** Check if a passage has ever been visited. */
export function hasVisited(title: string): boolean {
  return state.visitedSet.has(title);
}

/** Count how many times a passage appears in the history. */
export function visits(title: string): number {
  let count = 0;
  for (const moment of state.history) {
    if (moment.title === title) count++;
  }
  return count;
}

/** Get the current passage title. */
export function currentPassage(): string | undefined {
  const top = state.history[state.history.length - 1];
  return top?.title;
}

/** Get all passage titles from history. */
export function historyTitles(): string[] {
  return state.history.map(m => m.title);
}

/** Forget the n most recent undos. -1 forgets all. */
export function forgetUndos(n: number): void {
  if (n < 0) {
    // Keep only the current moment
    if (state.history.length > 1) {
      const current = state.history[state.history.length - 1];
      state.history.length = 0;
      state.history.push(current);
    }
  } else {
    // Remove n most recent moments (keeping at least the current one)
    const keep = Math.max(1, state.history.length - n);
    state.history.splice(0, state.history.length - keep);
  }
}

/** Clear visit history. */
export function forgetVisits(): void {
  state.visitedSet.clear();
}

// --- Persistence ---

const SLOT_PREFIX = "reincarnate-harlowe-save-";

/** Save current state to a named slot. */
export function saveSlot(name: string): boolean {
  try {
    const data = JSON.stringify({
      history: state.history,
      variables: state.storyVars,
    });
    saveLocal(SLOT_PREFIX + name, data);
    return true;
  } catch {
    return false;
  }
}

/** Load state from a named slot. Returns passage title or undefined. */
export function loadSlot(name: string): string | undefined {
  const raw = loadLocal(SLOT_PREFIX + name);
  if (raw === null) return undefined;
  const data = JSON.parse(raw);
  state.history.length = 0;
  for (const moment of data.history) {
    state.history.push(moment);
  }
  for (const key of Object.keys(state.storyVars)) {
    delete state.storyVars[key];
  }
  Object.assign(state.storyVars, data.variables);
  const top = state.history[state.history.length - 1];
  return top?.title;
}

/** Delete a save slot. */
export function deleteSlot(name: string): void {
  removeLocal(SLOT_PREFIX + name);
}

/** Check if a save slot exists. */
export function hasSlot(name: string): boolean {
  return loadLocal(SLOT_PREFIX + name) !== null;
}

const SLOT_COUNT = 8;

/** Register save/load commands. */
export function initCommands(
  registerCommand: (id: string, binding: string, handler: () => void) => void,
  goto: (passage: string) => void,
): void {
  registerCommand("quicksave", "$mod+s", () => saveSlot("auto"));
  registerCommand("quickload", "", () => {
    const title = loadSlot("auto");
    if (title) goto(title);
  });
  for (let i = 0; i < SLOT_COUNT; i++) {
    const slot = i;
    registerCommand(`save-to-slot-${slot + 1}`, "", () => saveSlot(String(slot)));
    registerCommand(`load-from-slot-${slot + 1}`, "", () => {
      const title = loadSlot(String(slot));
      if (title) goto(title);
    });
  }
  registerCommand("open-saves", "", () => {
    const slots: SaveSlotInfo[] = [];
    for (let i = 0; i < SLOT_COUNT; i++) {
      const has = hasSlot(String(i));
      slots.push({ index: i, title: has ? `Save ${i + 1}` : null, date: null, isEmpty: !has });
    }
    showSaveUI(
      slots,
      (i) => saveSlot(String(i)),
      (i) => { const t = loadSlot(String(i)); if (t) goto(t); },
      (i) => deleteSlot(String(i)),
    );
  });
}
