# KiriKiri / KiriKiri2 / KiriKiriZ

**Status: Planned** — No implementation started.

## Variants

- **KiriKiri 1** — older, rare
- **KiriKiri 2** (KS/KAG) — dominant format; the vast majority of commercial KiriKiri VNs
- **KiriKiriZ** (KZS) — modern successor; same KAG script format, updated engine

TyranoBuilder is derived from KiriKiri/KAG — `reincarnate-frontend-tyranobuilder` should share most of its parser and IR emitter with this crate.

## Format

### Archive: `.xp3`
KiriKiri packages assets and scripts in `.xp3` archives (often encrypted). A custom extractor is needed before parsing. Several open-source tools exist (`crass`, `GARbro`).

### Script: KAG (KiriKiri Adventure Game Script, `.ks`)

Same tag-based format as TyranoScript (which derives from it):

```
*start
[bg storage="bg01.jpg" time=1000 rule="rule01.bmp"]
[cm]
[chara_show storage="hero_normal.png" layer=0 x=320 y=100]
こんにちは。[l]
[chara_hide layer=0 time=500]
[jump target="*next"]
```

- `[tag attr=val ...]` — command tags
- `*label` — jump targets (entry points)
- Plain text — dialogue
- `[l]` / `[p]` / `[s]` — wait for click / page break / stop
- `[if exp="..."]` / `[elsif]` / `[else]` / `[endif]` — conditionals; `exp=` is TJS2 (not JavaScript)
- `[jump]` / `[call]` / `[return]` — control flow
- `[macro]` / `[endmacro]` — inline macro definitions
- Variables: `f.*` (save-persistent), `sf.*` (system-persistent), `tf.*` (temp)

### Expression language: TJS2

KiriKiri uses **TJS2** (a full scripting language, not JavaScript) for `exp=` attributes and `.tjs` plugin files. TJS2 is a class-based OOP language with C-like syntax. Lifting TJS2 into IR is the main complexity — it's substantially more expressive than TyranoScript's `eval()` shortcut.

`.tjs` files implement engine extensions (custom tags, system hooks). These are typically not needed for game script lifting but may be required for complex games.

## Lifting Strategy

Full recompilation (Tier 2). Shares structure with TyranoBuilder:

1. Extract `.xp3` archive (encryption varies; may require game-specific keys)
2. Parse `.ks` files — same tag tokenizer as TyranoBuilder
3. Labels → IR function entry points; `[jump]`/`[call]`/`[return]` → IR control flow
4. Dialogue → `SystemCall("KiriKiri.Output", text)` + `Yield`
5. Choices → `SystemCall("KiriKiri.ShowChoices", ...)` + `Yield`
6. Media tags → `SystemCall` stubs
7. TJS2 `exp=` expressions → IR ops (requires TJS2 parser; more complex than TyranoScript)

## Relationship to TyranoBuilder

TyranoScript copied the KAG tag format. The `.ks` parser and IR emitter should be shared — `reincarnate-frontend-tyranobuilder` likely becomes a thin wrapper over `reincarnate-frontend-kirikiri` with TyranoScript-specific tag handlers.

## What Needs Building

- [ ] `.xp3` extractor (with common encryption schemes)
- [ ] `.ks` KAG parser (shared with TyranoBuilder)
- [ ] TJS2 expression parser → IR ops
- [ ] TJS2 script (`.tjs`) lifter for plugin-heavy games
- [ ] `SystemCall` namespace: `KiriKiri.Output`, `KiriKiri.ShowChoices`, `KiriKiri.Show`, `KiriKiri.Play`, `KiriKiri.Transition`
- [ ] Replacement runtime (`runtime/kirikiri/ts/`)

## References

- [KiriKiri2 source (GitHub mirror)](https://github.com/krkrz/krkr2)
- [KiriKiriZ source](https://github.com/krkrz/krkrz)
- [TJS2 language reference](https://krkrz.github.io/tjs2doc/)
- [GARbro (archive extractor)](https://github.com/morkt/GARbro)
- [crass (xp3 extractor)](https://github.com/logicplace/crass)
