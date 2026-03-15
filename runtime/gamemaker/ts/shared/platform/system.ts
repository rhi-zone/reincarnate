/** System queries — locale, network, clipboard. */

/** Browser locale language code (e.g. "en", "fr", "de"). */
export function systemLanguage(): string {
  return (navigator.language ?? "en").split("-")[0]!.toLowerCase();
}

/** Whether the browser believes a network connection is available. */
export function isNetworkConnected(): boolean {
  return navigator.onLine;
}

/**
 * Write text to the system clipboard asynchronously.
 * No-op if the Clipboard API is unavailable (e.g. non-secure context).
 */
export function writeClipboard(text: string): void {
  navigator.clipboard?.writeText(text);
}
