# Director / Shockwave (Lingo)

**Status: Planned** — No implementation started.

## Format

Macromedia/Adobe Director uses several file formats:
- `.dir` — editable Director project (unprotected)
- `.dxr` — protected Director project (Lingo scripts encrypted)
- `.dcr` — Shockwave compressed file (for web delivery, zlib-compressed `.dxr`)
- `.cst` — external cast library (assets and scripts)
- `.cxt` — protected cast library

Director files are binary formats with a RIFF-like chunk structure. The main sections are:
- **RIFX/RIFF** — root container
- **Cast** — cast member library (images, sounds, scripts, fonts, video)
- **Score** — frame-by-frame timeline (channels, keyframes, behaviors)
- **Lingo bytecode** — compiled Lingo scripts in `Lscr` chunks
- **Xtra** headers — extension plugin references

The format has been partially reverse-engineered by the community. [ProjectorRays](https://github.com/ProjectorRays/ProjectorRays) is an open-source Lingo decompiler that can handle both protected and unprotected files.

## Lifting Strategy

Full recompilation (Tier 2).

1. Parse the RIFF container structure — chunk types are 4-char FourCC codes
2. Decode Lingo bytecode from `Lscr` chunks — stack-based VM
3. Emit IR per handler (`on eventName` blocks are Director's equivalent of methods)
4. Identify Director built-in boundaries (member references, sprite operations, Xtra calls)

Lingo is dynamically typed and event-driven. Cast members are referenced by number or name. Sprites are channels on the score timeline.

## What Needs Building

### Format Parser (new crate: `reincarnate-frontend-director`)

- [ ] RIFF/RIFX container parser (ProjectorRays source is the best reference)
- [ ] Cast member extraction: bitmap, sound, script, film loop, palette, shape, text, RTF text, video, Xtra
- [ ] Score extraction: frames, channels, keyframes, transitions
- [ ] `Lscr` bytecode chunk decoder
- [ ] Symbol/name table parsing
- [ ] `KEY*` chunk (key map linking resources to cast)
- [ ] Xtra list extraction (plugin names → identify system vs user-written)

### Lingo Bytecode

Lingo bytecode is a stack-based VM. Known opcodes cover:
- Push/pop literals (int, float, string, symbol, void, empty list/proplist)
- Variable access (local, global, property)
- Arithmetic and comparison operators
- Message dispatch (`callExternalEvent`, `call`, `play`, `pass`)
- List/proplist construction and access
- Sprite/member property get/set
- Control flow (jump, conditional jump)

ProjectorRays has decoded most of the opcode table for Director 6–11. Director 12 (the final version) made minor additions.

### IR Mapping

- Lingo `on message` handlers → functions
- `repeat while` / `repeat with x = 1 to N` → IR loop constructs
- `if ... then ... else ... end if` → branch
- Sprite property access (`sprite(1).locH`) → SystemCall with sprite/channel index
- Cast member property access (`member("foo").width`) → external asset lookup
- `global` declarations → module-level state
- `me` parameter → self reference

### Replacement Runtime (`runtime/director/ts/`)

Director's programming model is fundamentally different from Flash:

**Score-based animation**: The Score is a timeline of frames × channels. Each channel holds a cast member reference with position, scale, ink (blend mode), and transitions. Unlike Flash's display list (tree), Director's score is a flat array of channels.

Key APIs needed:
- [ ] `sprite(n)` — channel accessor (locH, locV, width, height, member, ink, visible, blend, rotation, etc.)
- [ ] `member(ref)` — cast member accessor (name, type, width, height, picture, sound, text, etc.)
- [ ] `frame` — current frame number
- [ ] `go(frame)` / `go to frame N` / `go to movie "name"` — navigation
- [ ] `play(frame)` / `play done` — sub-movie playback
- [ ] `updateStage` — force redisplay
- [ ] `on exitFrame`, `on enterFrame`, `on mouseUp`, `on mouseDown`, `on keyDown` — event handlers
- [ ] `the stageLeft`, `the stageTop`, `the stageRight`, `the stageBottom`, `the stageWidth`, `the stageHeight`
- [ ] `the mouseH`, `the mouseV`, `the mouseDown`, `the mouseUp`
- [ ] `the key`, `the keyCode`, `the shiftDown`, `the commandDown`, `the optionDown`
- [ ] String functions: `length(str)`, `char N of str`, `word N of str`, `line N of str`, `offset(substr, str)`, `chars(str, from, to)`
- [ ] List functions: `list()`, `propList()`, `add()`, `addProp()`, `getAt()`, `getProp()`, `count()`, `sort()`
- [ ] Math: `random(n)`, `sqrt()`, `pi`, trig functions (degree-based like GML)
- [ ] File I/O (via Xtra): basic read/write for save files

**Xtra plugins**: Many Director movies depend on Xtra plugins for:
- Video playback (QuickTime, DirectShow)
- Network access (NetLingo, INetURL)
- Database access
- Custom drawing

Xtras that are simple wrappers (INetURL for HTTP, FileIO for file access) can be shimmed. Heavy Xtras (video codecs) need platform-level support.

## Known Challenges

- **Protected files (`.dxr`/`.dcr`)**: Lingo scripts are XOR-obfuscated. ProjectorRays handles this by brute-forcing the key from known patterns.
- **Cast member encryption**: Some older versions encrypted cast data independently
- **Linked casts**: External `.cst` files may be on CD-ROM paths not present in digital copies
- **Director 4 and earlier**: Pre-bytecode Lingo stored source text directly; these versions need a text parser, not a bytecode decoder
- **Score complexity**: Complex interactive scores with multiple simultaneous channels and behaviors interleave script execution with rendering in ways that are non-trivial to model statically

## References

- [ProjectorRays decompiler (C++)](https://github.com/ProjectorRays/ProjectorRays)
- [LibreShockwave (Java SDK/decompiler/CFG analyzer)](https://github.com/Quackster/LibreShockwave)
- [ScummVM Director engine](https://github.com/scummvm/scummvm/tree/master/engines/director) — most complete existing reimplementation, useful as a reference
- [A Tour of the Adobe Director File Format (nosamu)](https://nosamu.medium.com/a-tour-of-the-adobe-director-file-format-e375d1e063c0) — best available format overview
- Lingo Language Reference (Adobe Director 11 documentation)
