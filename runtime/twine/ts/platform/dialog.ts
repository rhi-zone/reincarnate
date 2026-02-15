/** Browser dialog — modal dialogue boxes. */

import { buildDialogChrome, hideOverlay, isOverlayVisible } from "./_overlay";
import { registerCommand } from "./input";

export function showDialog(title: string, content: DocumentFragment | HTMLElement): void {
  buildDialogChrome(title, content, closeDialog);
}

export function closeDialog(): void {
  hideOverlay();
}

export function isDialogOpen(): boolean {
  return isOverlayVisible();
}

registerCommand("close-dialog", "escape", closeDialog);
