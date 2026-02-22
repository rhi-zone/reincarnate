# GameMaker 5/6/7 (GM5–7)

**Status: Planned** — Not yet started. Test game: Seiklus (`~/reincarnate/gamemaker/seiklus/`).

## Format

GM5–7 games compile to a self-contained Windows executable. The game data (sprites, rooms, scripts, GML bytecode) is appended to the PE at a known offset and self-extracted at runtime. There is no separate `data.win`; `seiklus.dat` (73 bytes) is just a small config file.

The format is documented by:
- [OpenGMK](https://github.com/OpenGMK/OpenGMK) — open-source GM8.0 runner in Rust; `src/game/reader.rs` parses the exe-embedded format from GM5.x onward
- [Altar.NET](https://github.com/colinator27/altar.net)
- [Game Maker Decompiler](https://github.com/WarlockD/GMdsam)

Key format characteristics:
- Data is appended to the PE after the `MZ`/`PE` headers; offset found by scanning for a magic marker near EOF
- Sections: general info, sprites, sounds, backgrounds, paths, scripts, fonts, objects, rooms, and the GML bytecode per script/event
- GML opcode set is simpler than GMS1 — no `Dup`, no `Break`, no `CallV`, no typed stack; closer to a tree-walking interpreter than a stack VM
- Strings are Pascal-style (length-prefixed, not null-terminated)
- No concept of anonymous functions or closures (those came in GMS2.3+)

## Lifting Strategy

1. **PE unpacker** — scan `seiklus.exe` for the GM data magic marker; extract the raw data blob
2. **New format reader** — parse the GM5/6/7 binary layout (entirely different from FORM/IFF)
3. **Opcode decoder** — GM5/6 GML is simpler than GMS1; subset of the existing translator handles most of it
4. **`reincarnate-frontend-gamemaker5` crate** — new crate sharing IR/translator infrastructure with the existing GML frontend
5. **Runtime reuse** — `runtime/gamemaker/ts/` applies; GML object/instance semantics are the same

## Implementation Plan

- [ ] **Locate and extract PE-embedded data** — read `seiklus.exe`, find the GM data offset (magic `0xDEADC0DE` or similar marker near EOF per OpenGMK), extract blob
- [ ] **Parse GM5/6 format** — implement section reader following OpenGMK's `reader.rs` as reference
- [ ] **Implement GM5/6 opcode decoder** — simpler than GMS1; start from OpenGMK's opcode enum
- [ ] **Wire into CLI** — `"GameMaker5"` engine value in manifest
- [ ] **Emit and TS-check Seiklus** — iterate until output is clean

## Reference

- `OpenGMK/src/game/reader.rs` — authoritative Rust reader for the GM5–8 exe format
- `~/reincarnate/gamemaker/seiklus/seiklus.exe` (4 MB — small enough to inspect fully)
