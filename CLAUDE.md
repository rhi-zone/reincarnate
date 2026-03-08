# CLAUDE.md

Behavioral rules for Claude Code in this repository.

## Overview

Reincarnate is a legacy software lifting framework. It extracts and transforms applications from obsolete runtimes (Flash, Director, VB6, HyperCard, RPG Maker, etc.) into modern web-based equivalents.

### Goals

- **High-performance, maintainable emitted code** — The output is compiled TypeScript (or Rust) functions, not a bundled interpreter. Shipped output should be readable, optimisable, and auditable.
- **Multiple backends** — The IR + transform pipeline is backend-agnostic. TypeScript is the current backend; Rust codegen is planned. Design choices that couple output to a single runtime target are wrong.

**Corollary: type inference belongs in the IR, not in a backend.** Inference happens in the frontend, IR transform passes, or a dedicated pass — never inside the TypeScript printer. A backend may omit annotations when the target language can infer contextually, but `FunctionSig`/`Type` in the IR is the source of truth. Backends read from it; they don't create it.

**Corollary: never suggest bundling an existing interpreter.** inkjs, Parchment, renpyweb, libqsp-WASM produce running games but not emitted code. Note them as "quick deploy" alternatives — not the goal.

## Core Rule

**Note things down immediately — before writing code, not after:**
- Bugs/issues → fix or add to TODO.md
- Design decisions → docs/ or code comments
- Future work → TODO.md
- Key insights → this file

**Triggers:** User corrects you, 2+ failed attempts, "aha" moment, framework quirk discovered → document **before** proceeding. Never describe a bug in conversation and move on — write it to TODO.md first, then continue working.

**Conversation is not memory.** Anything said in conversation — a promise, a correction, a resolution — evaporates at the end of the session. If a statement implies future behavior change, it MUST be written to CLAUDE.md or a memory file immediately, or it will not happen. A statement like "I won't do X again" made only in conversation is a lie by omission: the next session has no access to it.

**Every observed problem goes to TODO.md. No exceptions.** Code comments, commit messages, and conversation are not tracked items. If you write a `// TODO` in source code, open TODO.md next. Pre-existing bugs you discover go to TODO.md as critical priority.

**Something unexpected is a signal, not noise.** Surprising results are almost always early bug evidence. Investigate before proceeding.

**Any correction or realization means update CLAUDE.md now.** When the user corrects an approach (not just a fact), or when you realize mid-session that a way of thinking was wrong — ask: what rule would have prevented this? Write it before proceeding. A correction without a new rule will repeat. If corrected twice on the same topic, the first rule was too narrow — write a broader principle covering the entire class.

**Do the work properly.** When asked to analyze X, actually read X - don't synthesize from conversation.

**Investigation findings go to TODO.md before continuing.** Root causes, affected counts, code locations, fix strategies — write them before the next step. The next session can't see what you learned in conversation.

## Behavioral Patterns

From ecosystem-wide session analysis:

