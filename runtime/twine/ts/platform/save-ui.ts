/** Browser save UI — save/load slot presentation. */

export interface SaveSlotInfo {
  index: number;
  title: string | null;
  date: string | null;
  isEmpty: boolean;
}

let showDialog: (title: string, content: DocumentFragment | HTMLElement) => void;
let closeDialog: () => void;

export function init(
  show: typeof showDialog,
  close: typeof closeDialog,
): void {
  showDialog = show;
  closeDialog = close;
}

export function showSaveUI(
  slots: SaveSlotInfo[],
  onSave: (i: number) => void,
  onLoad: (i: number) => void,
  onDelete: (i: number) => void,
): void {
  const frag = document.createDocumentFragment();
  const table = document.createElement("table");
  table.style.cssText = "width: 100%; border-collapse: collapse;";

  for (const slot of slots) {
    const row = document.createElement("tr");
    row.style.cssText = "border-bottom: 1px solid #333;";

    const labelCell = document.createElement("td");
    labelCell.style.cssText = "padding: 0.5em;";
    labelCell.textContent = slot.isEmpty
      ? `Slot ${slot.index + 1}: (empty)`
      : `Slot ${slot.index + 1}: ${slot.title || "Untitled"} ${slot.date ? `(${slot.date})` : ""}`;

    const actionCell = document.createElement("td");
    actionCell.style.cssText = "padding: 0.5em; text-align: right; white-space: nowrap;";

    const saveBtn = document.createElement("button");
    saveBtn.textContent = "Save";
    saveBtn.style.cssText = "margin: 0 0.2em;";
    saveBtn.addEventListener("click", () => { onSave(slot.index); closeDialog(); });

    actionCell.appendChild(saveBtn);

    if (!slot.isEmpty) {
      const loadBtn = document.createElement("button");
      loadBtn.textContent = "Load";
      loadBtn.style.cssText = "margin: 0 0.2em;";
      loadBtn.addEventListener("click", () => { onLoad(slot.index); closeDialog(); });

      const delBtn = document.createElement("button");
      delBtn.textContent = "Delete";
      delBtn.style.cssText = "margin: 0 0.2em;";
      delBtn.addEventListener("click", () => { onDelete(slot.index); closeDialog(); });

      actionCell.appendChild(loadBtn);
      actionCell.appendChild(delBtn);
    }

    row.appendChild(labelCell);
    row.appendChild(actionCell);
    table.appendChild(row);
  }

  frag.appendChild(table);
  showDialog("Saves", frag);
}

export function closeSaveUI(): void {
  closeDialog();
}
