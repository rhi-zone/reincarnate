/** Browser timing — frame scheduling via setTimeout. */

export function scheduleFrame(cb: () => void, delay: number): number {
  return window.setTimeout(cb, delay) as unknown as number;
}

export function cancelFrame(handle: number): void {
  clearTimeout(handle);
}
