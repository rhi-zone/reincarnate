# GML Frontend Notes

## Runtime Signatures

GML function reference: https://manual.gamemaker.io/monthly/en/ (machine-parseable source: https://github.com/YoYoGames/GameMaker-Manual)

When adding or fixing a runtime function signature in `runtime.json` or `runtime.ts`, look up the function in the manual first. Never infer param counts or types from decompiled call sites — they may be wrong.
