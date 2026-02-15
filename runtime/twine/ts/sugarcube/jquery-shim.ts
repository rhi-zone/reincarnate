/** Minimal jQuery shim for SugarCube compatibility.
 *
 * DoL's user scripts use jQuery for DOM queries, event binding, and
 * SugarCube-specific extensions (.wikiWithOptions). This provides just enough
 * to let the scripts execute without crashing.
 *
 * Registered as both `jQuery` and `$` on globalThis.
 */

import * as Events from "./events";

class JQueryLike {
  private elements: Element[];

  constructor(elements: Element[]) {
    this.elements = elements;
    // Expose numeric indices and length like a real jQuery object
    for (let i = 0; i < elements.length; i++) {
      (this as any)[i] = elements[i];
    }
    this.length = elements.length;
  }

  length: number;

  // --- Traversal ---

  find(selector: string): JQueryLike {
    const results: Element[] = [];
    for (const el of this.elements) {
      results.push(...Array.from(el.querySelectorAll(selector)));
    }
    return new JQueryLike(results);
  }

  parent(): JQueryLike {
    const parents: Element[] = [];
    for (const el of this.elements) {
      if (el.parentElement && !parents.includes(el.parentElement)) {
        parents.push(el.parentElement);
      }
    }
    return new JQueryLike(parents);
  }

  closest(selector: string): JQueryLike {
    const results: Element[] = [];
    for (const el of this.elements) {
      const found = el.closest(selector);
      if (found && !results.includes(found)) {
        results.push(found);
      }
    }
    return new JQueryLike(results);
  }

  children(selector?: string): JQueryLike {
    const results: Element[] = [];
    for (const el of this.elements) {
      const kids = Array.from(el.children);
      if (selector) {
        results.push(...kids.filter(k => k.matches(selector)));
      } else {
        results.push(...kids);
      }
    }
    return new JQueryLike(results);
  }

  first(): JQueryLike {
    return new JQueryLike(this.elements.length ? [this.elements[0]] : []);
  }

  last(): JQueryLike {
    return new JQueryLike(this.elements.length ? [this.elements[this.elements.length - 1]] : []);
  }

  eq(index: number): JQueryLike {
    const el = this.elements[index];
    return new JQueryLike(el ? [el] : []);
  }

  filter(selector: string): JQueryLike {
    return new JQueryLike(this.elements.filter(el => el.matches(selector)));
  }

  not(selector: string): JQueryLike {
    return new JQueryLike(this.elements.filter(el => !el.matches(selector)));
  }

  // --- Content manipulation ---

  append(...content: any[]): JQueryLike {
    for (const el of this.elements) {
      for (const c of content) {
        if (typeof c === "string") {
          el.insertAdjacentHTML("beforeend", c);
        } else if (c instanceof Node) {
          el.appendChild(c);
        } else if (c instanceof JQueryLike) {
          for (const child of c.elements) {
            el.appendChild(child);
          }
        }
      }
    }
    return this;
  }

  prepend(...content: any[]): JQueryLike {
    for (const el of this.elements) {
      for (const c of content) {
        if (typeof c === "string") {
          el.insertAdjacentHTML("afterbegin", c);
        } else if (c instanceof Node) {
          el.insertBefore(c, el.firstChild);
        }
      }
    }
    return this;
  }

  after(...content: any[]): JQueryLike {
    for (const el of this.elements) {
      for (const c of content) {
        if (typeof c === "string") {
          el.insertAdjacentHTML("afterend", c);
        } else if (c instanceof Node) {
          el.parentNode?.insertBefore(c, el.nextSibling);
        }
      }
    }
    return this;
  }

  before(...content: any[]): JQueryLike {
    for (const el of this.elements) {
      for (const c of content) {
        if (typeof c === "string") {
          el.insertAdjacentHTML("beforebegin", c);
        } else if (c instanceof Node) {
          el.parentNode?.insertBefore(c, el);
        }
      }
    }
    return this;
  }

  empty(): JQueryLike {
    for (const el of this.elements) {
      while (el.firstChild) el.removeChild(el.firstChild);
    }
    return this;
  }

  remove(): JQueryLike {
    for (const el of this.elements) {
      el.remove();
    }
    return this;
  }

  clone(): JQueryLike {
    return new JQueryLike(this.elements.map(el => el.cloneNode(true) as Element));
  }

  replaceWith(content: any): JQueryLike {
    for (const el of this.elements) {
      if (typeof content === "string") {
        el.insertAdjacentHTML("beforebegin", content);
      } else if (content instanceof Node) {
        el.parentNode?.insertBefore(content, el);
      }
      el.remove();
    }
    return this;
  }

  // --- Getters/Setters ---

  html(): string;
  html(value: string): JQueryLike;
  html(value?: string): any {
    if (value === undefined) {
      return this.elements[0]?.innerHTML ?? "";
    }
    for (const el of this.elements) {
      el.innerHTML = value;
    }
    return this;
  }

