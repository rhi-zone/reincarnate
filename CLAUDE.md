# CLAUDE.md

## Goal

Reincarnate is a decompiler that produces working, type-safe, high-quality code from legacy game binaries. The emitted code (TypeScript, Rust, etc.) is the artifact — it compiles, runs, and is as readable and editable as any well-written codebase.

**Design target:** arbitrary source languages, arbitrary target languages, zero coupling between them, state-of-the-art type inference and static analysis, high-quality codebase architecture. Every decision is evaluated against this bar.

**Never suggest bundling an existing interpreter.** inkjs, Parchment, renpyweb, libqsp-WASM produce running games but not emitted code. Note them as "quick deploy" alternatives — not the goal.

**The emitted code is the mod surface.** Lift once, edit the output, forward-port via cherry-pick if the upstream game updates. No IR mutation API needed.

**Emitted code is measured against handwritten code.** The bar is not "compiles" or "runs" — it is "indistinguishable from a skilled human port." Runtime name lookup where a direct call is possible, `unknown` where a concrete type is inferrable, or any other indirection that a human would never write are all defects. Closing the gap is always higher priority than adding new features.

## Quality

**Wrong code causes cascading damage: wasted time, risky reverts, corrupted `git blame`, and misdirected future work. Read the relevant code before modifying anything — confidence is not a feeling, it is a result of having verified. When a request is ambiguous or could be interpreted multiple ways, state your interpretation and wait for confirmation before touching any file.**

**Sloppiness is not excusable.** There is no pressure, deadline, or metric that justifies a sloppy fix. The most common form of sloppiness in this codebase is treating TypeScript error counts as a goal — they are not. A change that reduces errors by widening a type, silencing a diagnostic, or guessing at correct behavior is a regression. The only valid reason to make a change is that it is correct.

**The delta between "compiles" and "correct" lives in TODO.md.** Every known gap, unverified assumption, silent limitation, and unimplemented behavior must be tracked there. Not adding a TODO entry is an implicit claim of correctness. Growing TODO.md as scope grows is fine; gaps missing from TODO.md are not.

**Implement fully — throwing is not a substitute.** `throw Error("impossible: ...")` is acceptable only when correct implementation is genuinely blocked by a missing prerequisite documented in TODO.md. "Not yet implemented" is not a reason to throw — it is a reason to implement. Silent returns (`0`, `""`, `false`, `null`, `{}`), no-op passes that return input unchanged, and stubs that compile but produce wrong output are all the same defect.

**Look up the spec; don't guess from call sites.** GML: docs.yoyogames.com (source: github.com/YoYoGames/GameMaker-Manual). Flash: AS3 API reference. A decompiled call site may be wrong; the spec is authoritative.

**Fix the real problem.** A workaround avoids fixing the actual cause. Narrow guards on symptoms indicate wrong core logic. If a fix is blocked by a deeper issue, fix the deeper issue first — or document both in TODO.md and leave the code unchanged. Emit-level casts and type widenings that exist only to satisfy the type checker without improving inference are workarounds — they paper over the gap instead of closing it.

**The sign of a correct fix is that code gets simpler — never monkeypatch.** A correct fix changes the model so the case can't arise. If a fix requires a "shouldn't happen but does" guard, fix what makes it happen instead. A branch that exists to compensate for upstream failures is a monkeypatch.

**Conversation is not memory.** Anything said in chat evaporates at session end. If it implies a future behavior change, write it to CLAUDE.md immediately — or it will not happen.

**Good tooling is a high priority.** When a task is tedious and error-prone, automate it. If you find yourself running the same command multiple times to get different views of the output, that is a tooling gap — fix the command's output first, then run it once.

**A correct principle that doesn't prevent bad behavior isn't doing its job.** When a rule fails to stop a mistake, make it more specific until it would have caught it — don't add new rules. More rules dilute attention and make existing rules less effective.

**Repeated pushback means CLAUDE.md is wrong.** If the user corrects the same behavior more than twice, stop guessing and ask directly what's missing. Then fix the principle that failed — not by adding a new rule, but by making the existing one specific enough that it would have caught it.

## Fundamental Laws

These are invariant. When a violation appears, adjust the law — don't add a corollary.

**1. Pipeline Stage Isolation.** The IR is the only channel between pipeline stages. Everything a backend needs must be in the IR. Side channels mean the IR is incomplete — extend it; don't route around it.

**2. Engine Specificity at Boundaries.** Frontends know the source engine. Backends know the target language. Core (IR, transforms) knows neither. Engine-specific logic in core is in the wrong place.

**3. Behavioral Equivalence.** Emitted code produces identical observable output for any input. Preserve source-language bugs. Never add guards to "fix" source bugs — fix the inference instead.

**4. Honest Representation.** IR types reflect source-language semantics, not VM storage format. A GML boolean is `Bool`, not `Float`. Source-level type violations surface as target-language type errors — that is correct behavior.

