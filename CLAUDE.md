# CLAUDE.md

Behavioral rules for Claude Code in this repository.

## Overview

Reincarnate is a legacy software lifting framework. It extracts and transforms applications from obsolete runtimes (Flash, Director, VB6, HyperCard, RPG Maker, etc.) into modern web-based equivalents. Output is compiled TypeScript (or Rust) — not a bundled interpreter. See `docs/architecture.md` for design details.

**Type inference belongs in the IR, not in a backend.** Inference happens in the frontend, IR transform passes, or a dedicated pass — never inside the TypeScript printer. `FunctionSig`/`Type` in the IR is the source of truth. Backends read from it; they don't create it.

**Never suggest bundling an existing interpreter.** inkjs, Parchment, renpyweb, libqsp-WASM produce running games but not emitted code. Note them as "quick deploy" alternatives — not the goal.

## Core Rule

**Note things down immediately — before writing code, not after:**
- Bugs/issues → fix or add to TODO.md
- Design decisions → docs/ or code comments
- Future work → TODO.md
- Key insights → this file

**Triggers:** User corrects you, 2+ failed attempts, "aha" moment, framework quirk discovered → document **before** proceeding. Never describe a bug in conversation and move on — write it to TODO.md first.

**Conversation is not memory.** Anything said in conversation evaporates at the end of the session. If a statement implies future behavior change, it MUST be written to CLAUDE.md or a memory file immediately. A statement like "I won't do X again" made only in conversation is a lie by omission.

**Every observed problem goes to TODO.md. No exceptions.** Code comments, commit messages, and conversation are not tracked items. If you write a `// TODO` in source code, open TODO.md next.

**Something unexpected is a signal, not noise.** Surprising results are almost always early bug evidence. Investigate before proceeding.

**Any correction or realization means update CLAUDE.md now.** Ask: what rule would have prevented this? Write it before proceeding. A correction without a new rule will repeat. If corrected twice on the same topic, write a broader principle covering the entire class.

**Do the work properly.** When asked to analyze X, actually read X — don't synthesize from conversation.

**Investigation findings go to TODO.md before continuing.** Root causes, affected counts, code locations, fix strategies — write them before the next step.

## Behavioral Patterns

- **Question scope early:** Before implementing, ask whether it belongs in this crate/module.
- **Check consistency:** Look at how similar things are done elsewhere in the codebase.
- **Implement fully.** Test projects are examples, not the spec — fix the entire class, not just the case that blew up. In a multi-stage pipeline, grep all stages before closing a task. Every API method, even ones no test game uses, belongs in the runtime.
- **Verify before stating:** Don't assert API behavior or codebase facts without checking.
- **Write regression tests for reproducible compiler bugs.** Tests must assert correct externally-observable behavior — not mirror the implementation. If writing the correct assertion causes it to fail, mark it `#[ignore = "known bug: ..."]` and add to TODO.md; never adjust the assertion to match broken behavior.
- **Treat special-casing as a smell:** A fix that adds a narrow guard often means the pass's core logic is wrong. Fix the assumption, not the symptom. Use `git blame` to check whether special-case guards have accumulated — that pattern indicates a deeper design gap.
- **A type error in emitted code is a diagnostic, not a prompt to coerce.** When emitted code has a type mismatch (TS2322, TS2345, TS2554, etc.), ask *why is the type wrong?* The error may be: (a) a correct diagnostic of a game-author bug — leave it; (b) a sign our type inference is wrong — fix the inference; (c) the emitter using a stale type — fix it to read `value_types`. Never suppress (a) with a coercion, never widen a runtime API signature to suppress (b), and never add `...args: any[]` to suppress (c). Coercions that paper over wrong IR types hide bugs and compound over time.
- **`value_types[v]` is the authoritative post-transform type; `BlockParam.ty` / `param.ty` may be stale.** The emitter's `collect_block_param_decls` must use `value_types[param.value]`, not `param.ty`.
- **Frontend/backend specific logic never belongs in `reincarnate-core`.** This includes language-specific coercions, target-specific type mappings, and engine-specific idioms. Before adding any logic to core, ask: "which language/engine requires this?" If specific, it belongs in that frontend/backend.
- **Don't hand-roll what a library does.** JS identifier validity → `unicode_ident::is_xid_start`/`is_xid_continue` (plus `$`); JS string escaping → `serde_json::to_string`.
- **Correctness is non-negotiable — 100%, always.** Never defend a shortcut with "our inputs are ASCII-only" or "this won't come up in practice."
- **Never optimize for fewer errors.** Never weaken types (`any`, optional params, wider unions) to silence errors. `any` means inference failed — find the missing information.
- **Preserve fidelity, including source bugs.** When emitted TypeScript reflects a bug in the source (e.g. `|` instead of `||`), that is correct behavior. Don't add special-case guards to "fix" source bugs. The only exception: if the error stems from imprecise type inference on our end, fix the inference.
- **Verify semantics against the authoritative source.** Check what the original actually does. If no authoritative source is found, record the assumption in TODO.md.
- **When something exists, it exists for a reason.** Before removing or bypassing a mechanism, read why it was added.
- **Eliminate megamorphic dispatch at compile time.** When a runtime method dispatches on a string literal (e.g. `math("round", x)`), the backend rewrite pass must resolve it to a direct monomorphic call. A missing method is a compile-time error, not silent `undefined`. Existing namespaces: `Math` (built-in), `Collections`, `Colors`, `StringOps`.
- **Games are instantiable — no singletons.** All mutable runtime state lives on a root runtime instance threaded through generated code — never in module-level `let` variables.

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
- **Stubs must throw, not silently fail.** Use `throw Error("name: not yet implemented")` + add a TODO.md entry. Silent returns (`0`, `""`, `false`, `null`, `{}`) are always wrong.
- Use path dependencies in Cargo.toml — causes clippy to stash changes across repos
- Use `--no-verify` — fix the issue or fix the hook
- Use interactive git commands (`git rebase -i`, `git add -i`, `git add -p`) — they hang forever. Stage files by name: `git add <file1> <file2>`.
- Use module-level mutable state — see "Games are instantiable"
- Use DOM data attributes as a state-passing mechanism
- **Promote `|`/`&` to `||`/`&&` based on inferred types.** They're semantically different. TS2447/TS2363 from `boolean | boolean` are game-author errors — don't suppress them.
