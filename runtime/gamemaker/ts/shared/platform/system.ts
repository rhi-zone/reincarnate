// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** System queries — locale, network, clipboard. */

/** Browser locale language code (e.g. "en", "fr", "de"). */
// HANDWRITTEN
export function systemLanguage(): string {
  return (navigator.language ?? "en").split("-")[0]!.toLowerCase();
}

/** Whether the browser believes a network connection is available. */
// HANDWRITTEN
export function isNetworkConnected(): boolean {
  return navigator.onLine;
}

/**
 * Write text to the system clipboard asynchronously.
 * No-op if the Clipboard API is unavailable (e.g. non-secure context).
 */
// HANDWRITTEN
export function writeClipboard(text: string): void {
  navigator.clipboard?.writeText(text);
}
