// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Window and display platform — size, fullscreen, URL launch. */

/** Visible viewport width in CSS pixels. */
// HANDWRITTEN
export function displayWidth(): number {
  return window.innerWidth;
}

/** Visible viewport height in CSS pixels. */
// HANDWRITTEN
export function displayHeight(): number {
  return window.innerHeight;
}

/** Open a URL in a new browser tab. */
// HANDWRITTEN
export function openUrl(url: string): void {
  window.open(url, "_blank");
}

/** Attempt to close the browser window/tab. May be a no-op in most browsers. */
// HANDWRITTEN
export function closeWindow(): void {
  try { window.close(); } catch { /* not permitted in most browsers */ }
}

/** Request the browser to enter fullscreen mode. No-op if unsupported. */
// HANDWRITTEN
export function requestFullscreen(): void {
  document.documentElement.requestFullscreen?.();
}

/** Exit fullscreen mode. No-op if not currently fullscreen. */
// HANDWRITTEN
export function exitFullscreen(): void {
  document.exitFullscreen?.();
}

/** Set the browser tab/window title. */
// HANDWRITTEN
export function setWindowTitle(title: string): void {
  document.title = title;
}

/** Whether the document currently has keyboard/input focus. */
// HANDWRITTEN
export function windowHasFocus(): boolean {
  return document.hasFocus();
}

/**
 * Trigger a browser file download from a data URL.
 * No-op if the Anchor/click API is unavailable (e.g. non-browser context).
 */
// HANDWRITTEN
export function downloadDataUrl(dataUrl: string, filename: string): void {
  try {
    const a = document.createElement("a");
    a.href = dataUrl;
    a.download = filename;
    a.click();
  } catch { /* ignore: may fail in non-browser environments */ }
}
