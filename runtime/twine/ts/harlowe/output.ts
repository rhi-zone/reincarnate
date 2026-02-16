/** Harlowe output rendering — declarative content tree.
 *
 * Passage functions return ContentNode[] which the renderer walks
 * depth-first to produce DOM nodes. Strings are text nodes. Builder
 * functions produce ContentElement objects. The navigation module
 * calls render(container, nodes) on passage transition.
 */

import { scheduleInterval, cancelInterval } from "../platform";

// --- Changer type (shared with engine.ts) ---

export interface Changer {
  name: string;
  args: any[];
}

// --- Content tree types ---

export type ContentNode = string | ContentElement | ContentNode[];

export type ContentElement =
  | { t: "el"; tag: string; attrs: Record<string, string>; c: ContentNode[] }
  | { t: "styled"; changer: Changer | Changer[]; c: ContentNode[] }
  | { t: "link"; text: string; passage: string }
  | { t: "link_cb"; text: string; cb: () => ContentNode[] }
  | { t: "live"; interval: number; cb: () => ContentNode[] }
  | { t: "void"; tag: string; attrs: Record<string, string> }
  | { t: "print"; v: any }
  | { t: "display"; passage: string };

// --- Changer builder helpers ---

function changerNode(name: string, value: any, c: ContentNode[]): ContentElement {
  return { t: "styled", changer: { name, args: [value] }, c };
}

// --- Content builders (imported by generated code via function_modules) ---

export function color(v: string, c: ContentNode[]): ContentElement {
  return changerNode("color", v, c);
}

export function background(v: string, c: ContentNode[]): ContentElement {
  return changerNode("background", v, c);
}

export function textStyle(v: string, c: ContentNode[]): ContentElement {
  return changerNode("text-style", v, c);
}

export function font(v: string, c: ContentNode[]): ContentElement {
  return changerNode("font", v, c);
}

export function align(v: string, c: ContentNode[]): ContentElement {
  return changerNode("align", v, c);
}

export function opacity(v: number, c: ContentNode[]): ContentElement {
  return changerNode("opacity", v, c);
}

export function css(v: string, c: ContentNode[]): ContentElement {
  return changerNode("css", v, c);
}

export function transition(v: string, c: ContentNode[]): ContentElement {
  return changerNode("transition", v, c);
}

export function transitionTime(v: number, c: ContentNode[]): ContentElement {
  return changerNode("transition-time", v, c);
}

export function hidden(c: ContentNode[]): ContentElement {
  return changerNode("hidden", true, c);
}

export function textSize(v: string, c: ContentNode[]): ContentElement {
  return changerNode("text-size", v, c);
}

export function textRotateZ(v: number, c: ContentNode[]): ContentElement {
  return changerNode("text-rotate-z", v, c);
}

export function collapse(c: ContentNode[]): ContentElement {
  return changerNode("collapse", true, c);
}

export function nobr(c: ContentNode[]): ContentElement {
  return changerNode("nobr", true, c);
}

export function hoverStyle(v: any, c: ContentNode[]): ContentElement {
  return changerNode("hover-style", v, c);
}

/** Variable/composed changer applied to content. */
export function styled(changer: Changer | Changer[], c: ContentNode[]): ContentElement {
  return { t: "styled", changer, c };
}

// --- Element builders ---

export function el(tag: string, c: ContentNode[], ...attrs: string[]): ContentElement {
  const a: Record<string, string> = {};
  for (let i = 0; i < attrs.length; i += 2) a[attrs[i]] = attrs[i + 1];
  return { t: "el", tag, attrs: a, c };
}

export function strong(c: ContentNode[]): ContentElement {
  return { t: "el", tag: "strong", attrs: {}, c };
}

export function em(c: ContentNode[]): ContentElement {
  return { t: "el", tag: "em", attrs: {}, c };
}

export function del(c: ContentNode[]): ContentElement {
  return { t: "el", tag: "del", attrs: {}, c };
}

export function sup(c: ContentNode[]): ContentElement {
  return { t: "el", tag: "sup", attrs: {}, c };
}

export function sub(c: ContentNode[]): ContentElement {
  return { t: "el", tag: "sub", attrs: {}, c };
}

// --- Void element builders ---

export function br(): ContentElement {
  return { t: "void", tag: "br", attrs: {} };
}

export function hr(): ContentElement {
  return { t: "void", tag: "hr", attrs: {} };
}

export function img(src: string): ContentElement {
  return { t: "void", tag: "img", attrs: { src } };
}

