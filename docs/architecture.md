# Architecture

## Philosophy

Reincarnate performs **full recompilation**, not interpretation. Source bytecode and scripts are decompiled into a typed intermediate representation, optimized, and compiled to native targets. The goal is accurate reproduction — preserve fidelity, don't redesign. When rendering can't be fully lifted, overlay a modern UI layer over the original rather than patching internal rendering.

## Pipeline

```
Source binary/scripts
        │
        ▼
   ┌──────────┐
   │ Frontend  │   (per-engine: Flash, Ren'Py, RPG Maker, etc.)
   └────┬─────┘
        │  untyped IR
        ▼
   ┌──────────┐
   │   Type    │
   │ Inference │
   └────┬─────┘
        │  typed IR
        ▼
   ┌──────────┐
   │ Transform │   (optimization passes, coroutine lowering, etc.)
   │  Passes   │
   └────┬─────┘
        │  optimized typed IR
        ▼
   ┌──────────┐
   │ Backend   │   (Rust source, TypeScript, etc.)
   └──────────┘
```

Frontends parse engine-specific formats and emit untyped IR. Type inference recovers concrete types. Transform passes optimize and lower (e.g., coroutines to state machines). Backends emit target code.

## Supported Engines

| Engine | Format | Strategy |
|--------|--------|----------|
| **Flash (AVM2)** | ABC bytecode | Full recompilation (first target) |
| **Ren'Py** | RPY scripts / RPYC bytecode | Full recompilation |
| **RPG Maker VX Ace** | Ruby/RGSS scripts | Full recompilation |
| **RPG Maker MV/MZ** | JSON event scripts + JS engine | Patch-in-place (compile JSON events, optimize engine, inject plugins) |
| **GameMaker** | GML bytecode | Full recompilation |
| **Director/Shockwave** | Lingo scripts | Full recompilation |
| **Twine / Inform** | Story formats | Full recompilation |
| **VB6** | P-Code | Full recompilation |
| **Java Applets** | JVM bytecode | Full recompilation |
| **Silverlight** | .NET IL | Full recompilation |
| **HyperCard / ToolBook** | Stack formats | Full recompilation |

RPG Maker MV/MZ is a special case: since the engine already runs in JavaScript, it may be more practical to compile event scripts and optimize the existing engine rather than full recompilation.

## Library Replacement (HLE)

Reincarnate works like a console recompiler: user logic is faithfully
translated, but **original runtime libraries are detected and replaced** with
native equivalents. This is the HLE (High-Level Emulation) approach — instead
of emulating the original runtime instruction-by-instruction, we recognize
API boundaries and swap in modern implementations.

```
┌─────────────────────────────────────────────────────┐
│                   Original binary                    │
│  ┌──────────────┐  ┌─────────────────────────────┐  │
│  │  User logic   │  │  Runtime libraries           │  │
│  │  (game code)  │  │  (flash.display, flash.text) │  │
│  └──────┬───────┘  └──────────────┬──────────────┘  │
│         │                         │                  │
└─────────┼─────────────────────────┼──────────────────┘
          │                         │
          ▼                         ▼
   Recompiled to IR          Detected at boundary,
   → transforms              replaced with native
   → codegen                 implementation
          │                         │
          ▼                         ▼
   ┌──────────────┐  ┌─────────────────────────────┐
   │  Translated   │  │  Replacement runtime         │
   │  user code    │──│  (canvas renderer, DOM text,  │
   │  (.ts / .rs)  │  │   Web Audio, etc.)           │
   └──────────────┘  └─────────────────────────────┘
```

### Why this matters

Every legacy runtime has a standard library: Flash has `flash.display`,
`flash.events`, `flash.text`; Director has `Lingo` built-ins; VB6 has COM
controls and the VB runtime. These libraries define the app's interaction
with rendering, input, persistence, etc.

The recompiler's job is two-fold:
1. **Translate user logic** — faithful recompilation of the app's own code
2. **Replace runtime libraries** — swap the original runtime with a modern
   implementation that provides the same API surface

This separation is what makes the output actually *run*. Translating
`MovieClip.gotoAndStop(3)` to TypeScript is useless without a `MovieClip`
class that does the right thing. The replacement runtime IS the optimization.

### Library boundary detection

The frontend is responsible for identifying library boundaries. For Flash,
the `flash.*::` namespace cleanly separates stdlib from user code. The
frontend marks these references in the IR so downstream passes know which
calls cross the boundary.

What the frontend produces:

```
IR instruction:   GetField "flash.text::TextFormatAlign"
IR metadata:      external_lib = "flash.text", short_name = "TextFormatAlign"
```

The backend consumes the metadata to emit imports. It never parses namespace
strings — that's the frontend's domain knowledge.