  text(): string;
  text(value: string): JQueryLike;
  text(value?: string): any {
    if (value === undefined) {
      return this.elements[0]?.textContent ?? "";
    }
    for (const el of this.elements) {
      el.textContent = value;
    }
    return this;
  }

  val(): string;
  val(value: string): JQueryLike;
  val(value?: string): any {
    if (value === undefined) {
      return (this.elements[0] as HTMLInputElement)?.value ?? "";
    }
    for (const el of this.elements) {
      (el as HTMLInputElement).value = value;
    }
    return this;
  }

  attr(name: string): string | undefined;
  attr(name: string, value: string): JQueryLike;
  attr(name: string, value?: string): any {
    if (value === undefined) {
      return this.elements[0]?.getAttribute(name) ?? undefined;
    }
    for (const el of this.elements) {
      el.setAttribute(name, value);
    }
    return this;
  }

  removeAttr(name: string): JQueryLike {
    for (const el of this.elements) {
      el.removeAttribute(name);
    }
    return this;
  }

  prop(name: string): any;
  prop(name: string, value: any): JQueryLike;
  prop(name: string, value?: any): any {
    if (value === undefined) {
      return (this.elements[0] as any)?.[name];
    }
    for (const el of this.elements) {
      (el as any)[name] = value;
    }
    return this;
  }

  data(key: string): any;
  data(key: string, value: any): JQueryLike;
  data(key: string, value?: any): any {
    if (value === undefined) {
      return (this.elements[0] as HTMLElement)?.dataset?.[key];
    }
    for (const el of this.elements) {
      (el as HTMLElement).dataset[key] = String(value);
    }
    return this;
  }

  // --- CSS/Classes ---

  css(prop: string): string;
  css(prop: string, value: string): JQueryLike;
  css(props: Record<string, string>): JQueryLike;
  css(prop: any, value?: string): any {
    if (typeof prop === "object") {
      for (const el of this.elements) {
        for (const [k, v] of Object.entries(prop)) {
          (el as HTMLElement).style.setProperty(k, v as string);
        }
      }
      return this;
    }
    if (value === undefined) {
      return (this.elements[0] as HTMLElement)?.style?.getPropertyValue(prop) ?? "";
    }
    for (const el of this.elements) {
      (el as HTMLElement).style.setProperty(prop, value);
    }
    return this;
  }

  addClass(className: string): JQueryLike {
    for (const el of this.elements) {
      el.classList.add(...className.split(/\s+/).filter(Boolean));
    }
    return this;
  }

  removeClass(className: string): JQueryLike {
    for (const el of this.elements) {
      el.classList.remove(...className.split(/\s+/).filter(Boolean));
    }
    return this;
  }

  toggleClass(className: string, force?: boolean): JQueryLike {
    for (const el of this.elements) {
      el.classList.toggle(className, force);
    }
    return this;
  }

  hasClass(className: string): boolean {
    return this.elements.some(el => el.classList.contains(className));
  }

  // --- Visibility ---

  show(): JQueryLike {
    for (const el of this.elements) {
      (el as HTMLElement).style.display = "";
    }
    return this;
  }

  hide(): JQueryLike {
    for (const el of this.elements) {
      (el as HTMLElement).style.display = "none";
    }
    return this;
  }

  toggle(show?: boolean): JQueryLike {
    for (const el of this.elements) {
      const htmlEl = el as HTMLElement;
      if (show === undefined) {
        htmlEl.style.display = htmlEl.style.display === "none" ? "" : "none";
      } else {
        htmlEl.style.display = show ? "" : "none";
      }
    }
    return this;
  }

  // --- Events ---

  on(events: string, selectorOrHandler: any, handler?: Function): JQueryLike {
    const actualHandler = handler || selectorOrHandler;
    for (const event of events.split(/\s+/)) {
      if (event.startsWith(":")) {
        // SugarCube custom event — route to event bus
        Events.on(event, actualHandler);
      } else {
        for (const el of this.elements) {
          el.addEventListener(event.split(".")[0], actualHandler as EventListener);
        }
      }
    }
    return this;
  }

  one(events: string, selectorOrHandler: any, handler?: Function): JQueryLike {
    const actualHandler = handler || selectorOrHandler;
    for (const event of events.split(/\s+/)) {
      if (event.startsWith(":")) {
        Events.one(event, actualHandler);
      } else {
        for (const el of this.elements) {
          el.addEventListener(event.split(".")[0], actualHandler as EventListener, { once: true });
        }
      }
    }
    return this;
  }

  off(events: string, handler?: Function): JQueryLike {
    for (const event of events.split(/\s+/)) {
      if (event.startsWith(":")) {
        if (handler) Events.off(event, handler);
      } else {
        if (handler) {
          for (const el of this.elements) {
            el.removeEventListener(event.split(".")[0], handler as EventListener);
          }
        }
      }
    }
    return this;
  }

  trigger(event: string, ...args: any[]): JQueryLike {
    if (event.startsWith(":")) {
      Events.trigger(event, ...args);
    } else {
      for (const el of this.elements) {
        el.dispatchEvent(new Event(event, { bubbles: true }));
      }
    }
    return this;
  }

