# RPG Maker

**Status: Planned** — No implementation started. Two very different versions exist.

---

## RPG Maker VX Ace (and earlier)

### Format

RPG Maker VX Ace stores game data in a proprietary binary format:
- **`Scripts.rvdata2`** — all Ruby/RGSS3 scripts, Marshal-serialized array of `[id, name, zlib(source)]`
- **`*.rvdata2`** — actors, classes, skills, items, weapons, armors, enemies, troops, states, animations, tilesets, commonevents, system, areas, maps (Marshal-serialized)
- **Map files** — `MapXXX.rvdata2` — room data with tile layers and event lists
- **`Game.exe`** — bundled RGSS (Ruby Game Scripting System) runtime, not user code
- **`Audio/`**, **`Graphics/`**, **`Fonts/`** — asset directories

RGSS3 (VX Ace) / RGSS2 (VX) / RGSS (XP) are wrappers around Ruby with game-specific extensions for sprites, windows, audio, and input.

### Lifting Strategy

Full recompilation (Tier 2).

1. Extract Ruby source from `Scripts.rvdata2` (decompress each entry with zlib)
2. Parse Ruby source — well-documented language with parser gems (RubyParser, Parser gem)
3. Convert to IR — Ruby is dynamically typed; type recovery needed
4. Identify RGSS API boundaries

### What Needs Building (VX Ace)

- [ ] `Scripts.rvdata2` extractor (Ruby Marshal format parser)
- [ ] Map/event data extractor (`*.rvdata2` for event commands)
- [ ] Ruby AST → IR lowering (significant: Ruby closures, blocks, modules, mixins)
- [ ] RGSS3 replacement runtime:
  - [ ] `Sprite` — bitmap + transform + viewport
  - [ ] `Window` / `Window_Base` / `Window_Command` — the standard menu/dialogue system
  - [ ] `Viewport` — z-ordered rendering layers
  - [ ] `Bitmap` — software raster (fill, blt, draw_text, etc.)
  - [ ] `Font` — typeface selection
  - [ ] `Input` — key polling (RPG Maker uses a fixed key map)
  - [ ] `Audio` — BGM/BGS/ME/SE playback
  - [ ] `Graphics` — frame timing, transition effects, freeze/transition
  - [ ] `RPG::*` data classes (auto-generated from Marshal data)

### Known Challenges

- **Ruby complexity** — Blocks, procs, lambdas, method_missing, `eval`, `require` make full static analysis hard. Most game code is straightforward but the default scripts use advanced patterns.
- **Default scripts** — A large portion of the game is built-in RGSS "default scripts" (`Scene_Map`, `Scene_Battle`, `Game_Player`, etc.). These can be recognized and replaced wholesale with reimplementations.
- **Battle system plugins** — VX Ace games often use community plugins (Yanfly, Fallen Angel, etc.) that heavily monkey-patch the default scripts. These require case-by-case handling.

---

## RPG Maker MV / MZ

### Format

MV and MZ are fundamentally JavaScript-based, already targeting the browser:
- **`js/rpg_*.js`** — bundled engine source (obfuscated in some distributions)
- **`data/*.json`** — all game data: actors, maps, events, skills, items, etc.
- **`js/plugins.js`** — plugin loading manifest
- **`js/plugins/`** — community plugins (JavaScript)
- **`audio/`**, **`img/`**, **`fonts/`** — assets

Map events are JSON arrays of command lists. The event command format is well-documented (integer command codes with parameter arrays).

### Lifting Strategy

Patch-in-place + event compilation (Tier 2, partial).

Since the engine already runs in JavaScript, the most practical approach is:
1. **Compile event scripts** — JSON event command lists → TypeScript functions
2. **Optimize the engine** — tree-shake unused features, replace Pixi.js calls with direct canvas
3. **Inject plugins** — compile and bundle community plugins alongside generated code

Full decompilation of the engine JavaScript itself is lower value since it already targets the web.

### What Needs Building (MV/MZ)

- [ ] JSON event command compiler — map each command code to IR or direct TypeScript emit
  - Movement commands: move route, change direction, transfer
  - Message commands: show message, show choices, input number, select item
  - Control commands: switches, variables, self switches, conditions, loops, labels
  - Audio/video commands: BGM/BGS/ME/SE, video playback
  - System commands: battle processing, shop processing, name input, menu, save, game over
  - Character commands: show/erase/rotate/change graphic, animation, movement
  - Picture commands: show/move/rotate/tint/erase picture
  - Screen commands: fadeout/fadein, flash/shake/tint screen, weather
- [ ] Data extractor — JSON → typed data classes for actors, items, skills, etc.
- [ ] Plugin compatibility shim — maintain `PluginManager` API so existing community plugins work

## References

- [RPG Maker VX Ace Help (Ruby)](https://www.rpgmakerweb.com/support/products/rpg-maker-vx-ace)
- [RPGMakerMV Event Command Reference](https://rpgmaker.net/tutorials/2506/)
- [MV/MZ Plugin API](https://forums.rpgmakerweb.com/index.php?threads/rpg-maker-mv-plugin-creation-tutorial.27721/)