- **Question scope early:** Before implementing, ask whether it belongs in this crate/module
- **Check consistency:** Look at how similar things are done elsewhere in the codebase
- **Implement fully.** Test projects are examples, not the spec — fix the entire class, not just the case that blew up. In a multi-stage pipeline, grep all stages (parser, translator, emitter) before closing a task; each encodes its own assumptions. Every API method, even ones no test game uses, belongs in the runtime.
- **Name for purpose:** Avoid names that describe one consumer
- **Verify before stating:** Don't assert API behavior or codebase facts without checking
- **Write regression tests for reproducible compiler bugs.** If a bug in a compiler pass (transforms, structurizer, AST, emit) can recur from future changes, write a test. Tests must assert the correct externally-observable behavior — not mirror the implementation. If writing the correct assertion causes it to fail, mark it `#[ignore = "known bug: ..."]` and add to TODO.md; never adjust the assertion to match broken behavior.
- **Treat special-casing as a smell:** When a fix adds a narrow guard (`if this_specific_case { continue }`) to a pass, stop and ask whether the pass's core logic is wrong. A special case that prevents one crash often means the pass's assumptions are too broad — fix the assumption, not the symptom. Use `git blame` on the file to check whether a cluster of special-case guards have accumulated around the same function; that pattern indicates a deeper design gap.
- **A type coercion is a red flag — diagnose before adding.** When emitted code has a type mismatch (TS2322, TS2362, etc.), the reflex to add a coercion (`Number()`, `Boolean()`, cast) is almost always wrong. Ask first: *why is the type wrong?* Coercions that paper over wrong IR types hide bugs and compound over time. The right fix is at the source: fix the type inference, the IR translation, or the emitter's type annotation — not the assignment site.
- **Coercions and diagnostics have different purposes.** A TypeScript error surfacing in emitted code may be: (a) a correct diagnostic of a game-author bug (leave it), (b) a sign our type inference is wrong (fix the inference), or (c) a sign the emitter is using a stale type (fix the emitter to read the authoritative `value_types`). Never decide which it is without reading the specific instances. Never suppress (a) with a coercion.
- **`value_types[v]` is the authoritative post-transform type; `BlockParam.ty` / `param.ty` may be stale.** After all IR passes, read from `value_types` in the emitter. The emitter's `collect_block_param_decls` must use `value_types[param.value]`, not `param.ty`. If transforms update only `value_types` and not `param.ty`, that is expected — emitters read the former.
- **Frontend/backend specific logic never belongs in `reincarnate-core`.** This includes language-specific coercions (e.g. GML bool-as-number), target-specific type mappings, and engine-specific idioms. If you find such logic in core, it is a bug to move, not a pattern to extend. Before adding any logic to core, ask: "which language/engine requires this?" If the answer is specific, it belongs in that frontend/backend.
- **Don't hand-roll what a library does; use the right abstraction level.** Use the high-level API by default — only drop to low-level with a concrete reason (streaming, performance). Building state machines on a tokenizer when a DOM parser exists is the same mistake as hand-rolling string parsing. Specific instances: JS identifier validity → `unicode_ident::is_xid_start`/`is_xid_continue` (plus `$`), NOT `[a-zA-Z_$][a-zA-Z0-9_$]*`; JS string escaping → `serde_json::to_string`, not `format!("\"{}\"", s)`.
- **Correctness is non-negotiable — 100%, always.** If a standard says X, implement X. Never defend a shortcut with "our inputs are ASCII-only" or "this won't come up in practice."
- **Never optimize for fewer errors.** Reduced error counts can mean wrong values replaced missing ones — verify output correctness, not just diagnostics. Never weaken types (`any`, optional params, wider unions) to silence errors. `any` means inference failed — find the missing information. Defaults must match source-language missing-arg semantics with the correct type; a `string` param defaulting to `0.0` is always wrong.
- **Verify semantics against the authoritative source.** When reimplementing runtime behavior, check what the original actually does — read the source if available (SugarCube, UndertaleModTool are open source). If no authoritative source is found, record the assumption in TODO.md. An unverified guess is indistinguishable from verified fact to the next session.
- **When something exists, it exists for a reason.** Before removing or bypassing a mechanism, read why it was added. Fix how it works; don't delete it because it causes a symptom.

## Design Principles

**Multi-turn confusion means missing tooling.** Repeated grep/hack combos and temp-file workarounds signal that the right tool doesn't exist. Add it to TODO.md — don't build it on the spot if non-trivial, but don't keep working around its absence either.

**Unify, don't multiply.** One interface for multiple engines > separate implementations per engine. Plugin systems > hardcoded switches.

**Eliminate megamorphic dispatch at compile time.** When a runtime method dispatches on a string literal (e.g. `math("round", x)`), the backend rewrite pass must resolve it to a direct monomorphic call — a JS built-in or a named function in a typed `as const` namespace (`StringOps.trimmed(s)`). A missing method is a compile-time error, not silent `undefined`. Existing namespaces: `Math` (built-in), `Collections`, `Colors`, `StringOps`.

**Compose, don't enumerate.** Enums force exactly one choice from a closed menu; adding an option requires modifying the type. Prefer a composable pipeline of independent wrappers that can be combined freely. Open composition > closed enumeration.

**Lazy extraction.** Don't parse everything upfront. Extract on demand, cache aggressively.

**Preserve fidelity, including source bugs.** The goal is accurate reproduction, not improvement. When emitted TypeScript reflects a bug in the source (e.g. `|` instead of `||`, `array_length(noone)`), that is correct behavior — don't suppress it. Don't add special-case guards in rewrite passes to "fix" source bugs; the emitted code should be wrong in exactly the same way the source was. The only exception: if the error stems from imprecise type inference on our end, fix the inference.

**Overlay > Patch.** When possible, render a modern UI layer over the original rather than patching internal rendering.

**Two-tier approach.** Accept that some targets need binary patching (Tier 1) while others can be fully lifted (Tier 2). Design APIs that work for both.

**Interface contracts are consumer-independent.** Design the minimal complete primitive set that any implementation can satisfy and any engine can build on. The consumer's needs don't drive the contract — engine-specific behaviors (named audio channels, BGM crossfade, sprite batching) belong in the shim layer, not the platform layer. An interface shaped by one consumer will be wrong for the next.

**Setup tier / hot tier split.** Separate init-time operations from per-frame operations. The game loop never sees async. Expensive work (decoding, compilation, upload) is front-loaded into init; the hot path is synchronous and allocation-free. Corollary: register strings at setup time and use opaque integer handles everywhere hot — strings are pointer-unstable across FFI and hash-compared on every lookup.

