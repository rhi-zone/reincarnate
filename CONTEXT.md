# Ubiquitous Language — Reincarnate

Domain vocabulary that trips up new agents. Each entry disambiguates a term that has precise meaning here, different from common usage or easily confused with a sibling concept.

---

## IR (Intermediate Representation)
_Avoid:_ AST, syntax tree

SSA-like, block-argument format that is the sole channel between pipeline stages. Blocks carry typed parameters instead of phi nodes; branches pass arguments explicitly. The IR is what the pipeline stages read and write — it is not an AST and carries no source-language or target-language syntax.

Confused with "AST": the linear emitter produces an AST as an internal step inside the backend, after structurization. That AST is backend-private and never crosses a pipeline boundary.

---

## Frontend
_Avoid:_ parser, decompiler, extractor

A pipeline stage that parses engine-specific binary or script formats and emits untyped IR plus a list of `PureIrPass` instances (`frontend_passes`). Knows the source engine; must not know any target language. The `Frontend` trait is the formal boundary.

Confused with "parser": a frontend does more than parse — it allocates `FuncId`s, registers runtime stubs, and seeds the type graph — but its output is always IR, never structured source text.

---

## Backend
_Avoid:_ emitter, compiler

A pipeline stage that consumes fully typed, optimized IR and emits target-language code (currently TypeScript). Knows the target language; must not know any source engine. The `Backend` trait is the formal boundary.

Confused with "emitter": the emitter is a sub-step inside the backend (the code that walks IR and writes strings). "Backend" is the full stage from typed IR to output files, including structurization and scaffolding.

---

## Transform / Transform Pipeline
_Avoid:_ "a transform" when meaning the pipeline; "the pipeline" when meaning a single pass

Two distinct concepts sharing a root word:

- **`Transform` (trait)**: a single pass — one `apply()` call consuming a `Module` and returning a new one. Examples: `Mem2Reg`, `CoroutineLowering`, `ConstraintSolveHM`.
- **`TransformPipeline`**: the ordered sequence of passes that runs between frontend and backend. Declarative: passes declare `requires()` and `invalidates()` dependencies; the pipeline topo-sorts them and inserts re-runs automatically.

Confused usage: "the transform lowered the coroutines" (means the pass); "the transform runs type inference" (means the pipeline). Use "pass" for individual transforms and "pipeline" for the sequence.

---

## PureIrPass
_Avoid:_ transform, pass (without qualification)

A marker trait that a `Transform` implements to certify it is stateless and IR-only: no filesystem, no network, no global mutable state. Required for passes injected by frontends via `FrontendOutput::frontend_passes` to enforce Law 1 (Pipeline Stage Isolation) at the type level. These run after the standard pipeline (after DCE) but before structurization.

Confused with any `Transform`: `PureIrPass` is a narrower contract. A frontend can only inject `PureIrPass` instances — not arbitrary `Transform`s — precisely to prevent engine-specific I/O from leaking into core.

---

## Mem2Reg
_Avoid:_ variable promotion, phi insertion

The SSA construction pass that promotes `Op::Alloc` / `Op::Store` / `Op::Load` chains into direct value references. Runs in two sub-passes: single-store promotion (fast path) and multi-store SSA via Cytron-style dominance-frontier phi placement. Invalidates `constraint-solve-hm` and `constant-folding`, triggering re-runs of those passes after it completes.

Confused with "variable promotion" in a general sense: Mem2Reg specifically eliminates stack-like allocation patterns that frontends emit when they cannot directly determine SSA form. It does not handle heap allocation or object fields.

---

## TypeVar / Type::Var
_Avoid:_ `Type::Unknown`, inference variable, placeholder

`Type::Var(TypeVarId)` is an unresolved type variable allocated during constraint collection and resolved by the HM solver. It is a pre-inference placeholder — normal and expected in IR before `constraint-solve-hm` runs.

`Type::Unknown` has two distinct uses:
- **Legitimate pre-inference**: `Constant::Null` types as `Type::Option(Type::Unknown)` to represent "nullable, type not yet known." Type coercion fallbacks also return Unknown when numeric coercion cannot determine the type, before constraint solving.
- **Post-inference defect**: Unknown remaining after `constraint-solve-hm` represents an inference gap. Every such Unknown in output IR is a defect — inference was not good enough.

Confusing them causes wrong fixes: suppressing post-inference `Unknown` at emit time or widening types to avoid it are monkeypatches. The correct fix is improving inference so the `TypeVar` resolves to a concrete type. But pre-inference `Unknown` in `Constant::Null` is correct and should not be "fixed."

---

## Constraint Solving / HM Unification
_Avoid:_ type inference pass, type propagation

