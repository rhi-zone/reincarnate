# CLAUDE.md

Behavioral rules for Claude Code in this repository.

## Overview

Reincarnate is a **decompiler that produces working, type-safe, maintainable code** from legacy game binaries (Flash, Director, VB6, HyperCard, RPG Maker, GameMaker, etc.). The emitted TypeScript (or Rust) is the artifact — it compiles, runs, and is as editable as any normal codebase. See `docs/architecture.md` and [ADR 003](docs/adr/003-project-identity-and-mod-surface.md) for design rationale.

**Never suggest bundling an existing interpreter.** inkjs, Parchment, renpyweb, libqsp-WASM produce running games but not emitted code. Note them as "quick deploy" alternatives — not the goal.

**The emitted TypeScript is the mod surface.** If someone wants to mod a lifted game, they edit the output. No IR mutation API is needed for this. Lift once, edit the TypeScript, forward-port via cherry-pick if the upstream game updates.

## Fundamental Laws

These are invariant. When a violation appears, adjust the law — don't add a corollary.

**1. Pipeline Stage Isolation.** The IR is the only channel between pipeline stages. Everything a backend needs — types, constants, class registries, control flow — must be in the IR. Side channels (metadata fields, raw source blobs in `AssetCatalog`, engine-specific callbacks) mean the IR is incomplete. Extend the IR; don't route around it.

**2. Engine Specificity at Boundaries.** Engine-specific knowledge belongs only in the stage that interfaces with that engine. Frontends know the source engine. Backends know the target language. Core (IR, transforms) knows neither. If core contains logic that only one engine needs, that logic is in the wrong place.

**3. Behavioral Equivalence.** Emitted code produces identical observable output for any input. This includes preserving source-language bugs — if GML uses `|` where `||` was intended, the emitted code uses `|`. Never add special-case guards to "fix" source bugs. The only exception: if the fidelity gap stems from our type inference being wrong, fix the inference.

**4. Honest Representation.** IR types reflect source-language semantics, not VM storage format. A GML boolean is `Bool`, not `Float`. A GML object is its class type, not a numeric ID. Source-level semantic violations (wrong operator, wrong type) surface as target-language type errors — that is correct behavior. When a type error appears in emitted code, ask *why is the type wrong?* — the answer is one of: (a) game-author bug, leave it; (b) our inference is wrong, fix the inference; (c) the emitter reads a stale type (use `value_types[v]`), fix it. The distinction between "acceptable fix" and "suppression": a fix is acceptable when it makes the emitted code more faithful to source semantics (e.g. inserting `Cast(Bool→Float)` when GML treats bool as number in arithmetic — this IS the right representation); a suppression is a workaround that hides a diagnostic without improving accuracy (e.g. widening a type to `any` to silence an error that reflects a real type mismatch). When in doubt: does the change make the output more accurate, or just less noisy?

**5. Instantiability.** All mutable runtime state lives on root runtime instances threaded through generated code. No module-level mutable variables. Multiple game instances must be able to coexist on one page.

## Core Rule

**Document before acting:**
- Bugs/issues → fix or add to TODO.md
- Design decisions → docs/ or code comments
- Future work → TODO.md
- Key insights → this file

**Conversation is not memory.** If a statement implies future behavior change, write it to CLAUDE.md or a memory file immediately. A statement like "I won't do X again" made only in conversation evaporates at session end.

**Every observed problem goes to TODO.md. No exceptions.** Code comments, commit messages, and conversation are not tracked items. If you write a `// TODO` in source code, open TODO.md next.

**Any correction means update CLAUDE.md now.** Ask: what rule would have prevented this? A correction without a rule change will repeat. If corrected twice on the same topic, write a broader principle covering the entire class.

**Do the work properly.** When asked to analyze X, actually read X — don't synthesize from conversation.

## How to Think and Act

**Good tooling is a high priority.** Build tools that make doing the right thing easy. Scripts that generate boilerplate, validators that catch mismatches at startup, CLI helpers that automate repetitive steps — these pay for themselves immediately by reducing friction and preventing mistakes. `RuntimeConfig::validate()` is one example: it catches `function_modules`/`function_signatures` mismatches at startup. A script that extracts TypeScript signatures into `runtime.json` entries is another. When a task is tedious and error-prone, automate it.

These principles govern judgment. Individual rules follow from them.