**Prefer expressive interfaces over simple ones.** When a platform interface can express more or less information, choose the more expressive form if the additional information maps to real hardware capabilities that some implementations can exploit. Simpler implementations can ignore the extra information; a less expressive interface loses it permanently and can never recover it. Example: `begin_pass` with `load_op`/`store_op` > `set_render_target` — tile-based GPU implementations need the information; WebGL2 implementations can ignore it.

**Games are instantiable — no singletons.** Multiple game instances must coexist on one page. All mutable runtime state lives on a root runtime instance (`GameRuntime`, `FlashRuntime`, etc.) threaded through generated code — never in module-level `let` variables. Do not argue "one instance is realistic"; it's a load-bearing rule (breaks hot reload, parallel testing, local multiplayer). Ask *where does this state belong?*, not *does the rule apply?*

## Runtime Architecture

Three-layer architecture: API Shims → Platform Interface (`platform/index.ts`) → Platform Implementation (`platform/browser.ts`). Swap platforms by changing the re-export in `platform/index.ts`. See `docs/architecture.md` for the full concern table and platform function list.

**Platform concern modules never import from siblings — not for cleanliness, but because cross-concern imports break the swap-by-re-export pattern.** A concern that imports from a sibling cannot be independently swapped. The fix is always shared primitive types (opaque u32 handles defined in a shared types module), never cross-concern imports. Example: `graphics_3d` takes `ImageHandle` as a plain u32 — no import from the images concern needed. Cross-concern deps that genuinely must exist are injected via callbacks (`init(showDialog, closeDialog)`), never via `import`.

