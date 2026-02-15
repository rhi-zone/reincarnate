/** SugarCube output rendering.
 *
 * Passage code calls these functions to produce visible output. Content
 * is accumulated in a DocumentFragment buffer, then flushed to the
 * #passages container. A buffer stack supports nested content blocks
 * (link bodies, widget bodies, DOM macro targets).
 */

import { scheduleTimeout, cancelTimeout, scheduleInterval, cancelInterval } from "../platform";

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

// --- Core output functions ---

/** Emit plain text, processing SugarCube markup. */
export function text(s: string): void {
  const buf = currentBuffer();
  // Process SugarCube inline markup
  processMarkup(s, buf);
}

/** Print a value (<<print expr>>). */
export function print(v: any): void {
  const buf = currentBuffer();
  buf.appendChild(document.createTextNode(String(v)));
}

/** Emit raw HTML. */
export function html(s: string): void {
  const buf = currentBuffer();
  const temp = document.createElement("template");
  temp.innerHTML = s;
  buf.appendChild(temp.content.cloneNode(true));
}

/** Emit a line break. */
// Using a name that avoids JS reserved word conflicts in import context.
export { lineBreak as break };
function lineBreak(): void {
  currentBuffer().appendChild(document.createElement("br"));
}

// --- Links ---

/** Emit a simple link (no body content). */
export function link(text: string, passage?: string, ...setters: any[]): void {
  const buf = currentBuffer();
  const a = document.createElement("a");
  // Strip ][$ setter suffix from passage name if present (frontend parser limitation)
  const cleanPassage = passage ? passage.replace(/\]\[.*$/, "") : undefined;
  a.textContent = text;
  if (cleanPassage) {
    a.addEventListener("click", (e) => {
      e.preventDefault();
      // Dynamic import to avoid circular dependency
      import("./navigation").then((nav) => nav.goto(cleanPassage));
    });
  }
  buf.appendChild(a);
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
  const cleanPassage = passage ? passage.replace(/\]\[.*$/, "") : undefined;
  linkBlockStack.push({ variant, text, passage: cleanPassage });
  pushBuffer();
}

/** End a link block — pop buffer, wrap in link element. */
export function link_block_end(): void {
  const body = popBuffer();
  const ctx = linkBlockStack.pop();
  if (!ctx) return;

  const buf = currentBuffer();
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
  buf.appendChild(wrapper);
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

  const buf = currentBuffer();
  const container = document.createElement("span");
  container.className = "timed-content";
  container.style.display = "none";
  // Clone body content into container now
  container.appendChild(body);
  buf.appendChild(container);

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

  const buf = currentBuffer();
  const container = document.createElement("span");
  container.className = "repeat-content";
  buf.appendChild(container);

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

  const buf = currentBuffer();
  const container = document.createElement("span");
  container.className = "type-content";
  buf.appendChild(container);

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
  // Reset buffer stack
  bufferStack.length = 0;
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

/** Process SugarCube inline markup and append nodes to target. */
function processMarkup(s: string, target: Node): void {
  // Process [img[src]] and [img[src][link]] patterns
  let remaining = s;
  let lastIndex = 0;

  const imgPattern = /\[img\[([^\]]+)\](?:\[([^\]]+)\])?\]/g;
  let match;

  while ((match = imgPattern.exec(remaining)) !== null) {
    // Add text before the match
    if (match.index > lastIndex) {
      target.appendChild(
        document.createTextNode(remaining.substring(lastIndex, match.index))
      );
    }

    const src = match[1];
    const linkTarget = match[2];

    const img = document.createElement("img");
    img.src = src;

    if (linkTarget) {
      const a = document.createElement("a");
      a.href = linkTarget;
      a.appendChild(img);
      target.appendChild(a);
    } else {
      target.appendChild(img);
    }

    lastIndex = match.index + match[0].length;
  }

  // Add remaining text
  if (lastIndex < remaining.length) {
    target.appendChild(
      document.createTextNode(remaining.substring(lastIndex))
    );
  } else if (lastIndex === 0 && remaining.length > 0) {
    // No matches found — add the whole string as text
    target.appendChild(document.createTextNode(remaining));
  }
}
