/** SugarCube output rendering.
 *
 * Passage code calls these functions to produce visible output. Content
 * is accumulated in a DocumentFragment buffer, then flushed to the
 * #passages container. A buffer stack supports nested content blocks
 * (link bodies, widget bodies, DOM macro targets). An element stack
 * tracks open HTML elements for proper nesting of structured HTML nodes.
 */

import { scheduleTimeout, cancelTimeout, scheduleInterval, cancelInterval } from "../platform";

// --- nobr flag ---

let nobrActive = false;

/** Set the nobr (no line break) flag. When active, break() is suppressed. */
export function setNobr(active: boolean): void {
  nobrActive = active;
}

// --- Buffer stack ---

const bufferStack: DocumentFragment[] = [];

function currentBuffer(): DocumentFragment {
  if (bufferStack.length === 0) {
    bufferStack.push(document.createDocumentFragment());
  }
  return bufferStack[bufferStack.length - 1];
}

/** Push a new output buffer for nested content. */
export function pushBuffer(): DocumentFragment {
  const frag = document.createDocumentFragment();
  bufferStack.push(frag);
  return frag;
}

/** Pop and return the top buffer. */
export function popBuffer(): DocumentFragment {
  return bufferStack.pop() || document.createDocumentFragment();
}

// --- Element stack (for structured HTML nesting) ---

const elementStack: HTMLElement[] = [];

/** Append a node to the current parent (top of element stack, or buffer). */
function appendNode(node: Node): void {
  if (elementStack.length > 0) {
    elementStack[elementStack.length - 1].appendChild(node);
  } else {
    currentBuffer().appendChild(node);
  }
}

// --- Core output functions ---

/** Emit plain text. */
export function text(s: string): void {
  appendNode(document.createTextNode(s));
}

/** Print a value (<<print expr>>). */
export function print(v: any): void {
  appendNode(document.createTextNode(String(v)));
}

// --- Structured HTML element functions ---

/** Open an HTML element, push onto element stack. */
export function open_element(tag: string, ...attrs: string[]): void {
  const el = document.createElement(tag);
  for (let i = 0; i < attrs.length; i += 2) {
    el.setAttribute(attrs[i], attrs[i + 1]);
  }
  appendNode(el);
  elementStack.push(el);
}

/** Close the current open element (pop from element stack). */
export function close_element(): void {
  elementStack.pop();
}

/** Emit a void/self-closing HTML element (no push). */
export function void_element(tag: string, ...attrs: string[]): void {
  const el = document.createElement(tag);
  for (let i = 0; i < attrs.length; i += 2) {
    el.setAttribute(attrs[i], attrs[i + 1]);
  }
  appendNode(el);
}

/** Set a dynamic attribute on the most recently opened/emitted element. */
export function set_attribute(name: string, value: any): void {
  // Find the last element: top of elementStack, or last child of buffer
  let el: Element | null = null;
  if (elementStack.length > 0) {
    el = elementStack[elementStack.length - 1];
  } else {
    const buf = currentBuffer();
    el = buf.lastElementChild;
  }
  if (el) {
    el.setAttribute(name, String(value));
  }
}

/** Emit a line break. Suppressed when nobr tag is active. */
// Using a name that avoids JS reserved word conflicts in import context.
export { lineBreak as break };
function lineBreak(): void {
  if (nobrActive) return;
  appendNode(document.createElement("br"));
}

// --- Links ---

/** Emit a simple link (no body content). */
export function link(text: string, passage?: string, setter?: () => void): void {
  const a = document.createElement("a");
  a.textContent = text;
  if (passage || setter) {
    a.addEventListener("click", (e) => {
      e.preventDefault();
      if (setter) setter();
      if (passage) {
        import("./navigation").then((nav) => nav.goto(passage));
      }
    });
  }
  appendNode(a);
}

// --- Images ---

/** Emit an inline image, optionally wrapped in a link. */
export function image(src: string, link?: string): void {
  const img = document.createElement("img");
  img.src = src;
  if (link) {
    const a = document.createElement("a");
    a.href = link;
    a.appendChild(img);
    appendNode(a);
  } else {
    appendNode(img);
  }
}

// --- Link blocks (<<link>> with body) ---

interface LinkBlockContext {
  variant: string;
  text: string;
  passage?: string;
}

const linkBlockStack: LinkBlockContext[] = [];

/** Start a link block — push buffer for body content. */
export function link_block_start(variant: string, text: string, passage?: string): void {
  linkBlockStack.push({ variant, text, passage });
  pushBuffer();
}

