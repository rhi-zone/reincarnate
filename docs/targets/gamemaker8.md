# GameMaker 8.x (GM8 / GM8.1)

**Status: Planned** — Not yet started. Test game: Hotline Miami (`~/reincarnate/gamemaker/hotlinemiami/`).

## Format

GM8.x games ship as a self-contained Windows executable. On Linux, Hotline Miami packages its data in `HotlineMiami_GL.wad`, a custom asset container (not the FORM/IFF format used by GMS1+). The container holds sprites, audio, room data, and GML bytecode. The header starts with a file count + directory of offset/size pairs, followed by path-prefixed entries (observed: `GL/Assets/GL/<filename>`).

GM8 predates the chunk-based FORM/IFF design of GMS1. The data layout is documented by:
- [UndertaleModTool](https://github.com/UndertalModTool/UndertaleModTool) — supports GM8 (older branch; `UndertaleChunkCode.cs`)
- [Altar.NET](https://github.com/colinator27/altar.net) — GM8 reader in Rust, good reference
- [GameMaker 8 Decompiler](https://github.com/WarlockD/GMdsam) — older C# reference

The GML opcode set in GM8 is similar to GMS1 but with differences:
- No `Dup`, no `Break` signals, no `CallV`
- Older `with`/`push`/`pop` semantics
- String encoding differs (Pascal strings vs length-prefixed)

## Lifting Strategy

1. **New container parser** — read the `.wad` (or Win32 PE resource section) to extract the raw GM8 data blob
2. **New chunk reader** — parse the GM8 binary format (not FORM/IFF; fixed-offset sections for objects, scripts, rooms, etc.)
3. **Reuse GML translator** — the IR emission and most opcode handling overlaps with the existing `reincarnate-frontend-gamemaker`; create `reincarnate-frontend-gamemaker8` that shares translator logic
4. **Runtime reuse** — `runtime/gamemaker/ts/` is largely applicable; GML semantics are the same

## Implementation Plan

- [ ] **Understand the `.wad` container** — parse the header, enumerate entries, extract the code/data blob
- [ ] **Read GM8 binary format** — map the fixed-offset sections (GEN8 equivalent, object table, script table, string pool, bytecode)
- [ ] **Implement GM8 opcode decoder** — diff against GMS1 opcode table; adjust/subset existing decoder
- [ ] **Wire into CLI** — `reincarnate-frontend-gamemaker8` crate, new `"GameMaker8"` engine value in manifest
- [ ] **Emit and TS-check Hotline Miami** — iterate until output is clean

## Reference

- UTMT issue tracker and wiki for GM8 format notes
- `Altar.NET/src/format/gm8.rs` for a Rust-native GM8 reader
- `~/reincarnate/gamemaker/hotlinemiami/HotlineMiami_GL.wad` (342 MB)
