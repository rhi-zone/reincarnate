/**
 * SugarCube Array and String prototype extensions.
 *
 * SugarCube augments the built-in Array and String prototypes with utility
 * methods used extensively in SugarCube games. This module declares the types
 * (via `declare global`) and installs the implementations at runtime.
 *
 * Called once from `SCEngine.ensureGlobals()` via `installSugarCubeExtensions()`.
 */

declare global {
  interface Array<T> {
    /** Remove all occurrences of each item; returns `this`. */
    delete(...items: T[]): this;
    /** Remove elements at the given indices; returns the removed elements. */
    deleteAt(...indices: number[]): T[];
    /** Returns the last element without removing it, or `undefined` if empty. */
    last(): T | undefined;
    /** Removes and returns a random element, or `undefined` if empty. */
    pluck(): T | undefined;
    /** Removes and returns `n` random elements. */
    pluckMany(n: number): T[];
    /** Push items that are not already present; returns new length. */
    pushUnique(...items: T[]): number;
    /** Returns a random element without removing it, or `undefined` if empty. */
    random(): T | undefined;
    /** Returns `n` random elements without removing them. */
    randomMany(n: number): T[];
    /** In-place Fisher–Yates shuffle; returns `this`. */
    shuffle(): this;
    /** Returns a shuffled copy. */
    toShuffled(): T[];
    /** Returns the number of elements strictly equal to `item`. */
    count(item: T): number;
    /** Returns the number of elements for which `predicate` returns true. */
    countWith(predicate: (item: T) => boolean): number;
    /** Returns `true` if every item is present in the array. */
    includesAll(...items: T[]): boolean;
    /** Returns `true` if any item is present in the array. */
    includesAny(...items: T[]): boolean;
    /** Returns the element at 1-based index `n` (DoL extension). */
    select(n: number): T | undefined;
    /** Appends items not already present and returns `this` (DoL/TRC extension). */
    concatUnique(...items: any[]): this;
  }

  interface Math {
    /** Clamp `num` to the range [`min`, `max`]. */
    clamp(num: number, min: number, max: number): number;
  }

  interface String {
    /** Uppercase the first character. */
    toUpperFirst(): string;
    /** Lowercase the first character. */
    toLowerFirst(): string;
    /** sprintf-style format (`%s`, `%d`, `%f`, `%i`). */
    format(...args: unknown[]): string;
  }
}

