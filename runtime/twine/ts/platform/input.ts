/** Browser input — command registry with keyboard bindings. */

interface CommandEntry {
  binding: string;
  handler: () => void;
}

const commands = new Map<string, CommandEntry>();

document.addEventListener("keydown", (e) => {
  for (const [, entry] of commands) {
    if (matchBinding(entry.binding, e)) {
      e.preventDefault();
      entry.handler();
      return;
    }
  }
});

function matchBinding(binding: string, e: KeyboardEvent): boolean {
  const parts = binding.toLowerCase().split("+");
  const key = parts.pop()!;
  const needCtrl = parts.includes("ctrl");
  const needShift = parts.includes("shift");
  const needAlt = parts.includes("alt");

  if (needCtrl !== e.ctrlKey) return false;
  if (needShift !== e.shiftKey) return false;
  if (needAlt !== e.altKey) return false;

  return e.key.toLowerCase() === key || e.code.toLowerCase() === key;
}

export function registerCommand(
  id: string,
  defaultBinding: string,
  handler: () => void,
): void {
  commands.set(id, { binding: defaultBinding, handler });
}

export function removeCommand(id: string): void {
  commands.delete(id);
}

export function triggerCommand(id: string): void {
  const entry = commands.get(id);
  if (entry) entry.handler();
}
