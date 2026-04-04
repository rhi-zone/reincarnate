# Suika2

**Status: Planned** — No implementation started.

## Overview

Suika2 is an open-source visual novel engine (MIT) by Keiichi Tabata, actively maintained. It uses a KAG-compatible `.ks` script format, which means the KiriKiri `.ks` parser from `reincarnate-frontend-kirikiri` should be directly reusable here. The main differences are in the tag set: Suika2 omits TJS2 and the full KiriKiri extension system, and defines its own tag library.

Notable properties:
- Cross-platform (Windows, macOS, Linux, iOS, Android, Web)
- Ships as a standalone executable alongside game data
- Development mode uses plain files; distribution uses `.s2arc` archives

## Format

### Archive: `.s2arc`

Distribution packages use `.s2arc` (Suika2 Archive), a simple custom container. In development mode, assets and scripts are loaded directly from the filesystem. No encryption in the default toolchain. Extraction is straightforward — the format is documented in the Suika2 source.

### Script: `.ks` (KAG-compatible)

Suika2 uses the same KAG tag syntax as KiriKiri2:

```
*start
@bg file=bg_room.png duration=1.0
@ch position=center file=chara_normal.png duration=0.5
Hello, world.
@click
@ch position=center file=none.png duration=0.5
@bgm file=bgm01.ogg
@goto target=*start
```

Syntax rules:

- `*label` — entry point / jump target
- `@tag param=value ...` — command on its own line (KAG `@` shorthand)
- `[tag param=value ...]` — inline command tag (KAG bracket form)
- Plain text lines — dialogue output
- `@click` — wait for player click (equivalent to KAG `[l]`)
- `@goto target=*label` — unconditional jump
- `@gosub target=*label` / `@return` — subroutine call/return
- `@if left=expr op=cmp right=expr label=*target` — conditional branch
- `@set var=name val=expr` — variable assignment
- `@load file=name.ks` — load another script file

Suika2's `@if` is notably simpler than KiriKiri's TJS2 `exp=` conditionals — it takes a three-operand form (`left`, `op`, `right`) rather than an arbitrary expression string. This makes expression lifting substantially easier: no expression language parser is needed.

Variables are named strings prefixed with `%` (e.g. `%SCORE`). Variable values are strings that are coerced to integers for numeric operations. There is no type system.

### Key Suika2 Tags

| Tag | Purpose |
|-----|---------|
| `@bg` | Set background image with transition |
| `@ch` | Show/hide character sprite at a layer position |
| `@bgm` | Start/stop background music |
| `@se` | Play sound effect |
| `@vo` | Play voice audio |
| `@click` | Wait for click |
| `@skip` | Enable/disable skip mode |
| `@menu` | Present a choice menu |
| `@goto` | Jump to label |
| `@gosub` / `@return` | Subroutine call/return |
| `@if` | Conditional branch |
| `@set` | Variable assignment |
| `@load` | Switch to another script file |
| `@video` | Play video |
| `@shake` | Screen shake effect |
| `@chs` | Multi-layer character show (extended form) |

## Runtime

Sequential tag interpreter. Execution state:

- Current script file and line position
- A call stack for `@gosub`/`@return`
- Named variable store (`%var` → string/int)
- Visual layer state (background, up to 8 character positions, effects)
- Audio state (BGM channel, SE channels, voice channel)

`@click` suspends until player input. `@menu` collects choices and branches to the selected label. No coroutine or threading model — the interpreter drives display and audio synchronously, waiting on transitions before proceeding.

## Lifting Strategy

Full recompilation (Tier 2). The `.ks` parser from `reincarnate-frontend-kirikiri` is directly reusable — tag tokenization and attribute parsing are identical. A Suika2-specific IR emitter replaces the KiriKiri tag dispatch table.

1. Extract scripts from `.s2arc` (or read directly in dev mode)
2. Parse `.ks` files using the shared KAG tokenizer
3. Labels → IR function entry points
4. `@goto` → `Op::Br`; `@gosub`/`@return` → `Op::Call`/`Op::Ret`
5. `@if` → condition eval (three-operand comparison) + `Op::Br`
6. `@set` → `Op::Store` on named variable slot
7. Dialogue text → `SystemCall("Suika2.Output", text)` + `Yield` at `@click`
8. `@menu` → `SystemCall("Suika2.ShowChoices", ...)` + `Yield` + branch on result
9. Display/audio tags → `SystemCall` stubs

Because Suika2 uses a simple three-operand `@if` rather than TJS2 expressions, lifting conditionals is direct — no expression language parser is needed beyond basic integer comparison.

## Relationship to KiriKiri

The `.ks` file format is the same tag-based syntax. The KAG tokenizer in `reincarnate-frontend-kirikiri` should be extracted into a shared crate (or re-exported) so `reincarnate-frontend-suika2` can use it without duplicating parsing logic. The divergence point is the tag dispatch table: Suika2 uses its own tag set, and its `@if` has different syntax from KiriKiri's TJS2-based `[if exp="..."]`.

## What Needs Building

- [ ] `.s2arc` extractor
- [ ] Suika2 tag IR emitter (reusing KAG tokenizer from `reincarnate-frontend-kirikiri`):
  - `@bg` / `@ch` / `@chs` → `SystemCall("Suika2.Bg", ...)` / `SystemCall("Suika2.Ch", ...)`
  - `@bgm` / `@se` / `@vo` → `SystemCall("Suika2.Bgm", ...)` / etc.
  - `@click` → `Yield`
  - `@menu` → `SystemCall("Suika2.ShowChoices", ...)` + `Yield`
  - `@goto` / `@gosub` / `@return` / `@if` / `@set` / `@load` → IR control flow and variable ops
- [ ] `SystemCall` namespace: `Suika2.Output`, `Suika2.ShowChoices`, `Suika2.Bg`, `Suika2.Ch`, `Suika2.Bgm`, `Suika2.Se`, `Suika2.Vo`, `Suika2.Video`
- [ ] Replacement runtime (`runtime/suika2/ts/`)
  - Dialogue display with click-wait
  - Background and character sprite layers
  - Choice menu
  - BGM/SE/voice audio channels
  - Save/load (variable store + script position)

## References

- [Suika2 source (MIT)](https://github.com/suika2engine/suika2)
- [Suika2 script tag reference](https://suika2.com/en/reference.html)