export function voidEl(tag: string, ...attrs: string[]): ContentElement {
  const a: Record<string, string> = {};
  for (let i = 0; i < attrs.length; i += 2) a[attrs[i]] = attrs[i + 1];
  return { t: "void", tag, attrs: a };
}

// --- Interactive builders ---

export function link(text: string, passage: string): ContentElement {
  return { t: "link", text, passage };
}

export function linkCb(text: string, cb: () => ContentNode[]): ContentElement {
  return { t: "link_cb", text, cb };
}

export function live(interval: number, cb: () => ContentNode[]): ContentElement {
  return { t: "live", interval, cb };
}

// --- Value builders ---

export function printVal(v: any): ContentElement {
  return { t: "print", v };
}

export function displayPassage(name: string): ContentElement {
  return { t: "display", passage: name };
}

// --- Color resolution ---

const HARLOWE_COLORS: Record<string, [number, number, number]> = {
  red: [0xe6, 0x19, 0x19],
  orange: [0xe6, 0x80, 0x19],
  yellow: [0xe5, 0xe6, 0x19],
  lime: [0x80, 0xe6, 0x19],
  green: [0x19, 0xe6, 0x19],
  aqua: [0x19, 0xe5, 0xe6],
  cyan: [0x19, 0xe5, 0xe6],
  blue: [0x19, 0x7f, 0xe6],
  navy: [0x19, 0x19, 0xe6],
  purple: [0x7f, 0x19, 0xe6],
  magenta: [0xe6, 0x19, 0xe5],
  fuchsia: [0xe6, 0x19, 0xe5],
  white: [0xff, 0xff, 0xff],
  black: [0x00, 0x00, 0x00],
  grey: [0x88, 0x88, 0x88],
  gray: [0x88, 0x88, 0x88],
};

function blendColors(a: [number, number, number], b: [number, number, number]): [number, number, number] {
  return [
    Math.min(Math.round((a[0] + b[0]) * 0.6), 255),
    Math.min(Math.round((a[1] + b[1]) * 0.6), 255),
    Math.min(Math.round((a[2] + b[2]) * 0.6), 255),
  ];
}

function resolveColor(value: string): string {
  const s = value.trim().toLowerCase();
  if (s === "transparent") return "transparent";
  if (!s.includes("+")) {
    const rgb = HARLOWE_COLORS[s];
    if (rgb) return `rgb(${rgb[0]}, ${rgb[1]}, ${rgb[2]})`;
    return value;
  }
  const parts = s.split("+");
  const first = HARLOWE_COLORS[parts[0].trim()];
  if (!first) return value;
  let acc: [number, number, number] = first;
  for (let i = 1; i < parts.length; i++) {
    const rgb = HARLOWE_COLORS[parts[i].trim()];
    if (!rgb) return value;
    acc = blendColors(acc, rgb);
  }
  return `rgb(${acc[0]}, ${acc[1]}, ${acc[2]})`;
}

// --- Transition animation support ---

let transitionStylesInjected = false;

function injectTransitionStyles(): void {
  if (transitionStylesInjected) return;
  transitionStylesInjected = true;
  const style = document.createElement("style");
  style.textContent = `
@keyframes tw-dissolve { from { opacity: 0; } to { opacity: 1; } }
@keyframes tw-slide-left { from { transform: translateX(-100%); } to { transform: translateX(0); } }
@keyframes tw-slide-right { from { transform: translateX(100%); } to { transform: translateX(0); } }
@keyframes tw-slide-up { from { transform: translateY(-100%); } to { transform: translateY(0); } }
@keyframes tw-slide-down { from { transform: translateY(100%); } to { transform: translateY(0); } }
@keyframes tw-fade-left { from { opacity: 0; transform: translateX(-50%); } to { opacity: 1; transform: translateX(0); } }
@keyframes tw-fade-right { from { opacity: 0; transform: translateX(50%); } to { opacity: 1; transform: translateX(0); } }
@keyframes tw-fade-up { from { opacity: 0; transform: translateY(-50%); } to { opacity: 1; transform: translateY(0); } }
@keyframes tw-fade-down { from { opacity: 0; transform: translateY(50%); } to { opacity: 1; transform: translateY(0); } }
@keyframes tw-zoom { from { transform: scale(0); } to { transform: scale(1); } }
@keyframes tw-blur { from { filter: blur(10px); opacity: 0; } to { filter: blur(0); opacity: 1; } }
@keyframes tw-flicker { 0% { opacity: 0; } 5% { opacity: 1; } 10% { opacity: 0; } 15% { opacity: 1; } 20% { opacity: 0; } 30% { opacity: 1; } 100% { opacity: 1; } }
@keyframes tw-shudder { 0% { transform: translateX(-3px); } 25% { transform: translateX(3px); } 50% { transform: translateX(-2px); } 75% { transform: translateX(2px); } 100% { transform: translateX(0); } }
@keyframes tw-pulse { 0% { transform: scale(1); } 50% { transform: scale(1.1); } 100% { transform: scale(1); } }
@keyframes tw-rumble { 0% { transform: translate(-2px, 2px); } 25% { transform: translate(2px, -2px); } 50% { transform: translate(-2px, -2px); } 75% { transform: translate(2px, 2px); } 100% { transform: translate(0, 0); } }
`;
  document.head.appendChild(style);
}

