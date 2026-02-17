/** Browser input — keybinds-backed command registry with UI components. */

import keybindsInit, { type Command, registerComponents, executeCommand } from "keybinds";

// Register web components (<command-palette>, <keybind-cheatsheet>, etc.)
registerComponents();

class InputManager {
  commands: Command[] = [];
  cleanup: (() => void) | null = null;
  palette: HTMLElement | null = null;
  cheatsheet: HTMLElement | null = null;
}

const input = new InputManager();

function ensureUI(): void {
  if (input.palette) return;
  input.palette = document.createElement("command-palette");
  input.palette.setAttribute("auto-trigger", "");
  document.body.appendChild(input.palette);

  input.cheatsheet = document.createElement("keybind-cheatsheet");
  input.cheatsheet.setAttribute("auto-trigger", "");
  document.body.appendChild(input.cheatsheet);
}

function syncUI(): void {
  if (input.palette) (input.palette as any).commands = input.commands;
  if (input.cheatsheet) (input.cheatsheet as any).commands = input.commands;
}

function rebind(): void {
  if (input.cleanup) input.cleanup();
  if (input.commands.length > 0) {
    input.cleanup = keybindsInit(input.commands);
  }
  syncUI();
}

export function registerCommand(
  id: string,
  defaultBinding: string,
  handler: () => void,
): void {
  ensureUI();
  input.commands = input.commands.filter(c => c.id !== id);
  input.commands.push({
    id,
    label: id.replace(/-/g, " ").replace(/\b\w/g, c => c.toUpperCase()),
    keys: defaultBinding ? [defaultBinding] : [],
    execute: handler,
  });
  rebind();
}

export function removeCommand(id: string): void {
  input.commands = input.commands.filter(c => c.id !== id);
  rebind();
}

export function triggerCommand(id: string): void {
  executeCommand(input.commands, id);
}

export function getCommands(): Command[] {
  return input.commands;
}
