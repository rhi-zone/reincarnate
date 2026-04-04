# NScripter / NScripter2

**Status: Planned** — No implementation started.

## Variants

- **NScripter** — original engine by Naoki Takahashi; used commercially from the late 1990s through mid-2000s
- **NScripter2** — successor with extended command set; broadly script-compatible
- **ONScripter** — open-source reimplementation (ONScripter, ONScripter-EN, ONScripter-RU); used to run NScripter games on non-Windows platforms. The ONScripter source is the most accessible reference for command semantics.

Notable games: Higurashi no Naku Koro ni, Umineko no Naku Koro ni (original releases), and a large body of commercial Japanese VNs and doujin titles from that era.

## Format

### Archives: `.nsa` / `.sar`

Scripts and assets are packed into `.nsa` (NScripter Archive) or `.sar` (Simple ARchive) files. Both are custom binary formats. `.sar` is simpler (no compression); `.nsa` uses LZSS compression on individual entries. Many games ship `arc.nsa` (or `arc1.nsa`, `arc2.nsa`, etc.) alongside a small executable.

Several extractors exist (e.g. `nsaio`, parts of GARbro).

### Script: `0.txt` / `nscr_sec.dat` / `.nsc`

The main script is typically `0.txt` (plain text, sometimes obfuscated by XOR with a fixed key). Some games split the script across numbered files (`0.txt`, `1.txt`, ...) or use a compiled `.nsc` format. NScripter2 games may use `nscr_sec.dat`.

The script language is line-based. Each line is one command or one line of dialogue:

```
*start
bg "bg_room.bmp",#ffffff,1
ld l,"chara_happy.bmp",1
こんにちは。@
cl all,1
play "bgm01.mp3"
mov %0,10
gosub *sub1
goto *start

*sub1
if %0 > 5 goto *label2
return
```

Key syntax elements:

- `*label` — entry point / jump target
- `bg "file",color,effect` — set background image
- `ld {l|c|r|a} "file",effect` — load character sprite (left/center/right/all)
- `cl {l|c|r|a},effect` — clear sprite layer
- `play "file"` / `stop` — BGM playback
- `playstop` — stop BGM
- `wave "file"` / `waveloop "file"` — SE/voice playback
- `mov %var,expr` — assign integer variable
- `mov $var,"string"` — assign string variable
- `add %var,expr` / `sub`, `mul`, `div`, `mod` — integer arithmetic
- `add $var,"str"` — string concatenation
- `if expr goto *label` / `if expr gosub *label` — conditional branch
- `goto *label` — unconditional jump
- `gosub *label` / `return` — subroutine call/return
- `@` — click-wait (player clicks to continue, text remains)
- `\` — page break (player clicks, screen clears to next page)
- `!` prefix — inline effect command embedded in dialogue text
- `#rrggbb` — inline color change in dialogue

### Variables

NScripter has two fixed variable banks:

- `%0`–`%199` — integer variables (some engines extend this range)
- `$0`–`$199` — string variables
- `%200`–`%999` — array integers (some games use `numalias` / `stralias` to name these)
- `numalias name,%N` / `stralias name,$N` — named aliases for variables

There is no local scope. All variables are global.

### Expression Language

Expressions are evaluated inline in command arguments. Integer expressions support `+`, `-`, `*`, `/`, `%` (modulo), comparisons (`>`, `<`, `>=`, `<=`, `==`, `!=`), and limited bitwise ops. String expressions support concatenation via `add`. No closures, no first-class functions.

## Runtime

Line-by-line interpreter. Execution state is:

- A program counter (line number in the current script file)
- A call stack for `gosub`/`return`
- The variable banks (`%`, `$`)
- Sprite layer state (which images are loaded at which positions)
- BGM/SE playback state

Click-wait (`@`) and page break (`\`) suspend execution until player input. Effect numbers are engine-defined transitions (fade, wipe, etc.) executed synchronously from the script's perspective. There is no async model — the script thread drives everything.

## Lifting Strategy

Full recompilation (Tier 2).

1. Extract assets and scripts from `.nsa`/`.sar` archives
2. Decrypt `0.txt` if XOR-obfuscated (key is usually discoverable from ONScripter source or game-specific research)
3. Parse the line-based script into a sequence of commands per label
4. Labels → IR function entry points
5. `goto` → `Op::Br`; `gosub`/`return` → `Op::Call`/`Op::Ret`
6. `if expr goto` → condition eval + `Op::Br`
7. Dialogue text → `SystemCall("NScripter.Output", text)` + `Yield` at `@` / `\`
8. Variable reads/writes → `Op::GlobalRef` / `Op::Store` on the integer/string banks
9. Media and display commands → `SystemCall` stubs
10. Integer expressions → IR arithmetic ops; comparisons → `Op::Cmp` + branch

The variable model is flat global arrays — these map straightforwardly to IR global slots named `%0`, `%1`, etc. (or their aliases). No inference is needed to determine scope.

## What Needs Building

- [ ] `.nsa` / `.sar` extractor
- [ ] XOR decryption for obfuscated `0.txt` (key discovery may be game-specific)
- [ ] Script parser (`0.txt` line-based command grammar)
  - Command dispatch table for all NScripter built-ins
  - Dialogue line detection vs. command line detection
  - Inline `!` effect syntax within dialogue
- [ ] IR emitter:
  - Labels → functions
  - `goto` / `gosub` / `return` / `if ... goto` → `Op::Br` / `Op::Call` / `Op::Ret`
  - `mov` / `add` / `sub` / `mul` / `div` / `mod` → IR arithmetic on typed integer/string ops
  - `@` / `\` → `Yield`
  - `numalias` / `stralias` → variable rename map applied before IR emit
- [ ] `SystemCall` namespace: `NScripter.Output`, `NScripter.Show`, `NScripter.Hide`, `NScripter.Bg`, `NScripter.Play`, `NScripter.PlaySE`, `NScripter.Effect`
- [ ] Replacement runtime (`runtime/nscripter/ts/`)
  - Dialogue display with click-wait / page-break
  - Sprite layer management (left/center/right slots)
  - Background image display
  - BGM and SE playback
  - Effect/transition system
  - Save/load (serialize variable banks + program counter)

## References

- [ONScripter source (GPLv2)](https://github.com/ogapee/onscripter)
- [ONScripter-EN (English-locale fork)](https://github.com/insani/onscripter-en)
- [GARbro (archive extractor, includes NSA/SAR support)](https://github.com/morkt/GARbro)
