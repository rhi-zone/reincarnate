# CLAUDE.md

## Goal

Reincarnate translates games from any source engine into working, type-safe, high-quality code on any target platform — for preservation, migration, and cross-platform deployment. The emitted code is the artifact — it must be indistinguishable from a skilled human port. The bar is not "compiles" or "runs."

**Never suggest bundling an existing interpreter.** inkjs, Parchment, renpyweb, libqsp-WASM are "quick deploy" alternatives — not the goal.

**Emitted code is measured against handwritten code.** Runtime name lookup where a direct call is possible, `unknown` where a concrete type is inferrable, or any other indirection a human would never write are all defects. Closing the gap is always higher priority than adding new features.

## Quality

**TS error counts are not a goal.** Any change that reduces errors by widening a type, guessing at correct behavior, or silencing a diagnostic is a regression. The only valid reason to make a change is that it is correct.

**`Type::Unknown` is an inference failure, not a legitimate type.** Every value has a concrete source-language type. Unknown means inference wasn't good enough. Suppressing Unknown at emit time, or propagating types by guessing from downstream uses, are both monkeypatches. When inference is genuinely blocked, leave Unknown in the IR, fail the build with a diagnostic pointing at the source location, and file a TODO entry. Do not emit.

**Fix the real problem.** A correct fix changes the model so the case can't arise. A branch that compensates for upstream failures is a monkeypatch — fix the upstream failure instead. Document blocked fixes in TODO.md and leave the code unchanged until unblocked.

**Pivoting when a problem is hard is a copout.** When an approach fails, understand exactly why and fix the underlying cause — don't suggest switching to a different bucket because this one is difficult. "Let's do X instead" after repeated failures on Y is not a plan, it's avoidance. The only legitimate reasons to defer a problem are: (a) it is genuinely blocked on an external decision, or (b) it has been fully diagnosed and documented as requiring prerequisite work that isn't yet done. "We tried three times and it's hard" is neither.

**Tech debt is never an acceptable tradeoff for easier implementation.** A workaround that avoids touching more files, breaking more callers, or requiring more refactoring is still a workaround. Do the right thing — rename, update all callers, restructure. The cost of carrying debt always exceeds the cost of paying it immediately. If a solution is tech debt, do not list it as an option — apply the constraint before generating options, not after.

**Known gaps live in TODO.md.** Every gap, unverified assumption, and unimplemented behavior must be tracked there. Not adding a TODO entry is an implicit claim of correctness.

**Multi-step reasoning belongs in subagents.** When checking whether a proposed change is correct requires non-trivial reasoning (e.g. does this violate Law 2? is this expressible in IR?), do the reasoning in a subagent. Wrong reasoning in main context poisons the session; wrong reasoning in a subagent contaminates only that subagent's context. Only the conclusion returns to main.

**Apply invariants before generating options, not after.** When a gap is identified (missing IR constant, missing op, missing type), reason through all applicable laws before proposing anything. A proposal that violates Law 2 is not a proposal — it is context poisoning: once stated, it pulls subsequent reasoning toward the wrong solution even after retraction. The correct sequence is: identify the constraint → rule out illegal options → propose only what remains. Proposing something and then immediately retracting it when reminded of an invariant is not a correction — it is evidence that the invariant was not consulted. Backend primitives are correct when they represent genuinely target-language-specific operations (runtime type reflection, platform APIs, language-specific sentinels). The reflex to eliminate backend primitives is wrong when the operation is inherently language-specific — `is_undefined_rt` is correct; `f64::INFINITY` in IR is correct because IEEE 754 is universal.

**Wrong code causes cascading damage.** Wasted time, risky reverts, corrupted `git blame`, misdirected future work.

## Fundamental Laws

Invariant. When a violation appears, adjust the law — don't add a corollary.

**1. Pipeline Stage Isolation.** The IR is the only channel between pipeline stages. Everything a backend needs must be in the IR — extend it rather than route around it.

**2. Engine Specificity at Boundaries.** Frontends know the source engine. Backends know the target language. Core knows neither — not GML, not TypeScript. Engine-specific logic in core is in the wrong place. This includes named engine functions hardcoded in transforms, backward inference that compensates for engine-specific gaps, any logic whose behavior changes based on which engine produced the IR, and any logic that encodes target-language assumptions (e.g. "Int and Float are both `number`"). The IR itself is subject to the same rule: IR structs and op variants must not carry source-engine or target-language knowledge — no emit hints, no operator syntax, no calling conventions, no native function names. **`BinOp`/`UnaryOp` enums must not exist in core** — operator semantics differ across backends (Lua `//`, Rust `>>`, TypeScript `>>>`, etc.). Arithmetic and bitwise operations are represented as `Op::Call` to builtin FuncIds; each backend dispatches on the function name to emit its native operator syntax. All planned frontends are documented in `docs/targets/`: GameMaker (GMS1/GMS2), Flash/AS3, Twine (SugarCube, Harlowe), Ren'Py, RPG Maker, Inform, Ink, Wolf RPG, HyperCard, Director, VB6, Java Applets, Silverlight, QSP, RAGS, SRPG Studio, PuzzleScript, TyranoBuilder, KiriKiri/KiriKiri2/KiriKiriZ, NScripter/NScripter2, Suika2, Artemis, Narrat, Construct 2/3. Planned backends: TypeScript, Love2D (Lua), Bevy (Rust), Godot, Android (Kotlin/Java). Every IR design decision must hold for all of these, not just the active pair.

**3. Behavioral Equivalence.** Emitted code produces identical observable output for any input. Preserve source-language bugs.

