# ADR 001: Harlowe Content Emission via `h` Parameter (Hyperscript)

**Status:** Accepted

**Date:** 2026-02-16

## Context

Harlowe passage output originally used a two-phase model: passage functions
build a `ContentNode[]` tree (virtual DOM), return it, and navigation calls
`render(container, nodes)` to walk the tree and create DOM elements.

An intermediate attempt used push-based emission (`__out.push(...)`) with AST
passes (`fold_array_init_push`, `merge_adjacent_push`) to clean up the output.
This reduced temporary variables but still allocated arrays and tagged objects
just to walk them once.

Both approaches produce excessive `vN` temporaries in the compiled output and
the intermediate data structures are pure overhead — allocated once, consumed
once, then discarded.

## Decision

Eliminate the content tree entirely. Passage functions receive an `h` context
parameter (hyperscript convention) and emit content via side-effecting method
calls:

```typescript
function passage_example(h: HarloweContext) {
  h.text("Hello ");
  h.em(h.strong("world"));
  h.br();
  if (h.get("x") === 1) {
    h.text("yes");
  }
  h.link("Go", "Next");
}
```

### Key Design Choices

- **`h` parameter name**: Follows the hyperscript convention (React's
  `createElement` was originally `h()`). The method names (`em`, `strong`,
  `br`, `link`) *are* the hyperscript vocabulary.

- **Direct output**: In the TypeScript/browser backend, `h.br()` creates and
  appends a `<br>` immediately via the DOM. The `h` object holds a container
  stack for nesting. A native backend would use a different `h` implementation
  (e.g. retained scene graph, immediate-mode UI, or a platform-specific
  layout engine) — the passage code is identical either way.

- **Nesting via return values**: `h.em(h.strong("world"))` — inner call
  appends to current container first, outer call moves it via `appendChild`
  (DOM move semantics). Callbacks (`() => void`) for complex nesting where
  multiple children need sequential emission.

- **State and navigation on `h`**: `h.get()`, `h.set()`, `h.goto()` — the
  passage function receives all engine context through one parameter. No
  global state accessed from passage code.

- **Backend-agnostic**: The `h` interface is a protocol, not tied to DOM. A
  native Rust backend would implement the same method surface on a different
  output abstraction (e.g. `wgpu` text layout, `egui` widgets, or a retained
  element tree). The compiled passage code emits `h.text()`, `h.em()`, etc.
  regardless of what `h` is backed by.

- **Pure functions stay as imports**: `plus()`, `Collections.*`, math functions
  remain regular imports via `function_modules`. Only output-producing and
  state-accessing operations go through `h`.

- **try/finally cleanup**: Navigation wraps the passage call; `h.closeAll()`
  in the finally block handles unclosed elements from early returns or gotos.

## Consequences

- **Eliminated**: `ContentNode` / `ContentElement` types, `render()` /
  `renderNode()` functions, array-based builder functions, push-folding AST
  passes, `pass_defaults` configuration.

- **New runtime**: `HarloweContext` class in `harlowe/context.ts` with
  container stack and output emission methods.

- **New SystemCall namespace**: `Harlowe.H` replaces `Harlowe.Output` for
  content emission. Backend rewrites map `Harlowe.H.*` calls to method calls
  on the `h` parameter variable.

- **Simpler IR**: Passage functions are `(h) => void` — no return value
  merging, no array construction, no spread operations for content.

- **Trade-off**: Passage functions are no longer pure (they produce output
  through `h`). This is acceptable because Harlowe passages are inherently
  side-effecting (they produce visible output) and were never meaningfully
  testable as pure functions anyway.

## Alternatives Considered

### 1. Keep ContentNode tree, optimize the builder

Still allocates intermediate objects. The tree is walked exactly once to
produce DOM. Eliminating it removes an entire layer of allocation and
indirection. The push-based variant (`__out.push(...)`) reduced temporaries
but the fundamental overhead remained: arrays of tagged objects allocated,
iterated, discarded.

### 2. String-based HTML concatenation

