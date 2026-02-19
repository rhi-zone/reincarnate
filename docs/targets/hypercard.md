# HyperCard / ToolBook

**Status: Planned** — No implementation started.

## HyperCard

HyperCard (Apple, 1987–2004) is one of the earliest interactive multimedia platforms. Content is organized into **stacks** of **cards**, each with graphical layers and interactive **buttons** and **fields**.

### Format

HyperCard stacks are single binary files in Apple's proprietary format:
- **Stack block** — master block with version info, window size, card count, background count
- **Background (BKGD) blocks** — shared visual layers for groups of cards
- **Card (CARD) blocks** — individual card data with button/field positions and text
- **Bitmap (BMAP) blocks** — 1-bit B&W bitmap graphics for each card/background layer
- **Script block** — HyperTalk script source text (stored as plain text, not bytecode!)
- **Resource fork** — icons, sounds, external commands (XCMDs/XFCNs)

Notably, HyperCard stores scripts as **source text**, not bytecode. There is nothing to decompile — the source code is present verbatim.

The format has been reverse-engineered by the community. StackReader and similar tools can extract cards, bitmaps, and scripts. The [HyperCard format specification](https://github.com/nicklockwood/HyperCardPreview) is documented in open-source implementations.

### HyperTalk

HyperTalk is a natural-language-inspired scripting language:
```
on mouseUp
  go to card "MyCard"
end mouseUp

on doSomething x, y
  put x + y into result
  return result
end doSomething
```

Key features:
- Event handlers: `on mouseUp`, `on mouseDown`, `on keyDown`, `on openCard`, `on closeCard`, `on openStack`, `on idle`
- Message passing: `send "myMessage" to button 1` — messages bubble up the hierarchy (card → background → stack → HyperCard)
- Container model: `put "hello" into field "Name"` — fields, variables, and selections are containers
- Visual effects: `visual effect dissolve / wipe left / zoom open`, `go to card "next"` with visual effect
- `answer` / `ask` — dialogue boxes
- `get` / `set` — property access
- String operations: `the first word of`, `char 1 to 5 of`, etc.

### What Needs Building (HyperCard)

#### Format Parser (new crate: `reincarnate-frontend-hypercard`)

- [ ] Stack binary parser (block structure with IDs and offsets)
- [ ] Card/background layout extractor (button/field positions, sizes, styles)
- [ ] BMAP bitmap decoder (1-bit QuickDraw bitmaps)
- [ ] Script extractor (plain text per object)
- [ ] Resource fork extractor (sounds, icons, XCMDs)

#### HyperTalk Parser

Since scripts are source text, we need a HyperTalk parser:
- [ ] Handler declarations (`on eventName`)
- [ ] `put` / `get` / `set` statements
- [ ] `go to card / background / stack` navigation
- [ ] `if ... then ... else ... end if`
- [ ] `repeat with X = 1 to N` / `repeat while` / `repeat until`
- [ ] `send` message dispatch
- [ ] `ask` / `answer` dialog
- [ ] Container references: `field "Name"`, `card field 1`, `me`, `target`, `sender`, `the message box`
- [ ] Chunk expressions: `the first word of X`, `char 3 of X`, `line 2 of X`
- [ ] XCMDs (external commands) — compiled code extensions in the resource fork

#### Replacement Runtime (`runtime/hypercard/ts/`)

- [ ] Card navigation system (go to, push, pop card)
- [ ] Visual effects (CSS transitions approximating dissolve, wipe, zoom, iris)
- [ ] Card/background rendering (canvas or DOM with absolutely positioned elements)
- [ ] Button rendering and click handling
- [ ] Field rendering (text display and input)
- [ ] Message hierarchy (card → background → stack → global)
- [ ] `answer` / `ask` dialogs
- [ ] Sound playback (HyperCard sounds are 8-bit µ-law or SND resources)
- [ ] Painted graphics (1-bit bitmaps rendered to canvas or converted to PNG)

---

## ToolBook

Asymetrix ToolBook (1990s) is HyperCard's Windows equivalent, used heavily for educational CD-ROMs and corporate training content.

### Format

ToolBook uses `.tbk` files (the "book" container) and `.sbk` files (read-only "shipped books"):
- Books contain **pages** (equivalent to HyperCard cards) and **backgrounds**
- Object hierarchy: book → chapter → page → foreground/background groups → objects
- **OpenScript** — ToolBook's scripting language (event-driven, message hierarchy like HyperTalk)
- Media: embedded bitmaps, sounds (WAVE/MIDI), video references (AVI, QuickTime)

ToolBook files are a proprietary binary format. Less community documentation exists than for HyperCard. ToolBook Instructor/Multimedia and ToolBook Assistant used slightly different formats.

### OpenScript

OpenScript is syntactically similar to HyperTalk but Windows-oriented:
```
to handle buttonClick
  go to page "Page2"
end buttonClick
```
Key differences from HyperTalk:
- `to handle` instead of `on`
- More OOP: objects have explicit class hierarchies
- `sysCursor` property for cursor changes
- Windows API calls via `system` and DLLs
- Database access (via ODBC or built-in)

### What Needs Building (ToolBook)

- [ ] `.tbk` / `.sbk` format parser (no public specification; requires reverse engineering)
- [ ] Page/object layout extractor
- [ ] OpenScript parser
- [ ] Replacement runtime similar to HyperCard's

## Known Challenges (Both)

- **Resource fork** (HyperCard) — On modern systems resource forks are often stripped from files. HyperCard stacks distributed on CD-ROM may have intact resource forks; digital copies may not.
- **XCMDs/XFCNs** — External commands are compiled 68k or PowerPC code. Cannot be lifted; must be shimmed or stubbed.
- **QuickTime dependencies** — Many HyperCard stacks embed QuickTime movies; ToolBook books reference AVI/QuickTime files. Video assets need conversion.
- **ToolBook format opacity** — Unlike HyperCard, ToolBook's binary format is poorly documented. Significant reverse-engineering effort required.

## References

- [HyperCardPreview (format documentation)](https://github.com/nicklockwood/HyperCardPreview)
- [HyperCard format research (various)](https://hypercard.org/)
- [HyperTalk Reference (Apple documentation, archived)](https://archive.org/details/hypertalk-reference)