- **`Dynamic` is a type inference failure, and `any` is what makes it invisible.** Every value has a concrete type at the source level. `Dynamic` in the IR means inference wasn't good enough. A Rust emitter on the same IR cannot use `any` — it must use a `Value` enum with runtime dispatch on every operation, which destroys the static guarantees and performance that make Rust worth targeting. `any` in TypeScript silently papers over the same gap that would force a Rust emitter into an unusable design. Specifically prohibited:
  - `any` in a runtime type signature → use `unknown`; the call site must narrow before use
  - `Record<string, any>` or `[key: string]: any` → use `unknown` as the value type
  - `(expr as any)` in the emitter → fix the IR type; `as unknown` is acceptable only as a temporary marker with a TODO entry for the root cause
  - Any of the above added to reduce TypeScript error counts → this is a regression, not a fix
- **Preserve integer vs float distinction.** Use `"int"` for indices, IDs, counts, flags, enum values; `"number"` for continuous values (coordinates, scales, angles). Matters for non-TS backends and the type checker.

**5. Instantiability.** All mutable runtime state lives on root runtime instances. No module-level mutable variables. Multiple game instances must coexist on one page.

## Workflow

**Batch cargo commands:**
```bash
cargo clippy --all-targets --all-features -- -D warnings && cargo test -- --include-ignored
```
Always pass `--include-ignored`. Edit all files first, then build once.

**Snapshot before/after each phase.** Dump representative functions to `~/reincarnate/snapshots/` before and after each phase and diff them to verify output quality didn't regress. Example: `cargo run -p reincarnate-cli -- emit --manifest ~/reincarnate/gamemaker/deadestate/reincarnate.json --dump-function <name> > ~/reincarnate/snapshots/before.txt`. This is a workaround for the lack of checked-in output snapshots (copyright); may not be needed once the rewrite is done.

**Commit after every phase.** Each commit = one logical unit of progress. Conventional commits: `type(scope): message`. Types: `feat`, `fix`, `refactor`, `docs`, `chore`, `test`.

**Use `bun`** for JavaScript/TypeScript scripting tasks.

**Never invoke `tsc`, `tsgo`, or `bunx tsc` directly — not even to check a single runtime file.** Always use `cargo run -p reincarnate-cli -- check --manifest <path>`. The emit and check caches make this fast — one command, always correct. When you need more detail: `--filter-code TS2345` (one error code), `--filter-file foo.ts` (one file), `--filter-message "some text"` (message substring), `--examples -1` (all instances instead of 3). These flags compose. Wanting to check a specific runtime file is not an exception — `--filter-file navigation.ts` is the right tool. **Practical enforcement**: running `tsc` on a large game (DoL, TRC) triggers earlyoom and kills other processes. There is no situation where running `tsc` directly gives you something `check` cannot.

**`reincarnate check` output format:** progress lines go to stderr; diagnostics go to stdout. Never use `2>&1` — it mixes them. Without filters, stdout is a sorted count-by-code summary. With `--filter-code`, the first stdout line is `Showing N of M diagnostics matching ...` — the count is right there, no grep needed. Never grep check output for a code that `--filter-code` already filters.


**Implementation always goes through agents.** The main context is for coordination only — decisions, review, direction. Every edit, write, and build command belongs in an agent. This is not about test-run overhead: even a successful inline implementation pollutes the main context with tool calls that consume context space and obscure the coordination-level thinking. An agent contains the entire implementation — successes, failures, and intermediate states — and returns a clean summary. The main context stays clean regardless of outcome.

**Session handoff:** plan mode → plan with only (next tasks / blocked items / what was done if it affects what's next) → flush TODO.md → ExitPlanMode. No commands, no build steps, no context summaries — those belong in CLAUDE.md or TODO.md. The next session reads both fresh.

**Adversarial audits:** periodically audit for suppressions, workarounds, and silent stubs.
1. Commit-diff: `git log --oneline --since="2 weeks ago"`, batch ~60 commits per haiku agent, flag violations.
2. Conversation-log: `~/git/rhizone/normalize/target/debug/normalize sessions messages --days 14 --role assistant --limit 0`, split into ~700-line batches, flag suppression patterns.

## Constraints

- No Claude Code auto-memory (`~/.claude/projects/.*./memory/`) — unversioned and invisible. Write behavioral changes to CLAUDE.md instead
- No engine-specific logic in `reincarnate-core`
- No path dependencies in Cargo.toml
- No `--no-verify`
- No interactive git commands (`git rebase -i`, `git add -i`, `git add -p`) — stage by name
- No DOM data attributes as state-passing mechanism
- No `function_modules` entry without a corresponding `function_signatures` entry
- No widening runtime types to match wrong emitter output — fix the inference
- No `any` in emitted TypeScript, runtime code, or Rust emit paths — `unknown` for unknown types, specific types for known types; see Law 4

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