**The platform interface is a cross-language contract** (TS, Rust, C#/Unity, SDL). Three hard rules:
- **Generic names only**: `play`/`stop`/`save` — never `audioPlay`/`audio_play_sound`. Engine-specific args are absorbed in the shim layer.
- **Primitives only in the API surface**: no `AudioBuffer`/`HTMLImageElement` in exported signatures — opaque handles instead.
- **Canonical names are snake_case** (TS implements as camelCase).


## Workflow

**Batch cargo commands** to minimize round-trips:
```bash
cargo clippy --all-targets --all-features -- -D warnings && cargo test -- --include-ignored
```
Always pass `--include-ignored` when running tests locally — some tests (e.g. real-game datawin tests) are gated with `#[ignore]` for CI but must pass locally. After editing multiple files, run the full check once — not after each edit. Formatting is handled automatically by the pre-commit hook (`cargo fmt`).

**When making the same change across multiple crates**, edit all files first, then build once.

**Minimize file churn.** When editing a file, read it once, plan all changes, and apply them in one pass. Avoid read-edit-build-fail-read-fix cycles by thinking through the complete change before starting.

**Commit after every phase.** Commit after each step that passes `cargo clippy && cargo test`. Each commit = one logical unit of progress. No exceptions.

**Use `bun` for JavaScript/TypeScript scripting tasks** (e.g. inspecting HTML files, running quick JS snippets). `bun` is available in the dev environment — use it instead of `node` or `python3`.

**Use `normalize view` for structural exploration:**
```bash
~/git/rhizone/normalize/target/debug/normalize view <file>    # outline with line numbers
~/git/rhizone/normalize/target/debug/normalize view <dir>     # directory structure
```

## Context Management

**Use subagents to protect the main context window.** For broad exploration or mechanical multi-file work, delegate to an Explore or general-purpose subagent rather than running searches inline. The subagent returns a distilled summary; raw tool output stays out of the main context.

Rules of thumb:
- Research tasks (investigating a question, surveying patterns) → subagent; don't pollute main context with exploratory noise
- Searching >5 files or running >3 rounds of grep/read → use a subagent
- Codebase-wide analysis (architecture, patterns, cross-file survey) → always subagent
- Mechanical work across many files (applying the same change everywhere) → parallel subagents
- Single targeted lookup (one file, one symbol) → inline is fine

## Session Handoff

Use plan mode as a handoff mechanism when:
- A task is fully complete (committed, pushed, docs updated)
- The session has drifted from its original purpose
- Context has accumulated enough that a fresh start would help

**For handoffs:** enter plan mode, write a short plan pointing at TODO.md, and ExitPlanMode. **Do NOT investigate first** — the session is context-heavy and about to be discarded. The fresh session investigates after approval.

**For mid-session planning** on a different topic: investigating inside plan mode is fine — context isn't being thrown away.

Before the handoff plan, update TODO.md and memory files with anything worth preserving.

## Commit Convention

Use conventional commits: `type(scope): message`

Types:
- `feat` - New feature
- `fix` - Bug fix
- `refactor` - Code change that neither fixes a bug nor adds a feature
- `docs` - Documentation only
- `chore` - Maintenance (deps, CI, etc.)
- `test` - Adding or updating tests

Scope is optional but recommended for multi-crate repos.

## Negative Constraints

Do not:
- Announce actions ("I will now...") - just do them
- Add to the monolith - split by domain into sub-crates
- **Stubs must throw, not silently fail.** Implement the function or `throw Error("name: not yet implemented")` + add a TODO.md entry immediately. Silent returns (`0`, `""`, `false`, `null`, `{}`) are always wrong. For platform APIs with no browser equivalent: explicit no-op with a comment (`/* no-op — PSN commerce not available in browser */`), not a silent return.
- Use path dependencies in Cargo.toml - causes clippy to stash changes across repos
- Use `--no-verify` - fix the issue or fix the hook
- Use `git add -p` or any other interactive command (`git rebase -i`, `git add -i`, etc.) — these block waiting for stdin and will hang forever in a non-interactive shell. Always stage files by name: `git add <file1> <file2>`.
- Assume tools are missing - check if `nix develop` is available for the right environment
- Use module-level mutable state — see "Games are instantiable"
- Use DOM data attributes as a state-passing mechanism — pass values through function parameters or object fields instead
- **Promote `|`/`&` to `||`/`&&` based on inferred types.** They're semantically different: `|` evaluates both operands, `||` short-circuits. TS2447/TS2363 from `boolean | boolean` are game-author errors — don't suppress them by changing the emitted operator.

## CLI Usage

Run via cargo from the repo root:

```bash
# Full pipeline: extract SWF → IR → transform → emit TypeScript
cargo run -p reincarnate-cli -- emit --manifest ~/reincarnate/flash/cc/reincarnate.json

# Print human-readable IR (for debugging)
cargo run -p reincarnate-cli -- print-ir <ir-json-file>

# Extract IR only (no transforms/emit)
cargo run -p reincarnate-cli -- extract --manifest ~/reincarnate/flash/cc/reincarnate.json

# Show project manifest info
cargo run -p reincarnate-cli -- info --manifest ~/reincarnate/flash/cc/reincarnate.json
```

The `--manifest` flag defaults to `reincarnate.json` in the current directory. Use `--skip-pass` to disable specific transform passes (e.g. `--skip-pass type-inference --skip-pass constant-folding`).

Debug flags (on `emit` only):
- `--dump-ir` — dump post-transform IR to stderr before structurization
- `--dump-ast` — dump raw AST to stderr before AST-to-AST passes
- `--dump-function <pattern>` — filter dumps to matching functions; supports bare substring, case-insensitive, and qualified names (`Gun.step`, `Gun::step`) via split-part matching
- `--dump-ir-after <pass>` — run pipeline up through the named pass, dump IR, then exit; use `frontend` to dump before any transforms

Additional subcommands:
- `list-functions [--filter <pattern>]` — list all IR function names (exact names used internally, same matching as `--dump-function`; run this first when `--dump-function` produces no output)
- `disasm [--function <filter>]` — disassemble GML bytecode directly from the DataWin (no IR pipeline); resolves variable names, strings, function names, instance types, and break signal names; without `--function`, lists all CODE entry names
- `stress [--runs N] [--skip-pass P] [--preset P]` — run the transform pipeline N times (default 5), detect fixpoint convergence or oscillation; use when adding a new pass to verify it doesn't conflict with existing passes

Test projects: see MEMORY.md for full list. Key manifests: `~/reincarnate/<engine>/<game>/reincarnate.json`.

**Checking TypeScript output:** Use `reincarnate check` for counts and summaries:
```bash
cargo run -p reincarnate-cli -- check --manifest ~/reincarnate/gamemaker/deadestate/reincarnate.json
# With baseline comparison:
cargo run -p reincarnate-cli -- check --manifest ... --baseline baseline.json
# Save a new baseline:
cargo run -p reincarnate-cli -- check --manifest ... --save-baseline baseline.json
```
Output: per-code counts (`TS2345: 222`, `TS2322: 180`, ...), per-file top 20, and total. Use `--json` for machine-readable output (includes full `diagnostics` array with file/line/message).

Filter flags: `--examples N`, `--filter-code TS2345`, `--filter-file foo.ts`, `--filter-message <text>` — compose (AND). When reporting TS error counts in TODO.md, always include total AND per-code breakdown.

## Crate Structure

All crates use the `reincarnate-` prefix:
- `reincarnate-core` - Core types and traits
- `reincarnate-cli` - CLI binary (named `reincarnate`)
- `reincarnate-frontend-flash` - Flash/SWF frontend (in `crates/frontends/`)
- `reincarnate-frontend-director` - Director/Shockwave frontend (in `crates/frontends/`)
- etc.
