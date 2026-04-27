# GML Frontend Notes

## Adding a New Runtime Function

Before adding a new runtime function, find an existing function that works the same way and copy its registration pattern exactly. The two paths are distinct:

- **Intrinsic** (`register_runtime_intrinsic` + `IntrinsicKind`): only for functions that need special IR-level handling (field access, global get/set, etc.). The intrinsic name must match the `function_modules` entry name for the stateful-call rewrite to fire. If they differ, the rewrite silently fails.
- **Plain runtime method** (`register_runtime` with bare name + entry in `function_modules` with `stateful: true`): for functions callable as `this._rt.methodName()`. The bare name must match exactly.

Find one existing working example first. Do not invent a registration pattern.

## Runtime Signatures

GML function reference: https://manual.gamemaker.io/monthly/en/ (machine-parseable source: https://github.com/YoYoGames/GameMaker-Manual)

Local manual lookup: `cargo run -p reincarnate-cli -- gml-docs <function_name>` — outputs the manual page as markdown. Use this before adding or fixing any runtime function signature. Never infer param counts or types from decompiled call sites — they may be wrong.

Signature diff tool: `bun scripts/gml-manual-sigs.ts --diff` — compares `runtime.json` against the local manual clone, flagging arity mismatches, type mismatches, and `any` where the manual has a concrete type.