function applyTransition(el: HTMLElement): void {
  const name = el.dataset.tw_transition || el.dataset.tw_transition_arrive;
  if (!name) return;
  injectTransitionStyles();
  const duration = el.dataset.tw_transition_time || "0.8s";
  const animName = `tw-${name}`;
  el.style.animation = `${animName} ${duration} ease-in-out`;
}

// --- Alignment resolution ---

function resolveAlign(value: string): string {
  const s = value.trim();
  if (s === "=><=" || s === "=><=") return "center";
  if (s === "<=>") return "justify";
  if (s.endsWith("<=") && !s.startsWith("<=")) return "center";
  if (/^=+>$/.test(s)) return "right";
  if (/^<=+$/.test(s)) return "left";
  return s;
}

// --- Changer application (to DOM elements during render) ---

function applyChanger(el: HTMLElement, changer: Changer): void {
  switch (changer.name) {
    case "color":
    case "colour":
    case "text-colour":
    case "text-color":
      el.style.color = resolveColor(String(changer.args[0]));
      break;
    case "background":
      el.style.backgroundColor = resolveColor(String(changer.args[0]));
      break;
    case "text-style": {
      const style = String(changer.args[0]);
      switch (style) {
        case "bold": el.style.fontWeight = "bold"; break;
        case "italic": el.style.fontStyle = "italic"; break;
        case "underline": el.style.textDecoration = "underline"; break;
        case "strike": el.style.textDecoration = "line-through"; break;
        case "superscript": el.style.verticalAlign = "super"; el.style.fontSize = "0.8em"; break;
        case "subscript": el.style.verticalAlign = "sub"; el.style.fontSize = "0.8em"; break;
        case "blink": el.style.animation = "blink 1s step-end infinite"; break;
        case "shudder": el.style.animation = "shudder 0.1s infinite"; break;
        case "mark": el.style.backgroundColor = "hsla(60, 100%, 50%, 0.6)"; break;
        case "condense": el.style.letterSpacing = "-0.08em"; break;
        case "expand": el.style.letterSpacing = "0.1em"; break;
        case "outline": el.style.webkitTextStroke = "1px"; el.style.color = "transparent"; break;
        case "shadow": el.style.textShadow = "0.08em 0.08em 0.08em black"; break;
        case "emboss": el.style.textShadow = "0.04em 0.04em 0em rgba(0,0,0,0.5)"; break;
        case "blur": el.style.filter = "blur(2px)"; el.style.transition = "filter 0.3s"; break;
        case "smear": el.style.filter = "blur(1px)"; el.style.textShadow = "0em 0em 0.3em currentColor"; break;
        case "mirror": el.style.transform = "scaleX(-1)"; el.style.display = "inline-block"; break;
        case "upside-down": el.style.transform = "scaleY(-1)"; el.style.display = "inline-block"; break;
        case "fade-in-out": el.style.animation = "fade-in-out 2s ease-in-out infinite"; break;
        case "rumble": el.style.animation = "rumble 0.1s infinite"; break;
      }
      break;
    }
    case "font":
      el.style.fontFamily = String(changer.args[0]);
      break;
    case "text-size":
      el.style.fontSize = String(changer.args[0]);
      break;
    case "align": {
      const a = resolveAlign(String(changer.args[0]));
      el.style.textAlign = a;
      if (a === "center" || a === "right") el.style.display = "block";
      break;
    }
    case "opacity":
      el.style.opacity = String(changer.args[0]);
      break;
    case "text-rotate-z":
      el.style.transform = `rotate(${changer.args[0]}deg)`;
      el.style.display = "inline-block";
      break;
    case "css":
      el.setAttribute("style", (el.getAttribute("style") || "") + ";" + String(changer.args[0]));
      break;
    case "transition":
    case "transition-time":
    case "transition-arrive":
    case "transition-depart":
      el.dataset[`tw_${changer.name.replace(/-/g, "_")}`] = String(changer.args[0]);
      break;
    case "collapse":
      el.classList.add("tw-collapse");
      break;
    case "nobr":
      el.classList.add("tw-nobr");
      break;
    case "hidden":
      el.style.display = "none";
      break;
    case "hover-style":
      el.dataset.tw_hover = JSON.stringify(changer.args[0]);
      break;
  }
}