Fast allocation-wise, but loses DOM references needed for live content —
timed macros (`(live:)`), interactive links, changers that modify elements
after creation all require real DOM nodes. `innerHTML` also carries XSS risk
with interpolated user state.

### 3. JSX / template literals

Would require a JSX transform in the build pipeline. Template literals
(`html\`...\``) would need a tagged template library (lit-html, etc.) adding
a runtime dependency. The `h.*` approach achieves the same result with plain
function calls and zero build tooling beyond TypeScript.

### 4. `using` keyword (Explicit Resource Management)

```typescript
{
  using _ = h.em();
  h.text("hello");
}
// auto-closes <em> at block exit via Symbol.dispose
```

Clean scoping, but `using` is an ECMAScript Stage 3 proposal — it requires
`tsc` with `--target esnext` and is not natively supported by any browser
engine or Node.js as of 2026. Adopting it would force a TypeScript compilation
step on every consumer of the runtime, which conflicts with the goal of
emitting plain JavaScript that runs without a build step. Revisit when
`using` lands in V8/SpiderMonkey.

### 5. Nested arrays representing tree structure

```typescript
function passage_example() {
  return ["div", {},
    ["text", "Hello "],
    ["em", {}, ["strong", {}, "world"]],
    cond ? ["text", "yes"] : null,
    ["link", { target: "Next" }, "Go"]
  ];
}
```

Hiccup-style (Clojure) or Mithril/Snabbdom-style virtual DOM as nested
arrays. The tree structure maps naturally to element nesting, but
conditionals are awkward — you end up with ternaries that produce `null`
and need filtering, or intermediate variables that fragment the tree.
Control flow (`if`/`else`, loops) doesn't compose cleanly inside array
literals. There is also slight overhead in walking and interpreting the
tag/attrs/children protocol at runtime. The `h.*` approach handles
conditionals naturally (just `if (cond) { h.text("yes"); }`) and
produces output directly without an intermediate tree walk.

### 6. Bare open/close calls with block grouping

```typescript
function passage_example(h: HarloweContext) {
  h.text("Hello ");
  h.open("em");
  {
    h.open("strong");
    h.text("world");
    h.close("strong");
  }
  h.close("em");
}
```

Explicit `open()`/`close()` pairs with `{}` blocks for visual grouping.
Simple to implement and emit. The problem is that `{}` blocks are purely
cosmetic in JavaScript — they provide no enforcement. The compiler must
still guarantee balanced open/close pairs, and any bug produces silently
malformed DOM. The `h.em(child)` approach encodes the nesting in the call
structure itself: a missing argument is a compile error, not a runtime DOM
corruption. The try/finally `closeAll()` safety net exists for the rare
callback-based nesting case, not as the primary mechanism.

### 7. Callback-based nesting as primary mechanism

```typescript
function passage_example(h: HarloweContext) {
  h.text("Hello ");
  h.em(() => {
    h.strong(() => {
      h.text("world");
    });
  });
  h.link("Go", "Next");
}
```

Every element with children takes a callback. Clean nesting and automatic
close — the element is closed when the callback returns. However, callbacks
break control flow: a `return` inside the callback exits the callback, not
the passage function. A `(goto:)` macro in the middle of nested content
cannot exit the passage — it would need to throw an exception to unwind
the callback stack, adding error handling complexity. `break`/`continue`
also cannot cross callback boundaries. The chosen design uses callbacks
only for rare complex nesting; the primary mechanism is argument-based
(`h.em(h.strong("world"))`) which keeps everything in the passage
function's own scope.

### 8. Generator functions with `yield*`

```typescript
function* passage_example() {
  yield { tag: "text", content: "Hello " };
  yield* em(strong("world"));
}
```

Elegant composition via `yield*`, and the caller controls when to flush
to DOM. However, generator functions carry a per-call performance cost:
each `yield` suspends and resumes the generator's stack frame, and `yield*`
adds delegation overhead on top. For Harlowe passages that produce dozens
to hundreds of content nodes per render, this accumulates. The `h.*`
approach is a series of plain function calls with no suspension machinery.