**4. Honest Representation.** IR types reflect source-language semantics, not VM storage format. A GML boolean is `Bool`, not `Float`. Source-level type violations surface as target-language type errors — that is correct behavior. Prohibited:
- Type escape of any kind. Every value has a source-language type; every field is statically known. If something can't be typed, inference is wrong — fix the model. There is no situation where a cast, suppression, widening, or type-system workaround is correct.
- Backward type propagation (inferring a value's type from how it is used downstream)

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

## Hard Constraints

- No engine-specific logic in `reincarnate-core` — no named engine functions, no engine-specific heuristics, no backward inference that compensates for engine gaps
- No backward type propagation in core transforms (inferring a value's type from how it is used downstream)
- No widening runtime types to match wrong emitter output — fix the inference
- No path dependencies in Cargo.toml
- No `--no-verify`
- No interactive git commands (`git rebase -i`, `git add -i`, `git add -p`) — stage by name
- No DOM data attributes as state-passing mechanism
- No `function_modules` entry without a corresponding `function_signatures` entry
- No special-casing for builtin functions — builtins are functions with FuncIds like any other function. No `BuiltinOp` enum, no prefix-based dispatch (`starts_with("builtin.")`), no separate pipeline paths for builtins vs. game-defined functions. A builtin call emits as a function call; the runtime defines the body. Name collisions are resolved at registration time (rename the game function), not by reserving a namespace prefix.
- Runtime library bodies are expressed in IR via `attach_runtime_body` in `runtime_bodies.rs` — not raw `FunctionBuilder` assembly (wrong abstraction level) and not source-language implementations (no IR primitive access, IR is a moving target). The M-frontends × N-backends problem is why: each frontend defines its runtime library in IR once; each backend emits it — avoiding M×N reimplementations. Functions that cannot be expressed in IR (e.g. platform APIs) are backend primitives: each backend emits them natively.

## IR Type System Architecture

### `module.types: PrimaryMap<TypeId, TypeDecl>`

The single authoritative type representation. `TypeDecl::Object` carries `name`, `namespace`, `visibility`, `parent`, `fields`, `methods`, `class_ref`, and `inferred`.

**Lifecycle:** Written by `ModuleBuilder::add_struct()` and `intern_type()`. Enriched by core passes (`ConstructorStructInfer` adds inferred entries, `GmlConstructorParent` sets `parent`). Read by all passes and backend emitters.

**Key invariants:**
- Instance-side entries are keyed by plain name in `module.type_names` (e.g. `"Foo"` → TypeId).
- Static-side (classref) entries are keyed by `"classref::Foo"` in `module.type_names`. Both have `name: Some("Foo")`.
- `ClassDef.type_id` points to the instance-side TypeId. Pure structs are TypeDecl::Object entries whose TypeId does not appear as any `ClassDef.type_id`.
- `build_own_fields` in `constraint_solve_hm.rs` reads fields from `module.types`. `build_all_fields` walks the `parent` chain via `module.types`.

`StructDef` is retained as a parameter type for `ModuleBuilder::add_struct()` for frontend convenience — it is not stored on `Module`. Frontends call `add_struct(def: StructDef) -> TypeId` which interns the name and writes namespace, visibility, and fields into `module.types`.

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

<!-- BEGIN ECOSYSTEM RULES -->

## Ecosystem Design Principles

Cross-cutting principles distilled from the ecosystem's own decisions (synthesized in `docs/decisions/throughlines.md`). Apply them when building new repos and recording decisions. (Already-encoded principles — independent-tools / no-path-deps, the delegation model, CLAUDE.md-as-control-surface — live in their own sections and are not repeated here.)

- **Prefer data over code at every seam.** Serializable AST / struct / JSON over closures, embedded DSLs, or source text — so artifacts cache, replay, transport, and diff.
- **Library-first; projection-from-one-definition.** The typed library is the source of truth; CLI / HTTP / MCP / WebSocket / JSON surfaces are generated projections, never hand-rolled per surface.
- **Capability security.** Hosts grant pre-opened handles; code only attenuates what it is given; nothing forges authority; allow-list over deny-list.
- **The LLM is an oracle at the leaves, never the control loop.** Determinism is a hard invariant: seeded RNG, event-log replay, build-time-only inference. Per-query LLM in the hot loop is a defect.
- **Trust comes from verifiable evidence, not authority.** Verbatim snippets, pinned-commit permalinks, claim→node citation — never a bare reference.
- **Retire, don't deprecate; collapse asymmetries to primitives.** Remove backward-compat aliases rather than carry them; reduce N special cases to their irreducible primitives.
- **Validate against reality; tests are the spec.** Load-bearing substrates are validated against real corpora; fixtures and tests define correctness, not aspirational specs.

## Hard Constraints

- No `--no-verify`. Fix the issue or fix the hook.
- No path dependencies in `Cargo.toml` — they couple repos and break independent publishing.
- No interactive git (no `git rebase -i`, no `git add -i`, no `--no-edit` on rebase).
- No suggesting project names. LLMs are bad at this; refine the conceptual space only.
- No tracking cross-project issues in conversation — they go in TODO.md in the affected repo.
- No ecosystem changes without checking all affected repos.
- No assuming a tool is missing without checking `nix develop`.
- Commit completed work in the same turn it finishes. Uncommitted work is lost work.

## Meta

- Something unexpected is a signal. Stop and find out why. Do not accept the anomaly and proceed.
- Corrections from the user are conversation, not material for new rules. Rules are added when a failure mode is observed repeatedly.

<!-- END ECOSYSTEM RULES -->