/** End a link block — pop buffer, wrap in link element. */
export function link_block_end(): void {
  const body = popBuffer();
  const ctx = linkBlockStack.pop();
  if (!ctx) return;

  const wrapper = document.createElement("span");
  wrapper.className = "link-block";

  const a = document.createElement("a");
  a.textContent = ctx.text;
  a.addEventListener("click", (e) => {
    e.preventDefault();
    // Execute body content
    const parent = a.parentElement;
    if (parent) {
      if (ctx.variant === "linkreplace") {
        // Replace the link with the body content
        while (parent.firstChild) parent.removeChild(parent.firstChild);
      } else if (ctx.variant === "linkprepend") {
        parent.insertBefore(body.cloneNode(true), a);
      } else {
        // linkappend or default link
        parent.appendChild(body.cloneNode(true));
      }
      if (ctx.variant !== "link") {
        a.remove();
      }
    }
    // Navigate if passage specified
    if (ctx.passage) {
      import("./navigation").then((nav) => nav.goto(ctx.passage!));
    }
  });

  wrapper.appendChild(a);
  appendNode(wrapper);
}

// --- Timed/Repeat/Type blocks ---

interface TimedContext {
  delay: number;
  transition?: string;
}

const timedStack: TimedContext[] = [];
const activeTimers: number[] = [];

/** Start a timed block. */
export function timed_start(delay: string | number, transition?: string): void {
  const ms = parseDelay(delay);
  timedStack.push({ delay: ms, transition });
  pushBuffer();
}

/** End a timed block — schedule content to appear after delay. */
export function timed_end(): void {
  const body = popBuffer();
  const ctx = timedStack.pop();
  if (!ctx) return;

  const container = document.createElement("span");
  container.className = "timed-content";
  container.style.display = "none";
  // Clone body content into container now
  container.appendChild(body);
  appendNode(container);

  const id = scheduleTimeout(() => {
    container.style.display = "";
  }, ctx.delay);
  activeTimers.push(id);
}

interface RepeatContext {
  interval: number;
  transition?: string;
}

const repeatStack: RepeatContext[] = [];

/** Start a repeat block. */
export function repeat_start(interval: string | number, transition?: string): void {
  const ms = parseDelay(interval);
  repeatStack.push({ interval: ms, transition });
  pushBuffer();
}

/** End a repeat block — append content at interval. */
export function repeat_end(): void {
  const body = popBuffer();
  const ctx = repeatStack.pop();
  if (!ctx) return;

  const container = document.createElement("span");
  container.className = "repeat-content";
  appendNode(container);

  const id = scheduleInterval(() => {
    container.appendChild(body.cloneNode(true));
  }, ctx.interval);
  activeTimers.push(id);
}

interface TypeContext {
  speed: number;
}

const typeStack: TypeContext[] = [];

/** Start a type (typewriter) block. */
export function type_start(speed: string | number): void {
  const ms = parseDelay(speed);
  typeStack.push({ speed: ms });
  pushBuffer();
}

/** End a type block — reveal characters one at a time. */
export function type_end(): void {
  const body = popBuffer();
  const ctx = typeStack.pop();
  if (!ctx) return;

  const container = document.createElement("span");
  container.className = "type-content";
  appendNode(container);

  // Collect all text content
  const fullText = body.textContent || "";
  let charIndex = 0;

  const id = scheduleInterval(() => {
    if (charIndex < fullText.length) {
      container.textContent = fullText.substring(0, charIndex + 1);
      charIndex++;
    } else {
      // Done typing — replace with full body content
      while (container.firstChild) container.removeChild(container.firstChild);
      container.appendChild(body);
      cancelInterval(id);
    }
  }, ctx.speed);
  activeTimers.push(id);
}

// --- Flush/Clear ---

/** Flush the output buffer to #passages. */
export function flush(): void {
  const container = document.getElementById("passages");
  if (!container) return;
  while (bufferStack.length > 0) {
    const buf = bufferStack.shift()!;
    container.appendChild(buf);
  }
}

/** Clear the #passages container and cancel any active timers. */
export function clear(): void {
  const container = document.getElementById("passages");
  if (container) {
    while (container.firstChild) {
      container.removeChild(container.firstChild);
    }
  }
  // Cancel active timers from previous passage
  for (const id of activeTimers) {
    cancelTimeout(id);
    cancelInterval(id);
  }
  activeTimers.length = 0;
  // Reset buffer and element stacks
  bufferStack.length = 0;
  elementStack.length = 0;
}

// --- Helpers ---

/** Parse a delay value (number or string like "2s", "500ms"). */
function parseDelay(value: string | number): number {
  if (typeof value === "number") return value;
  const s = value.trim().toLowerCase();
  if (s.endsWith("ms")) return parseFloat(s);
  if (s.endsWith("s")) return parseFloat(s) * 1000;
  return parseFloat(s) || 0;
}
