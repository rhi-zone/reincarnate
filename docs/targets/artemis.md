# Artemis Engine

**Status: Planned** — No implementation started.

## Overview

Artemis Engine is a proprietary Japanese visual novel engine developed by Feng. It is used across a significant portion of Feng's commercial catalog and has been licensed to other studios. Notable titles: Hatsukoi 1/1, Otoboku — Maidens Are Falling for Me! series, Nante Suteki ni Japanesque, and others.

Scripts are tag-based (broadly KAG-inspired in structure) but use Artemis-specific tags, and the distribution format uses `.paz` archives rather than `.xp3`.

## Format

### Archive: `.paz`

Assets and scripts are packed into `.paz` (Paz Archive) files. The format is a custom binary container used by all Artemis Engine games. Encryption keys vary by game — some games use a fixed key embedded in the executable, others derive it from game-specific data. Several game-specific extractors exist; GARbro includes `.paz` support for known key variants.

### Script: `.asf` (Artemis Script File)

Scripts use an `.asf` extension. The format is a tag-based plaintext format superficially similar to KAG, but with an Artemis-specific tag vocabulary. The script is compiled into a binary form (`.asd` — Artemis Script Data) for distribution, so the workflow requires either finding `.asf` source (present during development) or reversing the `.asd` binary format.

Syntax structure:

```
*scene_start
#chara_show name=hero layer=1 x=320 y=100 file=hero_normal.png
こんにちは。[l]
#chara_hide layer=1
#bgm file=bgm01.ogg
[jump target=*scene_next]
```

Elements (note: specific tag names vary by engine version and available documentation):

- `*label` — entry point / jump target
- `#tag param=value ...` or `[tag param=value ...]` — command tags
- Plain text — dialogue output accumulated until a wait
- `[l]` — click-wait
- `[p]` — page break
- `[jump target=*label]` — unconditional jump
- `[call target=*label]` / `[return]` — subroutine call/return
- `[if exp="..."]` / `[else]` / `[endif]` — conditionals with expression strings
- Variable syntax — believed to follow an `f.*` / `sf.*` / `tf.*` convention similar to KAG, but this varies by engine version

**Note:** The Artemis script format is not publicly documented. The tag vocabulary below is based on observed behavior and community research, not an authoritative reference. Verification against actual game scripts is required before implementing.

### Approximate Tag Set

| Tag | Purpose |
|-----|---------|
| `#bg` / `#background` | Set background image |
| `#chara_show` / `#chara_hide` | Show/hide character sprite |
| `#bgm` | Start background music |
| `#se` | Play sound effect |
| `#vo` | Play voice |
| `#fade` | Screen fade transition |
| `[l]` | Wait for click |
| `[p]` | Page break |
| `[jump]` | Unconditional jump |
| `[call]` / `[return]` | Subroutine call/return |
| `[if]` / `[else]` / `[endif]` | Conditional block |

The expression language in `[if exp="..."]` attributes is not well-documented externally. It may be a simple arithmetic/comparison language or a more capable scripting system — this needs to be determined from `.asf` source or `.asd` disassembly.

## Runtime

Tag-dispatch interpreter. The engine steps through the script sequentially, dispatching each tag to its handler. Execution state:

- Script position (file + line)
- Call stack for subroutine calls
- Variable store
- Sprite layer state
- Audio state (BGM, SE, voice)

Click-wait (`[l]`) suspends until player input. Page break (`[p]`) clears dialogue and waits. The engine supports the standard visual novel interaction loop.

## Lifting Strategy

Full recompilation (Tier 2). The tag-based syntax is structurally similar to KiriKiri/KAG — the KAG tokenizer from `reincarnate-frontend-kirikiri` may be reusable if the bracket tag syntax is identical. The primary unknowns are the `.asd` binary format and the expression language in `[if]` conditionals.

1. Extract assets and scripts from `.paz` archives (game-specific decryption key required)
2. If `.asd` binary scripts: reverse the binary format to recover the tag sequence (structure is likely a serialized tag list with string pools)
3. If `.asf` plaintext scripts are recoverable: parse using KAG-derived tokenizer
4. Labels → IR function entry points
5. `[jump]` → `Op::Br`; `[call]`/`[return]` → `Op::Call`/`Op::Ret`
6. `[if exp="..."]` → expression parse + `Op::Br` (requires understanding the expression language)
7. Variable reads/writes → `Op::GlobalRef` / `Op::Store`
8. Dialogue text → `SystemCall("Artemis.Output", text)` + `Yield` at `[l]`/`[p]`
9. Media and display tags → `SystemCall` stubs

The `.asd` binary format is the main blocker. Without a format specification or a working disassembler, the extraction path requires binary reversing on a per-game basis. GARbro's source is the most likely starting point for `.paz` extraction.

## What Needs Building

- [ ] `.paz` extractor (game-specific decryption key discovery; GARbro source as reference)
- [ ] `.asd` binary format reverse engineering and decoder
  - Tag sequence structure
  - String pool layout
  - Expression encoding
- [ ] Script parser / decoder → tag sequence
- [ ] IR emitter:
  - Labels → functions
  - `[jump]` / `[call]` / `[return]` / `[if]` → IR control flow
  - Variable ops → `Op::GlobalRef` / `Op::Store`
  - `[l]` / `[p]` → `Yield`
  - Dialogue text → `SystemCall("Artemis.Output", text)`
  - `[if exp="..."]` → conditional eval (expression language TBD)
- [ ] `SystemCall` namespace: `Artemis.Output`, `Artemis.ShowChoices`, `Artemis.Bg`, `Artemis.Ch`, `Artemis.Bgm`, `Artemis.Se`, `Artemis.Vo`, `Artemis.Fade`
- [ ] Replacement runtime (`runtime/artemis/ts/`)
  - Dialogue display with click-wait / page-break
  - Character sprite layers
  - Background display
  - BGM/SE/voice channels
  - Fade and transition effects
  - Save/load

## Known Unknowns

- The `.asd` binary format has no public specification — structure must be inferred from GARbro source or binary analysis
- The expression language in `[if exp="..."]` is undocumented externally
- Variable naming convention and scope (global/save-persistent/temp) is not confirmed
- The tag set varies across engine versions; no authoritative tag reference exists

## References

- [GARbro (archive extractor, includes Paz support)](https://github.com/morkt/GARbro)