### Replacement runtime as a swappable package

Each frontend ships one or more **replacement runtime packages** — standalone
libraries that implement the original runtime's API surface in the target
language. These are separate from both the frontend and the backend:

```
reincarnate-frontend-flash/
  runtime/                    ← Flash replacement runtime (TypeScript)
    flash-display/
    flash-events/
    flash-text/
    ...

reincarnate-frontend-director/
  runtime/                    ← Director replacement runtime
    lingo-builtins/
    ...
```

Key properties:
- **Swappable** — You can have multiple implementations at different fidelity
  levels: a minimal stub runtime for testing, a full-fidelity runtime for
  production, an optimized native runtime that skips Flash semantics entirely
- **Granular** — Replace `flash.display` with a Canvas2D implementation but
  keep `flash.text` as a DOM-based implementation. Mix and match per package.
- **Backend-agnostic** — The TypeScript replacement runtime is `.ts` files;
  a Rust replacement runtime would be `.rs` files with the same API surface.
  The backend copies/links whichever runtime matches its target language.

### Optimization through replacement

The biggest performance wins come from replacing library implementations, not
from optimizing user code. Examples:

| Original | Stub (correctness) | Optimized (native) |
|----------|-------------------|-------------------|
| `flash.display` (display list) | JS class hierarchy | Canvas2D direct draw, skip display list |
| `flash.text.TextField` | DOM measurement | Pre-measured glyph atlas |
| `flash.events` (bubbling) | Full capture/bubble | Flat listener dispatch |
| `flash.net.SharedObject` | localStorage wrapper | IndexedDB with sync API |

A project can start with stub implementations and progressively replace
individual packages with optimized versions — same user code, better runtime.

### Pipeline with library replacement

```
Source binary
      │
      ▼
┌──────────┐
│ Frontend  │ → IR + library boundary metadata
└────┬─────┘
     │
     ▼
┌──────────┐
│ Transforms│ → optimized IR (user logic only; library calls untouched)
└────┬─────┘
     │
     ▼
┌──────────┐     ┌───────────────────┐
│ Backend   │ ←── │ Replacement runtime │  (selected per frontend × target)
└────┬─────┘     └───────────────────┘
     │
     ▼
  Output (.ts/.rs) + runtime package
```

The backend's job: emit user code + wire up imports to the replacement
runtime. The frontend's job: detect library boundaries and attach metadata.
The runtime's job: make `MovieClip.gotoAndStop(3)` actually work.

## Codegen Backends

### Rust Source (primary)

Emits `.rs` files that compile with `rustc`. Benefits:
- Monomorphization eliminates all generic overhead
- Const evaluation resolves static data at compile time
- LLVM optimizations apply automatically
- Native desktop and mobile via standard Rust targets
- WASM via `rustc --target wasm32-unknown-unknown`

Desktop is truly native — no Tauri, no WebView, no embedded browser. The generated Rust code links against `wgpu` + `winit` for rendering and windowing.

### TypeScript (secondary)

Emits `.ts` files for web deployment. Useful when:
- The target is a browser game/app
- WASM binary size is a concern
- Integration with existing web infrastructure is needed

## Type Inference

Flow-sensitive, Hindley-Milner-ish type recovery. Source languages (ActionScript 3, Lingo, GML, etc.) are often dynamically typed — inference recovers concrete types where possible.

When inference fails, the type becomes `Dynamic`: a tagged union that backends emit as an enum with runtime dispatch. The goal is to minimize `Dynamic` usage — most well-typed ActionScript 3 code should infer fully.

Key properties:
- **Flow-sensitive**: types narrow through conditionals and casts
- **Constraint-based**: unification over type variables
- **Fallback**: `Dynamic` when constraints are unsatisfiable or insufficient
- **Per-function**: inference runs per function, cross-function via signatures

## System Architecture

Pluggable systems via generic traits that monomorphize at compile time:

```rust
trait Renderer {
    type Texture;
    type Surface;
    fn draw_sprite(&mut self, texture: &Self::Texture, x: f32, y: f32);
    // ...
}
```

Systems are generic parameters on the game/app entry point. The compiler monomorphizes each combination — zero virtual dispatch at runtime. Systems can be swapped (e.g., inject touch controls for mobile, replace save/load UX, add accessibility overlays) without changing the core translated code.

### System Traits

| System | Responsibility |
|--------|---------------|
| **Renderer** | Sprite/shape/text drawing, display list |
| **Audio** | Sound/music playback, mixing |
| **Input** | Keyboard, mouse, touch, gamepad |
| **SaveLoad** | Persistence (save files, local storage) |
| **UI** | Dialogue boxes, menus, HUD overlays |
| **Timing** | Frame pacing, delta time, timers |

