# TyranoBuilder

**Status: Planned** — No implementation started.

## Format

TyranoBuilder games ship as Electron apps or web packages. The script format is **TyranoScript** (`.ks` files), derived from KiriKiri/KAG. Tags drive all game logic:

```
[bg storage="bg01.jpg" time=1000]
[cm]
[chara_show name="hero" face="normal"]
Hello, world.[l]
[chara_hide name="hero"]
[s]
```

- `[tag attr=val ...]` — command tags (show image, play audio, branch, jump, call subroutine)
- Plain text — dialogue output, accumulated until a wait tag (`[l]`, `[p]`, `[s]`)
- `@tag` — shorthand for a tag on its own line
- `*label` — jump targets
- `[if exp="..."]` / `[elsif]` / `[else]` / `[endif]` — conditionals (expressions are JavaScript)
- `[jump storage="scene02.ks" target="*start"]` — cross-file jumps
- `[call storage="sub.ks" target="*label"]` / `[return]` — subroutine calls
- Variables: `f.*` (file-persistent), `sf.*` (shared-persistent), `tf.*` (temp)

Expressions in `exp=` attributes are evaluated JavaScript — the TyranoScript interpreter calls `eval()` on them.

## Runtime

Tag-dispatch interpreter written in JavaScript. Tags are registered handlers; the engine steps through the script, dispatching each tag. Dialogue waits on player input via `[l]` (wait for click) / `[p]` (page break). Each `.ks` file is parsed to a linear tag sequence. Cross-file jumps (`[jump storage=...]`) break the sequential model — the runtime maintains a call stack for `[call]`/`[return]`.

## Lifting Strategy

Full recompilation (Tier 2).

1. Parse `.ks` files to a tag sequence per file
2. Map labels to IR function entry points; `[jump]` → `Op::Br`/tail call; `[call]`/`[return]` → `Op::Call`/`Op::Ret`
3. Dialogue text → `SystemCall("Tyrano.Output", text)` + `Yield` at wait points
4. Choice tags (`[select]`, `[button]`) → `SystemCall("Tyrano.ShowChoices", ...)` + `Yield`
5. Image/audio/effect tags → `SystemCall("Tyrano.Show"/"Tyrano.Play"/...)` stubs
6. Variable reads/writes → `Op::GlobalRef` / `Op::Store` on `f.*`/`sf.*`/`tf.*`
7. JavaScript `exp=` expressions require a JS parser to lift into IR

## What Needs Building

- [ ] `.ks` parser (tag tokenizer + attribute parser)
- [ ] IR emitter: labels → functions, tags → ops, expressions → IR
- [ ] JS expression lifter for `exp=` attributes
- [ ] `SystemCall` namespace: `Tyrano.Output`, `Tyrano.ShowChoices`, `Tyrano.Show`, `Tyrano.Play`, `Tyrano.Transition`
- [ ] Replacement runtime (`runtime/tyranobuilder/ts/`)

## References

- [TyranoBuilder Steam page](https://store.steampowered.com/app/345370/TyranoBuilder_Visual_Novel_Studio/)
- [TyranoScript tag reference](https://tyrano.jp/usage/tech/all_tag)
- [TyranoScript source (GitHub)](https://github.com/ShikemokuMK/tyranoscript)
