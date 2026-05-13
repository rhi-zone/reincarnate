# CLAUDE.md

## Goal

Reincarnate translates games from any source engine into working, type-safe, high-quality code on any target platform — for preservation, migration, and cross-platform deployment. The emitted code is the artifact — it must be indistinguishable from a skilled human port. The bar is not "compiles" or "runs."

**Never suggest bundling an existing interpreter.** inkjs, Parchment, renpyweb, libqsp-WASM are "quick deploy" alternatives — not the goal.

**Emitted code is measured against handwritten code.** Runtime name lookup where a direct call is possible, `unknown` where a concrete type is inferrable, or any other indirection a human would never write are all defects. Closing the gap is always higher priority than adding new features.

## Quality

**TS error counts are not a goal.** Any change that reduces errors by widening a type, guessing at correct behavior, or silencing a diagnostic is a regression. The only valid reason to make a change is that it is correct.

**`Type::Unknown` is an inference failure, not a legitimate type.** Every value has a concrete source-language type. Unknown means inference wasn't good enough. Suppressing Unknown at emit time, or propagating types by guessing from downstream uses, are both monkeypatches.

**Fix the real problem.** A correct fix changes the model so the case can't arise. A branch that compensates for upstream failures is a monkeypatch — fix the upstream failure instead. Document blocked fixes in TODO.md and leave the code unchanged until unblocked.

**Pivoting when a problem is hard is a copout.** When an approach fails, understand exactly why and fix the underlying cause — don't suggest switching to a different bucket because this one is difficult. "Let's do X instead" after repeated failures on Y is not a plan, it's avoidance. The only legitimate reasons to defer a problem are: (a) it is genuinely blocked on an external decision, or (b) it has been fully diagnosed and documented as requiring prerequisite work that isn't yet done. "We tried three times and it's hard" is neither.

**Tech debt is never an acceptable tradeoff for easier implementation.** A workaround that avoids touching more files, breaking more callers, or requiring more refactoring is still a workaround. Do the right thing — rename, update all callers, restructure. The cost of carrying debt always exceeds the cost of paying it immediately. If a solution is tech debt, do not list it as an option — apply the constraint before generating options, not after.

**Known gaps live in TODO.md.** Every gap, unverified assumption, and unimplemented behavior must be tracked there. Not adding a TODO entry is an implicit claim of correctness.

**Read before modifying or proposing.** Confidence is not a feeling — it is a result of having verified. When a request is ambiguous, state your interpretation and wait for confirmation. Design proposals require the same standard: read the relevant code before making claims about how things work. Reasoning from first principles when the implementation is readable is not a substitute for reading it.

**Wrong code causes cascading damage.** Wasted time, risky reverts, corrupted `git blame`, misdirected future work.

**Conversation is not memory.** Anything said in chat evaporates at session end. Behavioral changes go in CLAUDE.md immediately.

**Corrections are documentation lag, not model failure.** When the same mistake recurs, the fix is writing the invariant down — not repeating the correction. Exception: during active design, corrections are the work itself — don't prematurely document a design that hasn't settled yet.

## Fundamental Laws

Invariant. When a violation appears, adjust the law — don't add a corollary.

**1. Pipeline Stage Isolation.** The IR is the only channel between pipeline stages. Everything a backend needs must be in the IR — extend it rather than route around it.

**2. Engine Specificity at Boundaries.** Frontends know the source engine. Backends know the target language. Core knows neither — not GML, not TypeScript. Engine-specific logic in core is in the wrong place. This includes named engine functions hardcoded in transforms, backward inference that compensates for engine-specific gaps, any logic whose behavior changes based on which engine produced the IR, and any logic that encodes target-language assumptions (e.g. "Int and Float are both `number`"). The IR itself is subject to the same rule: IR structs and op variants must not carry source-engine or target-language knowledge — no emit hints, no operator syntax, no calling conventions, no native function names. **`BinOp`/`UnaryOp` enums must not exist in core** — operator semantics differ across backends (Lua `//`, Rust `>>`, TypeScript `>>>`, etc.). Arithmetic and bitwise operations are represented as `Op::Call` to builtin FuncIds; each backend dispatches on the function name to emit its native operator syntax. All planned frontends are documented in `docs/targets/`: GameMaker (GMS1/GMS2), Flash/AS3, Twine (SugarCube, Harlowe), Ren'Py, RPG Maker, Inform, Ink, Wolf RPG, HyperCard, Director, VB6, Java Applets, Silverlight, QSP, RAGS, SRPG Studio, PuzzleScript, TyranoBuilder, KiriKiri/KiriKiri2/KiriKiriZ, NScripter/NScripter2, Suika2, Artemis, Narrat, Construct 2/3. Planned backends: TypeScript, Love2D (Lua), Bevy (Rust), Godot, Android (Kotlin/Java). Every IR design decision must hold for all of these, not just the active pair.

