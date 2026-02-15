/** SugarCube event bus.
 *
 * Handles SugarCube-specific jQuery events like :passageinit, :passageend,
 * :passagestart, :passagerender, :passagedisplay, :storyready.
 *
 * These are routed here from the jQuery shim when $(document).on(":foo", fn)
 * is called, and triggered from navigation.ts at the appropriate points.
 */

interface HandlerEntry {
  fn: Function;
  once: boolean;
}

const handlers: Map<string, HandlerEntry[]> = new Map();

/** Register a handler for an event. */
export function on(event: string, fn: Function): void {
  if (!handlers.has(event)) handlers.set(event, []);
  handlers.get(event)!.push({ fn, once: false });
}

/** Register a one-shot handler for an event. */
export function one(event: string, fn: Function): void {
  if (!handlers.has(event)) handlers.set(event, []);
  handlers.get(event)!.push({ fn, once: true });
}

/** Remove a specific handler for an event. */
export function off(event: string, fn: Function): void {
  const list = handlers.get(event);
  if (!list) return;
  const idx = list.findIndex(h => h.fn === fn);
  if (idx >= 0) list.splice(idx, 1);
}

/** Trigger an event, calling all registered handlers. */
export function trigger(event: string, ...args: any[]): void {
  const list = handlers.get(event);
  if (!list) return;
  // Copy so handlers can modify the list (one-shot removal)
  const snapshot = [...list];
  for (const entry of snapshot) {
    try {
      entry.fn(...args);
    } catch (e) {
      console.error(`[events] error in handler for "${event}":`, e);
    }
    if (entry.once) {
      const idx = list.indexOf(entry);
      if (idx >= 0) list.splice(idx, 1);
    }
  }
}
