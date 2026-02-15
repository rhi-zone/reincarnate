/** Browser timing — setTimeout/setInterval wrapper. */

export function scheduleTimeout(fn: () => void, ms: number): number {
  return window.setTimeout(fn, ms);
}

export function cancelTimeout(id: number): void {
  window.clearTimeout(id);
}

export function scheduleInterval(fn: () => void, ms: number): number {
  return window.setInterval(fn, ms);
}

export function cancelInterval(id: number): void {
  window.clearInterval(id);
}
