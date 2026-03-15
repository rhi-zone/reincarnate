/** Window and display platform — size, fullscreen, URL launch. */

/** Visible viewport width in CSS pixels. */
export function displayWidth(): number {
  return window.innerWidth;
}

/** Visible viewport height in CSS pixels. */
export function displayHeight(): number {
  return window.innerHeight;
}

/** Open a URL in a new browser tab. */
export function openUrl(url: string): void {
  window.open(url, "_blank");
}

/** Attempt to close the browser window/tab. May be a no-op in most browsers. */
export function closeWindow(): void {
  try { window.close(); } catch { /* not permitted in most browsers */ }
}

/** Request the browser to enter fullscreen mode. No-op if unsupported. */
export function requestFullscreen(): void {
  document.documentElement.requestFullscreen?.();
}

/** Exit fullscreen mode. No-op if not currently fullscreen. */
export function exitFullscreen(): void {
  document.exitFullscreen?.();
}

/**
 * Trigger a browser file download from a data URL.
 * No-op if the Anchor/click API is unavailable (e.g. non-browser context).
 */
export function downloadDataUrl(dataUrl: string, filename: string): void {
  try {
    const a = document.createElement("a");
    a.href = dataUrl;
    a.download = filename;
    a.click();
  } catch { /* ignore: may fail in non-browser environments */ }
}