- **Fix the real problem.** A workaround is any change that avoids fixing the actual cause. "Special handling in the emitter" is a workaround; "fix the pass that produces wrong output" is a fix. A narrow guard added to one case often means the pass's core logic is wrong — fix the assumption, not the symptom. Use `git blame` to check for accumulated guards; that pattern indicates a design gap. If a fix is blocked by a deeper issue, fix the deeper issue first — or document both layers in TODO.md and leave the code unchanged. The test: does the change make the system more correct, or hide that the system is wrong?
- **Effort is not an objection; design is.** The right fix touches however many files it touches. "That's a large refactor" is never a valid objection — "that's the wrong design" is. Never optimize for ease of implementation or add complexity for its own sake. The axis of quality is correctness, not simplicity or scope.
- **Friction is information.** When a fix is awkward, the awkwardness reveals a design problem. A type that's `Vec<(String, Type, Option<Constant>, bool)>` instead of a struct, or engine-specific logic in engine-agnostic code, is a bug — not a constraint to work within. Ask *what is wrong with the design that makes this hard?* Fix that, or document it.
- **Question existing code.** Code from last session is not more trustworthy than code from a stranger. Before building on an abstraction, ask whether it's correct. "It already works this way" is not evidence that it should.
- **Implement fully.** Test projects are examples, not the spec — fix the entire class, not just the case that blew up. Check all pipeline stages before closing a task. Every API method belongs in the runtime.
- **Verify before stating.** Don't assert API behavior or codebase facts without checking. Check what the original engine actually does. If no authoritative source exists, record the assumption in TODO.md.
- **Write regression tests for reproducible compiler bugs.** Assert correct externally-observable behavior — not implementation details. If the correct assertion fails, mark `#[ignore = "known bug: ..."]` and add to TODO.md; never adjust the assertion to match broken behavior.
- **Stubs must throw, not silently fail.** Implement the function or `throw Error("name: not yet implemented")` + add a TODO.md entry. Silent returns (`0`, `""`, `false`, `null`, `{}`) are always wrong.
- **Don't hand-roll what a library does.** JS identifier validity → `unicode_ident::is_xid_start`/`is_xid_continue` (plus `$`); JS string escaping → `serde_json::to_string`.
- **Before removing a mechanism, read why it was added.**

## Workflow

**Batch cargo commands** to minimize round-trips:
```bash
cargo clippy --all-targets --all-features -- -D warnings && cargo test -- --include-ignored
```
Always pass `--include-ignored`. After editing multiple files, run the full check once — not after each edit.

**When making the same change across multiple crates**, edit all files first, then build once.

**Minimize file churn.** Read once, plan all changes, apply in one pass.

**Commit after every phase.** Each commit = one logical unit of progress. No exceptions.

**Use `bun`** for JavaScript/TypeScript scripting tasks instead of `node` or `python3`.

**Use subagents to protect the main context window.** Research tasks, >5 files, >3 grep rounds → subagent. Single targeted lookup → inline is fine.

## Session Handoff

Use plan mode as a handoff when a task is complete, the session has drifted, or context is heavy. Write a short plan pointing at TODO.md and ExitPlanMode — **do not investigate first**. Update TODO.md and memory files before handing off.

## Commit Convention

Conventional commits: `type(scope): message`. Types: `feat`, `fix`, `refactor`, `docs`, `chore`, `test`.

## Negative Constraints

Do not:
- Announce actions ("I will now...") — just do them
- Add to the monolith — split by domain into sub-crates
- Use path dependencies in Cargo.toml — causes clippy to stash changes across repos
- Use `--no-verify` — fix the issue or fix the hook
- Use interactive git commands (`git rebase -i`, `git add -i`, `git add -p`) — they hang waiting for stdin. Stage files by name: `git add <file1> <file2>`.
- Use DOM data attributes as a state-passing mechanism
- **Widen runtime types to match wrong emitter output.** If emitted code passes `number` where the runtime declares `boolean`, the emitter is wrong — fix the type inference (e.g. ensure `function_signatures` has correct param types so `IntToBoolPromotion` can promote constants). Widening runtime types is a suppression that hides the real problem.
- **Add `function_modules` entries without corresponding `function_signatures` entries.** Every function registered in `function_modules` must also have param/return types in `function_signatures` so the type inference pipeline can do its job.

## Crate Structure

All crates use the `reincarnate-` prefix:
- `reincarnate-core` — Core types and traits
- `reincarnate-cli` — CLI binary (named `reincarnate`)
- `reincarnate-frontend-flash` — Flash/SWF frontend (in `crates/frontends/`)
- `reincarnate-frontend-gamemaker` — GML/GameMaker frontend (in `crates/frontends/`)
- etc.

## CLI Usage

Run via cargo from the repo root:

```bash
# Full pipeline: extract → IR → transform → emit TypeScript
cargo run -p reincarnate-cli -- emit --manifest ~/reincarnate/<engine>/<game>/reincarnate.json

# Check TypeScript output (error counts + summary):
cargo run -p reincarnate-cli -- check --manifest ~/reincarnate/gamemaker/deadestate/reincarnate.json

# Print human-readable IR:
cargo run -p reincarnate-cli -- print-ir <ir-json-file>
```

Debug flags on `emit`: `--dump-ir`, `--dump-ast`, `--dump-function <pattern>`, `--dump-ir-after <pass>`.

Additional subcommands: `list-functions`, `disasm`, `stress`.