The `ConstraintSolveHM` pass implements Hindley-Milner constraint solving. It allocates one `TypeVarId` per value, emits `TypeConstraint` (Equal, Subtype, HasField, Callable, HasIndex), and resolves them in a worklist fixpoint loop. Operates interprocedurally — constraints cross function boundaries via caller/callee linking.

Confused with "type propagation": backward type propagation (inferring a value's type from how it is used downstream) is explicitly forbidden. The solver propagates constraints forward from definitions, not backward from uses.

---

## Coroutine Lowering
_Avoid:_ async/await lowering, generator lowering

The `CoroutineLowering` pass rewrites functions containing `Op::Yield` into state-machine resume functions. Splits blocks at yield points, allocates a state index per yield, and rewrites callers to drive the state machine. The IR type `Type::Coroutine { yield_ty, return_ty }` becomes a concrete struct after lowering.

Confused with "async/await": the IR uses `Op::Yield` uniformly for all generator-like constructs regardless of source language. Coroutine lowering is target-agnostic; the backend decides how to represent the state machine in the target language.

---

## Behavioral Equivalence
_Avoid:_ correctness, fidelity, compatibility

The core output invariant (Law 3): emitted code produces identical observable output to the source for any input, including preserving source-language bugs. "Identical" means same external behavior — not same internal structure.

Confused with "correctness": correct TypeScript that produces different output from the source game is a violation. A bug faithfully reproduced is a success. Fixing game bugs during translation is out of scope.

---

## HLE (High-Level Emulation)
_Avoid:_ instruction-by-instruction emulation, interpreter bundling

The strategy of detecting engine library boundaries and replacing the original runtime with a native implementation, rather than emulating the original runtime instruction-by-instruction. User logic is recompiled; the runtime is swapped. This is the opposite of bundling a WASM runtime (e.g. inkjs, Parchment).

Confused with full emulation: HLE does not emulate — it replaces. The original runtime disappears; translated code calls into a purpose-built replacement that has the same API contract but runs natively.

---

## Module
_Avoid:_ file, namespace, package

The compilation unit: a `Module` holds `functions`, `types` (the live type graph), `structs` (the frozen snapshot), `runtime_registry`, global variables, and the entry point. One `Module` per source file or logical translation unit. The pipeline operates on `Vec<Module>` for multi-file projects; the linker resolves cross-module references.

Confused with "file": a module is a typed IR artifact, not a source file. A single source `.swf` may produce one module; a GameMaker project may produce several.

---

## RuntimeRegistry (`module.runtime_registry`)
_Avoid:_ stdlib, builtin table, function table

The `HashMap<String, FuncId>` in `Module` that maps stdlib and engine API function names to their `FuncId` entries. Registered via `Module::register_runtime`. These are IR functions with typed signatures — they participate in inlining, overload selection, and constraint solving just like user-defined functions. The backend recognizes them to avoid emitting their IR bodies as game code.

Confused with "stdlib": the registry is a mechanism, not a specific set of functions. Each frontend populates it with its engine's API surface. Core builtins (arithmetic, etc.) are registered separately via `register_core_builtins` and tracked in `core_builtin_fids`.

---

## StructDef / `module.structs`
_Avoid:_ type declaration, TypeDecl

The frozen, backend-facing snapshot of struct shapes as declared by the frontend. Written by `ModuleBuilder`, read by the backend emitter for type declarations. Never mutated by core passes after construction.

Confused with `TypeDecl` / `module.types`: `module.types` is the live inference-time type graph, enriched by passes (e.g. `ConstructorStructInfer` adds fields, `GmlConstructorParent` sets `parent`). The two must agree on struct names. Until the planned migration is complete, new fields go on `TypeDecl::Object` — never on `StructDef`.

---

## Structurization
_Avoid:_ lowering, code generation, emit

The step between the transform pipeline and the backend emitter that recovers structured control flow (`if`/`else`, `while`, loops) from the block-based CFG. Produces a `Shape` tree. Runs inside the backend, after all transforms — it is not a `Transform` pass and does not modify the IR.

Confused with "lowering" or "codegen": structurization is purely control-flow analysis. It reads blocks and terminators; it does not emit strings. The actual code string emission happens after structurization, in the emitter.

---

## Law 2 (Engine Specificity at Boundaries)
_Avoid:_ "keep it generic", "no hardcoding"

The invariant that frontends know the source engine, backends know the target language, and `reincarnate-core` knows neither. Violations include: named engine functions hardcoded in transforms, `BinOp`/`UnaryOp` enums in core (operator semantics differ per backend), IR fields carrying emit hints or calling conventions, and backward inference that compensates for engine-specific gaps. Arithmetic and bitwise operations are `Op::Call` to builtin `FuncId`s; backends dispatch on the function name to emit native operator syntax.

Confused with "avoid magic strings": the constraint is structural, not stylistic. Even a well-named constant for a GML function name in a core transform violates Law 2.