export function installSugarCubeExtensions(): void {
  // --- Array ---
  // `as unknown as Record<string, unknown>` is required: TypeScript does not allow
  // assigning properties to typed prototype objects without bypassing the type system.
  const ap = Array.prototype as unknown as Record<string, unknown>;

  if (!ap.delete) {
    // `this: unknown[]` cannot be used directly in prototype assignments since TypeScript
    // doesn't track the receiver type through `Array.prototype as any` assignment.
    // The cast to `unknown[]` inside the body is sufficient for type safety.
    ap.delete = function (this: unknown[], ...items: unknown[]): unknown[] {
      for (const item of items) {
        let i = 0;
        while (i < this.length) {
          if (this[i] === item) { this.splice(i, 1); } else { i++; }
        }
      }
      return this;
    };
  }

  if (!ap.deleteAt) {
    ap.deleteAt = function (this: unknown[], ...indices: number[]): unknown[] {
      if (this.length === 0 || indices.length === 0) return [];
      const normalized = indices
        .map(i => i < 0 ? Math.max(0, this.length + i) : Math.min(i, this.length - 1))
        .sort((a, b) => b - a);
      const removed: unknown[] = [];
      const seen = new Set<number>();
      for (const idx of normalized) {
        if (!seen.has(idx)) {
          seen.add(idx);
          removed.unshift(...this.splice(idx, 1));
        }
      }
      return removed;
    };
  }

  if (!ap.last) {
    ap.last = function (this: unknown[]): unknown {
      return this.length > 0 ? this[this.length - 1] : undefined;
    };
  }

  if (!ap.pluck) {
    ap.pluck = function (this: unknown[]): unknown {
      if (this.length === 0) return undefined;
      const idx = Math.floor(Math.random() * this.length);
      return this.splice(idx, 1)[0];
    };
  }

  if (!ap.pluckMany) {
    ap.pluckMany = function (this: unknown[], n: number): unknown[] {
      const copy = this.slice();
      const result: unknown[] = [];
      const count = Math.min(n, copy.length);
      for (let i = 0; i < count; i++) {
        const idx = Math.floor(Math.random() * copy.length);
        result.push(copy.splice(idx, 1)[0]);
      }
      return result;
    };
  }

  if (!ap.pushUnique) {
    ap.pushUnique = function (this: unknown[], ...items: unknown[]): number {
      for (const item of items) {
        if (!this.includes(item)) this.push(item);
      }
      return this.length;
    };
  }

  if (!ap.random) {
    ap.random = function (this: unknown[]): unknown {
      return this.length > 0 ? this[Math.floor(Math.random() * this.length)] : undefined;
    };
  }

  if (!ap.randomMany) {
    ap.randomMany = function (this: unknown[], n: number): unknown[] {
      if (this.length === 0 || n <= 0) return [];
      const indices = Array.from({ length: this.length }, (_, i) => i);
      const result: unknown[] = [];
      const count = Math.min(n, this.length);
      for (let i = 0; i < count; i++) {
        const pick = Math.floor(Math.random() * indices.length);
        result.push(this[indices.splice(pick, 1)[0]!]);
      }
      return result;
    };
  }

  if (!ap.shuffle) {
    ap.shuffle = function (this: unknown[]): unknown[] {
      for (let i = this.length - 1; i > 0; i--) {
        const j = Math.floor(Math.random() * (i + 1));
        [this[i], this[j]] = [this[j], this[i]];
      }
      return this;
    };
  }

  if (!ap.toShuffled) {
    ap.toShuffled = function (this: unknown[]): unknown[] {
      return (this.slice() as unknown[] & { shuffle(): unknown[] }).shuffle();
    };
  }

  if (!ap.count) {
    ap.count = function (this: unknown[], item: unknown): number {
      return this.filter(el => el === item).length;
    };
  }

  if (!ap.countWith) {
    ap.countWith = function (this: unknown[], predicate: (item: unknown) => boolean): number {
      return this.filter(predicate).length;
    };
  }

  if (!ap.includesAll) {
    ap.includesAll = function (this: unknown[], ...items: unknown[]): boolean {
      return items.every(item => this.includes(item));
    };
  }

  if (!ap.includesAny) {
    ap.includesAny = function (this: unknown[], ...items: unknown[]): boolean {
      return items.some(item => this.includes(item));
    };
  }

  if (!ap.select) {
    ap.select = function (this: unknown[], n: number): unknown {
      // DoL uses 1-based indexing for select()
      return this[n - 1];
    };
  }

  if (!ap.concatUnique) {
    ap.concatUnique = function (this: unknown[], ...items: unknown[]): unknown[] {
      for (const item of items) {
        if (!this.includes(item)) this.push(item);
      }
      return this;
    };
  }

  // --- Math ---
  if (!Math.clamp) {
    (Math as unknown as Record<string, unknown>).clamp = function (num: number, min: number, max: number): number {
      return Math.min(Math.max(num, min), max);
    };
  }

  // --- String ---
  // Same bypass needed as for Array.prototype above.
  const sp = String.prototype as unknown as Record<string, unknown>;

  if (!sp.toUpperFirst) {
    sp.toUpperFirst = function (this: string): string {
      return this.length > 0 ? this.charAt(0).toUpperCase() + this.slice(1) : this;
    };
  }

  if (!sp.toLowerFirst) {
    sp.toLowerFirst = function (this: string): string {
      return this.length > 0 ? this.charAt(0).toLowerCase() + this.slice(1) : this;
    };
  }

  if (!sp.format) {
    sp.format = function (this: string, ...args: unknown[]): string {
      let i = 0;
      return this.replace(/%([sdfi%])/g, (match, spec) => {
        if (spec === "%") return "%";
        const arg = args[i++];
        switch (spec) {
          case "d": case "i": return String(Math.trunc(Number(arg)));
          case "f": return String(Number(arg));
          default: return String(arg);
        }
      });
    };
  }
}
