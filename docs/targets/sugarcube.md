# Twine — SugarCube

**Status: Active** — Frontend and runtime functional. Test projects: DoL, TRC.

SugarCube is the dominant format for large, complex Twine games. It uses a macro DSL (`<<macro>>`) with a JavaScript-heavy runtime. The SugarCube source is open (MIT) and serves as the canonical reference for semantics.

## Format

- Single HTML file with `<tw-storydata>` containing `<tw-passagedata>` nodes
- `<script>` tags in `tw-passagedata` passages hold user JavaScript
- `<style>` tags hold user CSS
- Passage content uses `<<macro args>>` syntax
- Variables: `$story_var`, `_temp_var`
- Links: `[[text|target]]` or `[[target]]`

SugarCube uses `desugar()` (regex keyword substitution from SugarCube operators to JS) + `eval()` at runtime. It is **not** a proper AST compiler — it's a text preprocessor feeding into `eval`. Our frontend models this correctly by desugaring expressions through the same substitution rules before parsing.

## Macro Implementation Status

Legend: ✅ Implemented · ⚠️ Partial/stub · ❌ Missing

### Variables

| Macro | Status | Notes |
|-------|--------|-------|
| `<<set>>` | ✅ | `lower_set` |
| `<<put>>` | ✅ | alias for `set` |
| `<<unset>>` | ✅ | `lower_unset` |
| `<<capture>>` | ✅ | `lower_capture` |

### Scripting

| Macro | Status | Notes |
|-------|--------|-------|
| `<<run>>` | ✅ | `lower_run` |
| `<<script>>` | ✅ | `lower_script` |

### Display

| Macro | Status | Notes |
|-------|--------|-------|
| `<<print>>` / `<<= >>` / `<<- >>` | ✅ | `lower_print` |
| `<<include>>` | ✅ | `lower_include` |
| `<<nobr>>` | ✅ | `lower_nobr` |
| `<<silent>>` | ❌ | missing — only deprecated `<<silently>>` is handled |
| `<<silently>>` | ✅ | `lower_silently` (deprecated alias) |
| `<<do>>` | ❌ | |
| `<<redo>>` | ❌ | |
| `<<type>>` | ⚠️ | wired via `lower_timed_macro` — typewriter effect stub |

### Control

| Macro | Child macros | Status | Notes |
|-------|-------------|--------|-------|
| `<<if>>` | `<<elseif>>`, `<<else>>` | ✅ | `lower_if` |
| `<<for>>` | `<<break>>`, `<<continue>>` | ✅ | `lower_for` |
| `<<switch>>` | `<<case>>`, `<<default>>` | ✅ | `lower_switch` |

### Interactive

| Macro | Child macros | Status | Notes |
|-------|-------------|--------|-------|
| `<<button>>` | — | ✅ | wired with link macros |
| `<<link>>` | — | ✅ | `lower_link` |
| `<<linkappend>>` | — | ✅ | |
| `<<linkprepend>>` | — | ✅ | |
| `<<linkreplace>>` | — | ✅ | |
| `<<checkbox>>` | — | ✅ | `lower_input_macro` |
| `<<radiobutton>>` | — | ✅ | `lower_input_macro` |
| `<<listbox>>` | `<<option>>`, `<<optionsfrom>>` | ✅ | `lower_input_macro` |
| `<<cycle>>` | `<<option>>`, `<<optionsfrom>>` | ✅ | `lower_input_macro` |
| `<<textarea>>` | — | ✅ | `lower_input_macro` |
| `<<textbox>>` | — | ✅ | `lower_input_macro` |
| `<<numberbox>>` | — | ✅ | `lower_input_macro` |

### Links

| Macro | Status | Notes |
|-------|--------|-------|
| `<<back>>` | ✅ | `lower_nav` |
| `<<return>>` | ✅ | `lower_nav` |
| `<<goto>>` | ✅ | `lower_goto` |
| `<<actions>>` (deprecated) | ❌ | rarely used |
| `<<choice>>` (deprecated) | ❌ | rarely used |

### DOM

| Macro | Status | Notes |
|-------|--------|-------|
| `<<replace>>` | ✅ | `lower_dom_macro` |
| `<<append>>` | ✅ | `lower_dom_macro` |
| `<<prepend>>` | ✅ | `lower_dom_macro` |
| `<<remove>>` | ✅ | `lower_dom_macro` |
| `<<copy>>` | ✅ | `lower_dom_macro` |
| `<<addclass>>` | ✅ | `lower_dom_macro` |
| `<<removeclass>>` | ✅ | `lower_dom_macro` |
| `<<toggleclass>>` | ✅ | `lower_dom_macro` |

