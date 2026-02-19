# PuzzleScript

**Status: Planned (low priority)** — Already a web engine; the main use case is archival and offline hosting of existing games.

## Background

PuzzleScript is an open-source browser-based puzzle game engine by Stephen Lavelle (increpare). Games are written in a domain-specific language and compiled to JavaScript in the browser. The engine is hosted at [puzzlescript.net](https://www.puzzlescript.net/). PuzzleScript Plus ([Auroriax/PuzzleScriptPlus](https://github.com/Auroriax/PuzzleScriptPlus)) extends it with additional features.

## Format

PuzzleScript games are **plain text** — a single `.puz` file or inline text divided into labeled sections:

```
TITLE My Game

OBJECTS
Background
black

Player
black red
.0...
.1...

LEGEND
P = Player
. = Background

SOUNDS
Player MOVE 12345

COLLISION LAYERS
Background
Player

RULES
[ Player | ... ] -> [ | Player ... ]

WIN CONDITIONS
No Player

LEVELS
.....
..P..
.....
```

Key sections:
- `OBJECTS` — 5×5 sprite definitions with named color palette and pixel grid
- `LEGEND` — single-character aliases for objects and combinations
- `SOUNDS` — sound event associations (bfxr seed numbers)
- `COLLISION LAYERS` — layer ordering for collision detection
- `RULES` — pattern-replacement rules (the core game logic): `[pattern] -> [replacement]`
- `WIN CONDITIONS` — victory predicates (`No X`, `Some X`, `All X on Y`)
- `LEVELS` — ASCII grid maps

There is no binary format. Published games are typically hosted as:
- The official editor URL with source encoded in the URL hash
- A self-contained HTML file with the source embedded and the engine bundled

## Runtime

PuzzleScript games are compiled to JavaScript at load time. The compiler (`compiler.js`) processes source text in ~16 passes:
1. Parsing sections
2. Legend expansion (character aliases → object sets)
3. Rule parsing (pattern matching with direction specifiers: `UP`, `DOWN`, `LEFT`, `RIGHT`, `HORIZONTAL`, `VERTICAL`, `ACTION`, `STATIONARY`, `MOVING`)
4. Direction expansion (relative → absolute — `HORIZONTAL` → two rules for left/right)
5. Rule simplification (redundant rules removed)
6. Bitmask generation (each cell is a bitmask of which objects are present)

The compiled output feeds into `engine.js` — a turn-based engine that:
1. Receives a player input (move direction or action)
2. Applies all `late` rules first, then `realtime` rules, then regular rules
3. Iterates rule application until fixpoint (no further rules fire in a pass)
4. Checks win conditions
5. Renders the board state

Rule matching uses bitmask operations for O(1) per-cell checking.

## Lifting Strategy

PuzzleScript is natively a web engine. There is no meaningful "lift" needed — games already run in any browser.

The relevant use cases for reincarnate:

### Use Case 1: Archival / Offline Hosting

Many PuzzleScript games exist only as editor URLs or rely on the online editor for hosting. A reincarnate frontend could:
1. Extract the game source from a saved HTML file or URL-encoded source
2. Bundle it with the engine as a self-contained offline HTML file

This is trivially achievable and doesn't require the full pipeline.

### Use Case 2: Translation to IR

If PuzzleScript rule semantics need to be represented in the IR (e.g., for analysis or transformation):
1. Parse the PuzzleScript source text
2. Represent the rule system as IR — each rule becomes a function that pattern-matches cell bitmasks and performs replacements
3. The rule fixpoint loop becomes an IR loop with an IR function per rule

The rule matching semantics are formally specified and deterministic, making this amenable to exact translation.

## What Needs Building

For archival/offline hosting (immediate value):
- [ ] Source extractor from saved HTML (parse the embedded source text)
- [ ] URL hash decoder (base64/compressed source in editor URLs)
- [ ] Self-contained HTML generator (source + bundled engine)

For full IR translation (optional, lower priority):
- [ ] PuzzleScript source parser (section-based text format)
- [ ] Rule compiler → IR (bitmask-based pattern matching functions)
- [ ] Level data extractor → asset catalog

## References

- [PuzzleScript source (MIT)](https://github.com/increpare/PuzzleScript)
- [PuzzleScript Plus](https://github.com/Auroriax/PuzzleScriptPlus)
- [PuzzleScript documentation](https://www.puzzlescript.net/Documentation/documentation.html)
