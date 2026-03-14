# CLAUDE.md

## Goal

Reincarnate is a decompiler that produces working, type-safe, maintainable code from legacy game binaries. The emitted code (TypeScript, Rust, etc.) is the artifact — it compiles, runs, and is as editable as any normal codebase.

**Design target:** arbitrary source languages, arbitrary target languages, zero coupling between them, state-of-the-art type inference and static analysis, high-quality codebase architecture. Every decision is evaluated against this bar.

**Never suggest bundling an existing interpreter.** inkjs, Parchment, renpyweb, libqsp-WASM produce running games but not emitted code. Note them as "quick deploy" alternatives — not the goal.

**The emitted code is the mod surface.** Lift once, edit the output, forward-port via cherry-pick if the upstream game updates. No IR mutation API needed.

## Quality

**Sloppiness is not excusable.** There is no pressure, deadline, or metric that justifies a sloppy fix. The most common form of sloppiness in this codebase is treating TypeScript error counts as a goal — they are not. A change that reduces errors by widening a type, silencing a diagnostic, or guessing at correct behavior is a regression. The only valid reason to make a change is that it is correct.

**The delta between "compiles" and "correct" lives in TODO.md.** Every known gap, unverified assumption, silent limitation, and unimplemented behavior must be tracked there. Not adding a TODO entry is an implicit claim of correctness. Growing TODO.md as scope grows is fine; gaps missing from TODO.md are not.

**Implement fully or throw — never stub silently.** `throw Error("name: not yet implemented")` is always correct when a function isn't implemented. Silent returns (`0`, `""`, `false`, `null`, `{}`) hide missing functionality. If a function needs design work first, add it to TODO.md and throw.

**Look up the spec; don't guess from call sites.** GML: docs.yoyogames.com (source: github.com/YoYoGames/GameMaker-Manual). Flash: AS3 API reference. A decompiled call site may be wrong; the spec is authoritative.

**Fix the real problem.** A workaround avoids fixing the actual cause. Narrow guards on symptoms indicate wrong core logic. If a fix is blocked by a deeper issue, fix the deeper issue first — or document both in TODO.md and leave the code unchanged.

**Conversation is not memory.** Write behavior changes to CLAUDE.md or a memory file immediately. A statement made only in conversation evaporates at session end. Any correction → update CLAUDE.md now.

**Good tooling is a high priority.** When a task is tedious and error-prone, automate it.

## Fundamental Laws

These are invariant. When a violation appears, adjust the law — don't add a corollary.

**1. Pipeline Stage Isolation.** The IR is the only channel between pipeline stages. Everything a backend needs must be in the IR. Side channels mean the IR is incomplete — extend it; don't route around it.

**2. Engine Specificity at Boundaries.** Frontends know the source engine. Backends know the target language. Core (IR, transforms) knows neither. Engine-specific logic in core is in the wrong place.

**3. Behavioral Equivalence.** Emitted code produces identical observable output for any input. Preserve source-language bugs. Never add guards to "fix" source bugs — fix the inference instead.

**4. Honest Representation.** IR types reflect source-language semantics, not VM storage format. A GML boolean is `Bool`, not `Float`. Source-level type violations surface as target-language type errors — that is correct behavior.

- **`Dynamic` is a type inference failure.** Every value has a concrete type. `Dynamic` in the IR means inference wasn't good enough. `any` in emitted TypeScript or runtime code is never acceptable — use specific types, `unknown`, union types, or generics.
- **Preserve integer vs float distinction.** Use `"int"` for indices, IDs, counts, flags, enum values; `"number"` for continuous values (coordinates, scales, angles). Matters for non-TS backends and the type checker.

**5. Instantiability.** All mutable runtime state lives on root runtime instances. No module-level mutable variables. Multiple game instances must coexist on one page.

## Workflow

**Batch cargo commands:**
```bash
cargo clippy --all-targets --all-features -- -D warnings && cargo test -- --include-ignored
```
Always pass `--include-ignored`. Edit all files first, then build once.

**Commit after every phase.** Each commit = one logical unit of progress. Conventional commits: `type(scope): message`. Types: `feat`, `fix`, `refactor`, `docs`, `chore`, `test`.

**Use `bun`** for JavaScript/TypeScript scripting tasks.

**Use subagents** for research tasks, >5 files, or >3 grep rounds.

**Session handoff:** plan mode → short plan pointing at TODO.md → update memory files → ExitPlanMode.

**Adversarial audits:** periodically audit for suppressions, workarounds, and silent stubs.
1. Commit-diff: `git log --oneline --since="2 weeks ago"`, batch ~60 commits per haiku agent, flag violations.
2. Conversation-log: `~/git/rhizone/normalize/target/debug/normalize sessions messages --days 14 --role assistant --limit 0`, split into ~700-line batches, flag suppression patterns.

## Constraints

- No engine-specific logic in `reincarnate-core`
- No path dependencies in Cargo.toml
- No `--no-verify`
- No interactive git commands (`git rebase -i`, `git add -i`, `git add -p`) — stage by name
- No DOM data attributes as state-passing mechanism
- No `function_modules` entry without a corresponding `function_signatures` entry
- No widening runtime types to match wrong emitter output — fix the inference
- No `any` in emitted TypeScript or runtime code

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

Subcommands: `list-functions`, `disasm`, `stress`.