### Audio

All audio macros are wired to `lower_audio_macro` → `SugarCube.Audio.<method>(args)` stub. The runtime has a no-op `SimpleAudio.select()` implementation.

| Macro | Status | Notes |
|-------|--------|-------|
| `<<audio>>` | ⚠️ | wired; runtime stub |
| `<<cacheaudio>>` | ⚠️ | wired; runtime stub |
| `<<masteraudio>>` | ⚠️ | wired; runtime stub |
| `<<playlist>>` | ⚠️ | wired; runtime stub |
| `<<createplaylist>>` | ⚠️ | wired; runtime stub |
| `<<createaudiogroup>>` | ⚠️ | wired; runtime stub |
| `<<removeaudiogroup>>` | ⚠️ | wired; runtime stub |
| `<<removeplaylist>>` | ⚠️ | wired; runtime stub |
| `<<waitforaudio>>` | ⚠️ | wired; runtime stub |

### Miscellaneous

| Macro | Child macros | Status | Notes |
|-------|-------------|--------|-------|
| `<<done>>` | — | ✅ | `lower_done` |
| `<<timed>>` | `<<next>>` | ⚠️ | `lower_timed_macro` — structure wired, runtime stub |
| `<<repeat>>` | `<<stop>>` | ⚠️ | `lower_timed_macro` — structure wired, runtime stub |
| `<<stop>>` | — | ✅ | wired |
| `<<widget>>` | — | ✅ | `lower_widget` |

## Systematic Gaps

### `<<silent>>` vs `<<silently>>`

`<<silent>>` is the current (non-deprecated) form. Only `<<silently>>` (the deprecated alias) is handled. Fix: add `"silent"` to the dispatch alongside `"silently"`.

### Timed/repeat runtime

`<<timed>>` and `<<repeat>>` are structurally wired but the runtime implementation is a stub. The `SugarCube.Output.timed_start` / `timed_end` etc. need actual setTimeout/setInterval implementations.

### Audio runtime

All audio macros call `SugarCube.Audio.<method>`. The runtime's `SimpleAudio.select()` is a no-op stub. Full audio requires implementing the AudioRunner chain.

### `<<do>>` / `<<redo>>`

New in SugarCube 2.37+. `<<do>>` displays contents and listens for `<<redo>>` commands to update. Not yet extracted.

## Expression Parsing Status

- ✅ `skipArgs: true` macros: full arg string desugared via `args.full` → `lower_expr` / `lower_raw_statement_str`
- ✅ `parseArgs()` tokenizer macros: `$var`, `` `backtick` ``, `"string"`, `'string'`, numeric, null/true/false/NaN, `settings.x`/`setup.x`
- ✅ Template literal `${...}` preprocessing
- ✅ Single-param arrow functions (`x => expr`)
- ✅ HTML entity decoding (html-escape crate)
- ✅ Custom macro extraction from `Macro.add()` calls in user JS passages
- ✅ `skipArgs` and block/self-closing kind inferred from `Macro.add()` options
- ⚠️ `settings.x` / `setup.x` in `<<case>>` args — falls to Bareword string constant; should evaluate dynamically
- ⚠️ `[[...]]` SquareBracket token in `<<case>>` args — not handled

## Parse Error Status

DoL (Degrees of Lewdity): **4 remaining** oxc parse errors (down from 290)
TRC: **0** parse errors

Remaining 4 in DoL are edge cases in complex embedded JS expressions.

## Runtime Stubs

| Stub | Status |
|------|--------|
| `Scripting.parse()` | returns code unchanged (identity stub) |
| `SimpleAudio.select()` | AudioRunner is no-op |
| `L10n.get()` | returns key as-is |
| `Engine.forward()` | no-op (deprecated but called) |
| `Engine.show()` | no-op; should re-render current passage — fix: call `Navigation.goto(current)` |
| `State.isEmpty()` | always returns `true` — **correctness bug**: first-visit `<<if State.isEmpty()>>` blocks will always fire even after navigation |

## References

- [SugarCube Documentation](https://www.motoslave.net/sugarcube/2/docs/)
- [SugarCube Source (MIT)](https://github.com/tmedwards/sugarcube-2)