**3. Behavioral Equivalence.** Emitted code produces identical observable output for any input. Preserve source-language bugs.

**4. Honest Representation.** IR types reflect source-language semantics, not VM storage format. A GML boolean is `Bool`, not `Float`. Source-level type violations surface as target-language type errors — that is correct behavior. Prohibited:
- `any` anywhere — unconditionally forbidden. In TypeScript: `unknown`. In Rust emit paths: concrete types. No exceptions for "open" objects, dynamic fields, or handwritten runtime code. The full source is available; every field is statically known.
- `(expr as any)` in the emitter — fix the IR type instead
- Backward type propagation (inferring a value's type from how it is used downstream)
- Any of the above added to reduce TS error counts

**5. Instantiability.** All mutable runtime state lives on root runtime instances. No module-level mutable variables. Multiple game instances must coexist on one page. The correct mechanism for instanced runtimes is to pass the runtime object as an explicit first parameter (`rt`) to all translated functions. An optional dead parameter elimination pass removes `rt` from functions that never use it. No special-casing in the IR — the runtime is just a typed value like any other.


## Workflow

**Batch cargo commands:**
```bash
cargo clippy --all-targets --all-features -- -D warnings && cargo test -q -- --include-ignored
```
Always pass `--include-ignored`. Edit all files first, then build once.

**Complexity ratchet:** after any change to `reincarnate-core/src/transforms/`, run `normalize ratchet check` to verify no file's cyclomatic complexity increased. A complexity increase means a monkeypatch was added — real fixes simplify the model.

`cargo run -p reincarnate-cli -- check --manifest <path>` is the replacement for `tsc`. Flags: `--filter-code TS2345`, `--filter-file foo.ts`, `--filter-message "..."`, `--examples -1`. Stdout = diagnostics; stderr = progress. Never `2>&1`.

**Implementation always goes through agents.** The main context is for coordination only — decisions, review, direction. Every edit, write, and build command belongs in an agent.

**Implementation executes designs, it doesn't make them.** Before attempting a fix, check whether the plan covers it. If you're inventing something the plan didn't specify — new parameters, new fields, new methods, new patterns — that's a design decision. Surface it and wait for confirmation before propagating.

**Agent prompts must include scope constraints.** Every agent prompt must explicitly state what the agent is and is not allowed to invent. If the agent hits a case not covered by the plan, the prompt must instruct it to stop and report back rather than solve it autonomously. Never write an agent prompt with open-ended scope like "fix any remaining issues" or "also fix the other errors you find".

**Commit after every phase.** Each commit = one logical unit of progress. Conventional commits: `type(scope): message`. Types: `feat`, `fix`, `refactor`, `docs`, `chore`, `test`.

**Session handoff:** flush TODO.md → plan mode with next tasks and blocked items only. No commands, build steps, or context summaries.

**Initiate a handoff after a significant mid-session correction.** When a correction happens after substantial wrong-path work, the wrong reasoning is still in context and keeps pulling. Writing down the invariant and starting fresh beats continuing with poisoned context — the next session loads the invariant from turn 1 before any wrong reasoning exists.

**Adversarial audits:** periodically audit for suppressions, workarounds, and silent stubs.
1. Commit-diff: `git log --oneline --since="2 weeks ago"`, batch ~60 commits per haiku agent, flag violations.
2. Conversation-log: `~/git/rhizone/normalize/target/debug/normalize sessions messages --days 14 --role assistant --limit 0`, split into ~700-line batches, flag suppression patterns.

## Constraints

- No engine-specific logic in `reincarnate-core` — no named engine functions, no engine-specific heuristics, no backward inference that compensates for engine gaps
- No backward type propagation in core transforms (inferring a value's type from how it is used downstream)
- No `any` anywhere — unconditionally forbidden in emitted TypeScript, handwritten runtime code, and Rust emit paths. No exceptions.
- No widening runtime types to match wrong emitter output — fix the inference
- No path dependencies in Cargo.toml
- No `--no-verify`
- No interactive git commands (`git rebase -i`, `git add -i`, `git add -p`) — stage by name
- No DOM data attributes as state-passing mechanism
- No `function_modules` entry without a corresponding `function_signatures` entry
- No special-casing for builtin functions — builtins are functions with FuncIds like any other function. No `BuiltinOp` enum, no prefix-based dispatch (`starts_with("builtin.")`), no separate pipeline paths for builtins vs. game-defined functions. A builtin call emits as a function call; the runtime defines the body. Name collisions are resolved at registration time (rename the game function), not by reserving a namespace prefix.

## IR Type System Architecture

The IR has two parallel type representations that serve different purposes and must not be conflated.

### `module.structs: Vec<StructDef>`

**Purpose:** Frozen, backend-facing record of struct shapes as declared by the frontend. Contains `namespace`, `visibility`, and `fields` — everything the emitter needs to output a type declaration.

**Lifecycle:** Written by the frontend (ModuleBuilder), read by the backend emitter. Never mutated by core passes.

**Consumers:** Backend emitters only. `build_own_fields` in `constraint_solve_hm.rs` reads this to seed field type maps.

### `module.types: PrimaryMap<TypeId, TypeDecl>`

**Purpose:** Mutable inference-time representation of the type graph. Contains `parent` (for inheritance chains), `inferred` flag, and `methods`. Used by all core transforms and constraint solving.

**Lifecycle:** Initialized by the frontend; enriched by passes (e.g. `ConstructorStructInfer` adds `TypeDecl::Object` entries, `GmlConstructorParent` sets `parent`). Authoritative during inference.

**Consumers:** All core passes. The backend reads it for parent-chain traversal during `build_all_fields`.

### Invariant: They are used in tandem by `constraint_solve_hm.rs`

`build_own_fields` seeds field types from `module.structs`. `build_all_fields` then walks the TypeDecl parent chain via `module.types` to merge inherited fields. Both systems must agree on struct names — a name present in one but not the other will produce wrong or missing types.

After `ModuleBuilder::build()`, enrichments made to `module.types` by later passes (e.g. setting `parent`) do **not** sync back to `StructDef` — this is intentional. `StructDef` is a snapshot; `TypeDecl` is the live graph.

### Migration path

`TypeDecl::Object` is strictly more capable than `StructDef`. The end state is one system. Migration requires:
1. Add `namespace: Vec<String>` and `visibility: Visibility` to `TypeDecl::Object`
2. Migrate all `module.structs` consumers to read from `module.types`
3. Remove `StructDef` and `module.structs`

Until that migration is complete: **never add new fields to `StructDef`** — add them to `TypeDecl::Object` instead. Never route around `module.types` by reading `module.structs` for anything that should be in the live type graph (parent chains, inferred flag, methods).

## Crate Structure

All crates use the `reincarnate-` prefix:
- `reincarnate-core` — Core types and traits
- `reincarnate-cli` — CLI binary (`reincarnate`)
- `reincarnate-frontend-flash` — Flash/SWF (in `crates/frontends/`)
- `reincarnate-frontend-gamemaker` — GML/GameMaker (in `crates/frontends/`)

## CLI

```bash
cargo run -p reincarnate-cli -- emit --manifest ~/reincarnate/<engine>/<game>/reincarnate.json
cargo run -p reincarnate-cli -- check --manifest ~/reincarnate/gamemaker/deadestate/reincarnate.json
cargo run -p reincarnate-cli -- print-ir <ir-json-file>
```

Debug flags on `emit`: `--dump-ir`, `--dump-ast`, `--dump-function <pattern>`, `--dump-ir-after <pass>`.

Subcommands: `list-functions`, `disasm`, `stress`, `inspect-runtime [--sig NAME] [--list-sigs] [--validate]`.

`check` flags: `--filter-code TS2304`, `--filter-file foo.ts`, `--filter-message "..."`, `--examples N` (samples per code; -1 = all). One run is sufficient — use these flags instead of piping to grep.

`emit` flags: `--no-emit` (run pipeline, skip file output; useful with `--dump-function`).
