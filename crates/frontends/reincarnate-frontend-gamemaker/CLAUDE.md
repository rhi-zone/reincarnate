# GML Frontend Notes

## Runtime Signatures

GML function reference: https://manual.gamemaker.io/monthly/en/ (machine-parseable source: https://github.com/YoYoGames/GameMaker-Manual)

Local manual lookup: `cargo run -p reincarnate-cli -- gml-docs <function_name>` — outputs the manual page as markdown. Use this before adding or fixing any runtime function signature. Never infer param counts or types from decompiled call sites — they may be wrong.

Signature diff tool: `bun scripts/gml-manual-sigs.ts --diff` — compares `runtime.json` against the local manual clone, flagging arity mismatches, type mismatches, and `any` where the manual has a concrete type.