## IR Design

### Entity-Based Arenas

All IR nodes live in typed arenas (`PrimaryMap<K, V>`) and are referenced by `u32` indices:

```
FuncId(0) → Function { name: "main", blocks: [BlockId(0), BlockId(1)] }
BlockId(0) → Block { params: [ValueId(0)], insts: [InstId(0), InstId(1)] }
InstId(0) → Inst { op: Op::Const(42), result: ValueId(1) }
```

Cache-friendly, trivially serializable, no lifetimes or `Rc`/`Arc`.

### Block Arguments (not Phi Nodes)

Following Cranelift and MLIR, the IR uses block arguments instead of phi nodes. Simpler to construct from frontends, easier to reason about.

```
block0(v0: i32):
    v1 = add v0, 1
    br block1(v1)

block1(v2: i32):
    return v2
```

### Operations

The `Op` enum covers:
- **Constants**: integers, floats, strings, booleans, null
- **Arithmetic/logic**: standard ALU operations
- **Control flow**: branch, conditional branch, switch, return
- **Memory**: load, store, alloc, field access
- **Calls**: direct call, indirect call, system call
- **Type operations**: cast, type check, dynamic dispatch
- **Coroutines**: yield, create, resume
- **Aggregates**: struct/array/tuple construction and access

### SystemCall

```
SystemCall { system: "Renderer", method: "draw_sprite", args: [v0, v1, v2] }
```

String-based at IR level, resolved to concrete trait method calls at codegen. Keeps the `Op` enum from exploding with per-engine operations.

### Coroutines as First-Class

`Yield`, `CoroutineCreate`, `CoroutineResume` are IR operations. A transform pass lowers them to state machines before codegen. Different backends may lower differently:
- Rust: state machine enum or async/await
- TypeScript: generator functions

### Module Scope

Entity IDs are module-scoped: `FuncId(0)` in Module A ≠ `FuncId(0)` in Module B. Cross-module references use string-based imports. A linking pass builds the global symbol table.

## AST Normalization Passes

After the backend lowers IR to an AST (`Vec<Stmt>`), a pipeline of rewrite passes normalizes the output for readability. Passes run in a fixed order; some are in a fixpoint loop that iterates until no further changes occur.

### Cleanup Phase (one-shot, before fixpoint)

| Pass | Effect |
|------|--------|
| **eliminate_self_assigns** | Remove `x = x` |
| **eliminate_duplicate_assigns** | Collapse consecutive identical assignments |
| **eliminate_forwarding_stubs** | Remove uninit phi + immediate read into another phi |
| **invert_empty_then** | `if (x) {} else { B }` → `if (!x) { B }` |
| **eliminate_unreachable_after_exit** | Truncate dead code after return/break/continue or if-else where both branches exit |
| **rewrite_ternary** | `if (c) { x = a } else { x = b }` → `x = c ? a : b` |
| **rewrite_minmax** | `(a >= b) ? a : b` → `Math.max(a, b)` |

### Fixpoint Phase

Runs in a loop until statement count stabilizes:

| Pass | Effect |
|------|--------|
| **forward_substitute** | Inline single-use adjacent assignments |
| **rewrite_ternary** | (re-run to catch newly exposed patterns) |
| **simplify_ternary_to_logical** | `c ? x : c` → `c && x`, `c ? c : x` → `c \|\| x` |
| **absorb_phi_condition** | Merge split-path phi booleans into their assigning branch |
| **narrow_var_scope** | Push uninit `let` into the single child scope where all refs live |
| **merge_decl_init** | `let x; ... x = v` → `let x = v` |
| **fold_single_use_consts** | Inline single-use const/let declarations |

### Final Phase (one-shot, after fixpoint)

| Pass | Effect |
|------|--------|
| **rewrite_compound_assign** | `x = x + 1` → `x += 1` |
| **rewrite_post_increment** | Read-modify-write → `x++` |

## Multi-Platform Strategy

| Platform | Rendering | Windowing | Audio |
|----------|-----------|-----------|-------|
| Desktop (Linux/macOS/Windows) | wgpu | winit | TBD |
| Mobile (iOS/Android) | wgpu | winit | TBD |
| WASM | wgpu (WebGPU) | winit (canvas) | Web Audio API |
| Web (TypeScript backend) | Canvas/WebGL | DOM | Web Audio API |

Platform differences are abstracted at the system trait level. The same IR compiles to any target by swapping system implementations.

## Dependencies

- **`swf` crate** (from Ruffle, MIT/Apache-2.0): SWF file parsing for the Flash frontend
- **`thiserror`**: Structured error types in core
- **`serde` / `serde_json`**: Project manifest and asset catalog serialization
