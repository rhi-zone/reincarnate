# CLAUDE.md

## Goal

Reincarnate decompiles legacy game binaries into working, type-safe, high-quality code. The emitted code is the artifact — it must be indistinguishable from a skilled human port. The bar is not "compiles" or "runs."

**Never suggest bundling an existing interpreter.** inkjs, Parchment, renpyweb, libqsp-WASM are "quick deploy" alternatives — not the goal.

**Emitted code is measured against handwritten code.** Runtime name lookup where a direct call is possible, `unknown` where a concrete type is inferrable, or any other indirection a human would never write are all defects. Closing the gap is always higher priority than adding new features.

## Quality

**TS error counts are not a goal.** Any change that reduces errors by widening a type, guessing at correct behavior, or silencing a diagnostic is a regression. The only valid reason to make a change is that it is correct.

**`Type::Unknown` is an inference failure, not a legitimate type.** Every value has a concrete source-language type. Unknown means inference wasn't good enough. Suppressing Unknown at emit time, or propagating types by guessing from downstream uses, are both monkeypatches.

**Fix the real problem.** A correct fix changes the model so the case can't arise. A branch that compensates for upstream failures is a monkeypatch — fix the upstream failure instead. Document blocked fixes in TODO.md and leave the code unchanged until unblocked.

**Known gaps live in TODO.md.** Every gap, unverified assumption, and unimplemented behavior must be tracked there. Not adding a TODO entry is an implicit claim of correctness.

**Read before modifying.** Confidence is not a feeling — it is a result of having verified. When a request is ambiguous, state your interpretation and wait for confirmation.

**Wrong code causes cascading damage.** Wasted time, risky reverts, corrupted `git blame`, misdirected future work.

**Conversation is not memory.** Anything said in chat evaporates at session end. Behavioral changes go in CLAUDE.md immediately.

**A correct principle that doesn't prevent bad behavior isn't doing its job.** When a rule fails to stop a mistake, make it more specific — don't add new rules.

## Fundamental Laws

Invariant. When a violation appears, adjust the law — don't add a corollary.

**1. Pipeline Stage Isolation.** The IR is the only channel between pipeline stages. Everything a backend needs must be in the IR — extend it rather than route around it.

**2. Engine Specificity at Boundaries.** Frontends know the source engine. Backends know the target language. Core knows neither. Engine-specific logic in core is in the wrong place. This includes named engine functions hardcoded in transforms, backward inference that compensates for engine-specific gaps, and any logic whose behavior changes based on which engine produced the IR.

**3. Behavioral Equivalence.** Emitted code produces identical observable output for any input. Preserve source-language bugs.

**4. Honest Representation.** IR types reflect source-language semantics, not VM storage format. A GML boolean is `Bool`, not `Float`. Source-level type violations surface as target-language type errors — that is correct behavior. Prohibited:
- `any` in runtime type signatures, emitted TypeScript, or Rust emit paths — use `unknown`
- `(expr as any)` in the emitter — fix the IR type instead
- Backward type propagation (inferring a value's type from how it is used downstream)
- Any of the above added to reduce TS error counts

**5. Instantiability.** All mutable runtime state lives on root runtime instances. No module-level mutable variables. Multiple game instances must coexist on one page.

## Workflow

**Batch cargo commands:**
```bash
cargo clippy --all-targets --all-features -- -D warnings && cargo test -q -- --include-ignored
```
Always pass `--include-ignored`. Edit all files first, then build once.

**Complexity ratchet:** after any change to `reincarnate-core/src/transforms/`, run `normalize ratchet check` to verify no file's cyclomatic complexity increased. A complexity increase means a monkeypatch was added — real fixes simplify the model.

`cargo run -p reincarnate-cli -- check --manifest <path>` is the replacement for `tsc`. Flags: `--filter-code TS2345`, `--filter-file foo.ts`, `--filter-message "..."`, `--examples -1`. Stdout = diagnostics; stderr = progress. Never `2>&1`.

**Implementation always goes through agents.** The main context is for coordination only — decisions, review, direction. Every edit, write, and build command belongs in an agent.

**Commit after every phase.** Each commit = one logical unit of progress. Conventional commits: `type(scope): message`. Types: `feat`, `fix`, `refactor`, `docs`, `chore`, `test`.

**Session handoff:** flush TODO.md → plan mode with next tasks and blocked items only. No commands, build steps, or context summaries.

**Adversarial audits:** periodically audit for suppressions, workarounds, and silent stubs.
1. Commit-diff: `git log --oneline --since="2 weeks ago"`, batch ~60 commits per haiku agent, flag violations.
2. Conversation-log: `~/git/rhizone/normalize/target/debug/normalize sessions messages --days 14 --role assistant --limit 0`, split into ~700-line batches, flag suppression patterns.

## Constraints

- No engine-specific logic in `reincarnate-core` — no named engine functions, no engine-specific heuristics, no backward inference that compensates for engine gaps
- No backward type propagation in core transforms (inferring a value's type from how it is used downstream)
- No `any` in emitted TypeScript, runtime code, or Rust emit paths — `unknown` for unknown types, specific types for known types
- No widening runtime types to match wrong emitter output — fix the inference
- No Claude Code auto-memory (`~/.claude/projects/.*./memory/`) — unversioned and invisible; write behavioral changes to CLAUDE.md instead
- No path dependencies in Cargo.toml
- No `--no-verify`
- No interactive git commands (`git rebase -i`, `git add -i`, `git add -p`) — stage by name
- No DOM data attributes as state-passing mechanism
- No `function_modules` entry without a corresponding `function_signatures` entry

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
