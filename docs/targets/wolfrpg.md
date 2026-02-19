# WolfRPG (Wolf RPG Editor / Woditor)

**Status: Planned** — No implementation started. Community tools provide format documentation.

## Background

Wolf RPG Editor (ウディタ, Woditor) is a Japanese RPG creation tool widely used for indie RPGs and eroge. Games ship as Windows executables backed by DxLib (DirectX). The game data is fully external in binary files — the `Game.exe` is the closed-source engine.

## Format

WolfRPG stores game data in a `Data/` directory (or encrypted inside a `.wolf` archive):

- **`.mps`** — map files: tile layer data + per-map event scripts
- **`CommonEvent.dat`** — common events (global event scripts shared across maps)
- **`SysDatabase.dat`** — system database (engine configuration, tile types, etc.)
- **`UserDatabase.dat`** — user database (custom data tables used by game logic)
- **`VariableDatabase.dat`** — variable database (named global variables)
- **`Game.dat`** — project metadata (start map, starting position, window title, etc.)
- **`.wolf` archive** — optional encrypted container: XOR-encrypted using DxLib's key scheme; all above files packed inside

All `.dat` and `.mps` formats are custom binary with no official specification. Substantially reverse-engineered by the translation community.

## Event Scripts

WolfRPG game logic is stored as **sequences of numbered commands** — closer to a bytecode-ish representation than source text. The command set is fixed and finite (unlike a general scripting language). Commands include:

- Variable assignment (set database value, set variable)
- Conditional branches (`IF/ELSE/END` by variable comparison)
- String operations
- Map transitions (`change map`)
- Message display (dialogue boxes, choices)
- Database read/write
- Picture manipulation (show/move/rotate/erase)
- Sound/music playback
- Instance creation/destruction
- System calls (wait, screen effects, save/load)

The engine is event-driven: events fire on collision, button press, or trigger conditions.

## Existing Tools

- **[WolfTL](https://github.com/Sinflower/WolfTL)** — parses `CommonEvent.dat`, `*Database.dat`, and `.mps` files to JSON; designed for translation patching
- **[WolfDec / UberWolf](https://github.com/Sinflower/WolfDec)** — decrypts `.wolf` archives
- **[wolfrpg-map-parser](https://crates.io/crates/wolfrpg-map-parser)** — Rust crate parsing `.mps` files to a struct tree (library + JSON-dump binary)
- **[woteconv](https://github.com/amm073334/woteconv)** — converts `CommonEvent.dat` to text
- **[elizagamedev/wolftrans](https://github.com/elizagamedev/wolftrans)** — Ruby translation tooling

## Lifting Strategy

Data + event script extraction (Tier 2).

1. Decrypt `.wolf` archive if needed (WolfDec)
2. Parse `.mps`, `CommonEvent.dat`, `*Database.dat` files — existing Rust crate `wolfrpg-map-parser` handles `.mps`; WolfTL handles `.dat`
3. Translate event command sequences to IR — each event is a function, commands map to IR ops
4. Reimplement the runtime (tile map rendering, dialogue system, etc.)

## What Needs Building

### Format Parser (new crate: `reincarnate-frontend-wolfrpg`)

- [ ] `.wolf` archive decryption (DxLib XOR scheme — documented in WolfDec)
- [ ] `.mps` parser — can potentially reuse or adapt `wolfrpg-map-parser` (Rust)
- [ ] `CommonEvent.dat` parser — event script command sequences
- [ ] `*Database.dat` parser — tabular data with type-annotated cells (int, string, filename)
- [ ] `Game.dat` parser — project configuration

### Event Command Compiler

Map each Wolf command ID to an IR operation:
- [ ] Variable/database read → field load
- [ ] Variable/database write → field store
- [ ] Conditional branch → IR branch
- [ ] Message display → `SystemCall("Wolf.Message", text)`
- [ ] Choice → `SystemCall("Wolf.Choice", options)` + `Yield`
- [ ] Map transition → `SystemCall("Wolf.MapTransition", mapId, x, y)`
- [ ] Picture commands → `SystemCall("Wolf.Picture.*")`
- [ ] Sound/music → `SystemCall("Wolf.Audio.*")`

### Replacement Runtime (`runtime/wolfrpg/ts/`)

- [ ] Tile map renderer (Canvas 2D, multi-layer: lower/event/upper)
- [ ] Event system (touch events, auto-run events, parallel events)
- [ ] Dialogue box (character name, message text, next-page advancement)
- [ ] Choice menu
- [ ] Character movement (step-based, 4-directional)
- [ ] Battle system (WolfRPG has a configurable turn-based battle engine stored in `SysDatabase`)
- [ ] Picture layer (layered images with blending)
- [ ] Screen effects (fade, flash, shake)
- [ ] Save/load (variable state + current map position)
- [ ] Audio (BGM, sound effects)

## Known Challenges

- **DxLib graphics** — The original engine uses DxLib for hardware-accelerated 2D rendering. Canvas 2D approximation is fine for most games; pixel-perfect matching for games with complex shader effects may not be achievable.
- **Battle system** — WolfRPG's configurable battle system is defined in `SysDatabase`. Games can heavily customize it; faithfully reimplementing all possible configurations is complex.
- **Image format** — WolfRPG uses standard Windows BMP/PNG/JPEG for tiles and sprites; no special format conversion needed.
- **`.wolf` encryption variants** — Different versions of the editor use different encryption. WolfDec supports multiple versions; the reincarnate parser should handle the same range.

## References

- [WolfTL](https://github.com/Sinflower/WolfTL)
- [WolfDec / UberWolf](https://github.com/Sinflower/WolfDec)
- [wolfrpg-map-parser (Rust crate)](https://crates.io/crates/wolfrpg-map-parser)
- [Wolf RPG Editor official site](https://www.silversecond.com/WolfRPGEditor/)