function applyChangers(el: HTMLElement, changers: Changer | Changer[]): void {
  if (Array.isArray(changers)) {
    for (const c of changers) applyChanger(el, c);
  } else {
    applyChanger(el, changers);
  }
  applyTransition(el);
}

// --- Timer tracking ---

const activeTimers: number[] = [];

/** Clear the #passages container and cancel active timers. */
export function clear(): void {
  const container = document.getElementById("passages");
  if (container) {
    while (container.firstChild) {
      container.removeChild(container.firstChild);
    }
  }
  for (const id of activeTimers) {
    cancelInterval(id);
  }
  activeTimers.length = 0;
}

// --- Passage lookup (set by navigation.ts) ---

let passageLookup: ((name: string) => (() => ContentNode[]) | undefined) | null = null;

/** Called by navigation.ts to provide passage lookup for (display:). */
export function setPassageLookup(fn: (name: string) => (() => ContentNode[]) | undefined): void {
  passageLookup = fn;
}

// --- Stop signal for (live:) ---

let stopRequested = false;

export function requestStop(): void {
  stopRequested = true;
}

// --- Renderer ---

/** Render a content tree into a DOM container. */
export function render(container: Element, nodes: ContentNode[]): void {
  renderNodes(container, nodes);
}

function renderNodes(parent: Element | DocumentFragment, nodes: ContentNode[]): void {
  for (const node of nodes) {
    renderNode(parent, node);
  }
}

function renderNode(parent: Element | DocumentFragment, node: ContentNode): void {
  if (typeof node === "string") {
    parent.appendChild(document.createTextNode(node));
    return;
  }
  if (Array.isArray(node)) {
    renderNodes(parent, node);
    return;
  }
  switch (node.t) {
    case "el": {
      const el = document.createElement(node.tag);
      for (const [k, v] of Object.entries(node.attrs)) el.setAttribute(k, v);
      renderNodes(el, node.c);
      parent.appendChild(el);
      break;
    }
    case "styled": {
      const span = document.createElement("span");
      applyChangers(span, node.changer);
      renderNodes(span, node.c);
      parent.appendChild(span);
      break;
    }
    case "link": {
      const a = document.createElement("a");
      a.textContent = node.text;
      a.className = "tw-link";
      const passage = node.passage;
      a.addEventListener("click", (e) => {
        e.preventDefault();
        import("./navigation").then((nav) => nav.goto(passage));
      });
      parent.appendChild(a);
      break;
    }
    case "link_cb": {
      const a = document.createElement("a");
      a.textContent = node.text;
      a.className = "tw-link";
      const cb = node.cb;
      a.addEventListener("click", (e) => {
        e.preventDefault();
        const container = a.parentElement;
        if (container) {
          const result = cb();
          const frag = document.createDocumentFragment();
          renderNodes(frag, result);
          // Replace link with callback content
          a.replaceWith(frag);
        }
      });
      parent.appendChild(a);
      break;
    }
    case "live": {
      const container = document.createElement("span");
      container.className = "tw-live";
      parent.appendChild(container);
      const cb = node.cb;
      const ms = node.interval * 1000;
      const id = scheduleInterval(() => {
        container.innerHTML = "";
        const result = cb();
        renderNodes(container, result);
        if (stopRequested) {
          cancelInterval(id);
          stopRequested = false;
        }
      }, ms);
      activeTimers.push(id);
      break;
    }
    case "void": {
      const el = document.createElement(node.tag);
      for (const [k, v] of Object.entries(node.attrs)) el.setAttribute(k, v);
      parent.appendChild(el);
      break;
    }
    case "print": {
      parent.appendChild(document.createTextNode(String(node.v)));
      break;
    }
    case "display": {
      if (passageLookup) {
        const fn = passageLookup(node.passage);
        if (fn) {
          const result = fn();
          renderNodes(parent, result);
        } else {
          parent.appendChild(document.createTextNode(`[passage not found: "${node.passage}"]`));
        }
      }
      break;
    }
  }
}
