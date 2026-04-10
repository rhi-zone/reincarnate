// HANDWRITTEN: This file is a temporary implementation placeholder. All exports
// will be replaced by code generated from IR bodies once implemented. Do not
// add new functionality here — implement it in the appropriate runtime_bodies.rs
// (or equivalent source-engine registration file) instead.

/** Browser dialog — modal dialogue boxes. */

import { OverlayManager } from "./_overlay";

// HANDWRITTEN
export class DialogManager {
  private overlay: OverlayManager;

  constructor(overlay: OverlayManager) {
    this.overlay = overlay;
  }

  showDialog(title: string, content: DocumentFragment | HTMLElement): void {
    this.overlay.buildDialogChrome(title, content, () => this.closeDialog());
  }

  closeDialog(): void {
    this.overlay.hideOverlay();
  }

  isDialogOpen(): boolean {
    return this.overlay.isOverlayVisible();
  }

  initCommands(register: (id: string, binding: string, handler: () => void) => void): void {
    register("close-dialog", "escape", () => this.closeDialog());
  }
}