  click(handler?: Function): JQueryLike {
    if (handler) {
      return this.on("click", handler);
    }
    for (const el of this.elements) {
      (el as HTMLElement).click();
    }
    return this;
  }

  // --- Iteration ---

  each(fn: (index: number, element: Element) => void): JQueryLike {
    this.elements.forEach((el, i) => fn(i, el));
    return this;
  }

  map(fn: (index: number, element: Element) => any): JQueryLike {
    const results: Element[] = [];
    for (let i = 0; i < this.elements.length; i++) {
      const result = fn(i, this.elements[i]);
      if (result instanceof Element) results.push(result);
    }
    return new JQueryLike(results);
  }

  toArray(): Element[] {
    return [...this.elements];
  }

  get(index?: number): any {
    if (index === undefined) return [...this.elements];
    return this.elements[index];
  }

  is(selector: string): boolean {
    return this.elements.some(el => el.matches(selector));
  }

  // --- Dimensions (stubs) ---

  width(): number { return (this.elements[0] as HTMLElement)?.offsetWidth ?? 0; }
  height(): number { return (this.elements[0] as HTMLElement)?.offsetHeight ?? 0; }
  outerWidth(): number { return (this.elements[0] as HTMLElement)?.offsetWidth ?? 0; }
  outerHeight(): number { return (this.elements[0] as HTMLElement)?.offsetHeight ?? 0; }

  // --- Animations (no-op) ---

  animate(_props: any, _duration?: any, _easing?: any, _complete?: Function): JQueryLike { return this; }
  fadeIn(_duration?: any, _complete?: Function): JQueryLike { return this.show(); }
  fadeOut(_duration?: any, _complete?: Function): JQueryLike { return this.hide(); }
  slideDown(_duration?: any, _complete?: Function): JQueryLike { return this.show(); }
  slideUp(_duration?: any, _complete?: Function): JQueryLike { return this.hide(); }
  stop(): JQueryLike { return this; }

  // --- SugarCube extensions ---

  wikiWithOptions(_opts: any, _text: string): JQueryLike {
    // Minimal: append text content directly (no wiki parsing)
    for (const el of this.elements) {
      el.appendChild(document.createTextNode(_text));
    }
    return this;
  }

  wiki(_text: string): JQueryLike {
    return this.wikiWithOptions({}, _text);
  }
}

/** jQuery factory function. */
function jQuery(selectorOrElement: any): JQueryLike {
  if (typeof selectorOrElement === "string") {
    // HTML string → create element
    if (selectorOrElement.trim().startsWith("<")) {
      const temp = document.createElement("template");
      temp.innerHTML = selectorOrElement.trim();
      return new JQueryLike(Array.from(temp.content.children));
    }
    // Selector string → query DOM
    return new JQueryLike(Array.from(document.querySelectorAll(selectorOrElement)));
  }
  if (selectorOrElement instanceof Element) {
    return new JQueryLike([selectorOrElement]);
  }
  if (selectorOrElement instanceof Document) {
    // $(document) — wrap document.documentElement for event binding
    return new JQueryLike([document.documentElement]);
  }
  if (selectorOrElement instanceof DocumentFragment) {
    return new JQueryLike(Array.from(selectorOrElement.children));
  }
  // Fallback: empty wrapper
  return new JQueryLike([]);
}

// Static methods
jQuery.event = { trigger(_event: string) {} };
jQuery.extend = Object.assign;
jQuery.isArray = Array.isArray;
jQuery.isFunction = (obj: any) => typeof obj === "function";
jQuery.isPlainObject = (obj: any) => obj !== null && typeof obj === "object" && Object.getPrototypeOf(obj) === Object.prototype;
jQuery.each = (obj: any, fn: Function) => {
  if (Array.isArray(obj)) {
    obj.forEach((v: any, i: number) => fn(i, v));
  } else if (obj && typeof obj === "object") {
    for (const k of Object.keys(obj)) fn(k, obj[k]);
  }
};
jQuery.noop = () => {};
jQuery.Deferred = () => {
  let resolveFn: Function = () => {};
  let rejectFn: Function = () => {};
  const p = new Promise((resolve, reject) => { resolveFn = resolve; rejectFn = reject; });
  return {
    resolve: (...args: any[]) => { resolveFn(...args); return this; },
    reject: (...args: any[]) => { rejectFn(...args); return this; },
    promise: () => p,
    then: (fn: Function) => p.then(fn as any),
    done: (fn: Function) => { p.then(fn as any); return this; },
    fail: (fn: Function) => { p.catch(fn as any); return this; },
    always: (fn: Function) => { p.finally(fn as any); return this; },
  };
};

/** Register jQuery and $ on globalThis. */
export function installJQuery(): void {
  const g = globalThis as any;
  if (!g.jQuery) g.jQuery = jQuery;
  if (!g.$) g.$ = jQuery;
}
