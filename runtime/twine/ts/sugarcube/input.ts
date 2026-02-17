/** SugarCube input macros.
 *
 * Form elements that bind to story variables via State.set/get.
 * Each creates a DOM element and appends it to the current output buffer.
 */

import * as State from "./state";
import { output } from "./output";

/** Get the current output target. */
function appendToOutput(el: HTMLElement): void {
  const container = output.container ?? document.getElementById("passages");
  if (container) {
    container.appendChild(el);
  }
}

/** <<textbox "$var" "default" ["PassageName"]>> */
export function textbox(varName: string, defaultValue?: string, passageName?: string): void {
  const input = output.doc.createElement("input");
  input.type = "text";
  input.value = defaultValue ?? (State.get(varName) as string) ?? "";

  input.addEventListener("input", () => {
    State.set(varName, input.value);
  });

  if (passageName) {
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        State.set(varName, input.value);
        import("./navigation").then((nav) => nav.goto(passageName));
      }
    });
  }

  // Set initial value
  State.set(varName, input.value);
  appendToOutput(input);
}

/** <<textarea "$var" "default" ["PassageName"]>> */
export function textarea(varName: string, defaultValue?: string, passageName?: string): void {
  const el = output.doc.createElement("textarea");
  el.value = defaultValue ?? (State.get(varName) as string) ?? "";
  el.rows = 4;

  el.addEventListener("input", () => {
    State.set(varName, el.value);
  });

  State.set(varName, el.value);
  appendToOutput(el);
}

/** <<numberbox "$var" default ["PassageName"]>> */
export function numberbox(varName: string, defaultValue?: number, passageName?: string): void {
  const input = output.doc.createElement("input");
  input.type = "number";
  input.value = String(defaultValue ?? (State.get(varName) as number) ?? "");

  input.addEventListener("input", () => {
    State.set(varName, Number(input.value));
  });

  if (passageName) {
    input.addEventListener("keydown", (e) => {
      if (e.key === "Enter") {
        e.preventDefault();
        State.set(varName, Number(input.value));
        import("./navigation").then((nav) => nav.goto(passageName));
      }
    });
  }

  if (defaultValue !== undefined) {
    State.set(varName, defaultValue);
  }
  appendToOutput(input);
}

/** <<checkbox "$var" checkedValue uncheckedValue>> */
export function checkbox(varName: string, checkedValue: any, uncheckedValue: any): void {
  const input = output.doc.createElement("input");
  input.type = "checkbox";

  const current = State.get(varName);
  input.checked = current === checkedValue;

  input.addEventListener("change", () => {
    State.set(varName, input.checked ? checkedValue : uncheckedValue);
  });

  State.set(varName, input.checked ? checkedValue : uncheckedValue);
  appendToOutput(input);
}

/** <<radiobutton "$var" checkedValue>> */
export function radiobutton(varName: string, checkedValue: any): void {
  const input = output.doc.createElement("input");
  input.type = "radio";
  input.name = varName; // group by variable name

  const current = State.get(varName);
  input.checked = current === checkedValue;

  input.addEventListener("change", () => {
    if (input.checked) {
      State.set(varName, checkedValue);
    }
  });

  appendToOutput(input);
}

/** <<listbox "$var" items...>> */
export function listbox(varName: string, ...items: any[]): void {
  const select = output.doc.createElement("select");

  for (const item of items) {
    const option = output.doc.createElement("option");
    option.value = String(item);
    option.textContent = String(item);
    select.appendChild(option);
  }

  const current = State.get(varName);
  if (current !== undefined) {
    select.value = String(current);
  } else if (items.length > 0) {
    State.set(varName, items[0]);
  }

  select.addEventListener("change", () => {
    // Try to find the original typed value
    const idx = select.selectedIndex;
    State.set(varName, idx >= 0 && idx < items.length ? items[idx] : select.value);
  });

  appendToOutput(select);
}

/** <<cycle "$var" items...>> */
export function cycle(varName: string, ...items: any[]): void {
  if (items.length === 0) return;

  let currentIndex = 0;
  const current = State.get(varName);
  if (current !== undefined) {
    const idx = items.indexOf(current);
    if (idx >= 0) currentIndex = idx;
  }

  const btn = output.doc.createElement("button");
  btn.textContent = String(items[currentIndex]);
  State.set(varName, items[currentIndex]);

  btn.addEventListener("click", (e) => {
    e.preventDefault();
    currentIndex = (currentIndex + 1) % items.length;
    btn.textContent = String(items[currentIndex]);
    State.set(varName, items[currentIndex]);
  });

  appendToOutput(btn);
}

/** <<button "text" ["PassageName"]>> */
export function button(text: string, passageName?: string): void {
  const btn = output.doc.createElement("button");
  btn.textContent = text;

  if (passageName) {
    btn.addEventListener("click", (e) => {
      e.preventDefault();
      import("./navigation").then((nav) => nav.goto(passageName));
    });
  }

  appendToOutput(btn);
}
