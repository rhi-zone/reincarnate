# RAGS (Rich Authoring and Generation System)

**Status: Planned** — No implementation started. `rags2html` provides a working reference implementation.

## Background

RAGS is a Windows-only interactive fiction authoring tool popular in the NSFW IF community. Games are click-and-read affairs with an inventory system, branching narrative, image display, and timed events. Development of the engine stopped years ago; the game format is closed and proprietary.

## Format

RAGS uses two distinct binary formats depending on the engine version:

**Pre-1.7 (NRBF):**
- .NET Remoting Binary Format — a serialization of the engine's internal C# classes using `BinaryFormatter`
- Encrypted with AES-256-CBC using a hardcoded key and IV
- Game data after decryption is a graph of C# object instances: rooms, characters, items, timers, and media references

**Post-1.7 (SDF):**
- Microsoft SQL Server Compact Edition (SQL CE) database
- Each 4096-byte page is encrypted with AES-128-CBC; per-page keys are derived from a global key and a page checksum
- The SDF format is otherwise undocumented

Both formats store: rooms, characters, items, inventory, timers, variables, player state, and media file references (images, audio). Game logic is **data-driven** — there is no Turing-complete scripting language. Conditions are stored as structured predicates, not code.

## Existing Tool: rags2html

**[rags2html](https://github.com/Kassy2048/rags2html)** is an open-source tool that:
1. Decrypts both the NRBF and SDF formats using the known keys
2. Parses the game data
3. Generates a self-contained playable HTML file using a reimplemented "Regalia" JavaScript runtime
4. Has been tested against ~100 games

This is the definitive reference for the format. The reincarnate approach would build on this work.

## Lifting Strategy

Data-driven extraction (Tier 2). Since game logic is stored as structured predicates rather than code:
1. Decrypt the `.rags` file (keys are known from rags2html)
2. Deserialize the NRBF or SDF structure
3. Translate room/item/character/event data to IR or directly to TypeScript
4. Emit a replacement runtime for the "Regalia" engine UI (text display, inventory, images, choices)

The absence of a scripting language makes this one of the simpler engines to lift — the entire game state machine is in the data, not code.

## What Needs Building

### Format Parser (new crate: `reincarnate-frontend-rags`)

- [ ] AES-256-CBC decryption (NRBF variant) — key/IV already documented in rags2html
- [ ] NRBF deserializer (pre-1.7): parse .NET BinaryFormatter stream → game object graph
- [ ] SQL CE SDF parser (post-1.7): page-level AES decryption + SQL CE B-tree page format → game object graph
- [ ] Game data model: rooms, exits, items, characters, variables, timers, conditions, actions, media references

### IR / Code Generation

Since the game logic is data-driven (not a scripting language), the "IR" is more of a data model translation:
- [ ] Rooms → TypeScript objects with text, image, exit list, item list
- [ ] Conditions → TypeScript boolean expressions over game state
- [ ] Actions → TypeScript functions (set variable, move item, print text, play audio, etc.)
- [ ] Event system → triggered actions on condition match

### Replacement Runtime (`runtime/rags/ts/`)

- [ ] Text output area with formatted HTML
- [ ] Image display panel (room/character images)
- [ ] Inventory display
- [ ] Choice/button UI (exits, item interactions, conversation options)
- [ ] Variable state management
- [ ] Timer system (timed events)
- [ ] Audio playback (background music, sound effects)
- [ ] Save/load

## References

- [rags2html (GPL)](https://github.com/Kassy2048/rags2html)
- [RAGS — IFWiki](https://www.ifwiki.org/R.A.G.S.)
