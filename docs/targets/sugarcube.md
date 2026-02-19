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

## Implementation Status

### Extraction

- ✅ HTML extraction via `html5ever` tokenizer (exact browser semantics for `<script>` / `<style>`)
- ✅ Passage content parsing (macros, nested blocks, links, markup)

### Standard Macros

- ✅ `<<if>>`, `<<elseif>>`, `<<else>>`
- ✅ `<<set>>`, `<<unset>>`, `<<run>>`
- ✅ `<<for>>`, `<<break>>`, `<<continue>>`
- ✅ `<<switch>>`, `<<case>>`, `<<default>>`
- ✅ `<<link>>`, `<<button>>`, `<<linkappend>>`, `<<linkprepend>>`, `<<linkreplace>>`
- ✅ `<<include>>`
- ✅ `<<widget>>`
- ✅ `<<nobr>>`, `<<silently>>`, `<<capture>>`

### Expression Parsing

- ✅ `skipArgs: true` macros: full arg string desugared via `args.full` → `lower_expr` / `lower_raw_statement_str`
- ✅ `parseArgs()` tokenizer macros: `$var`, `` `backtick` ``, `"string"`, `'string'`, numeric, null/true/false/NaN, `settings.x`/`setup.x`
- ✅ Template literal `${...}` preprocessing
- ✅ Single-param arrow functions (`x => expr`)
- ✅ HTML entity decoding (html-escape crate)
- ✅ Custom macro extraction from `Macro.add()` calls in user JS passages
- ✅ `skipArgs` and block/self-closing kind inferred from `Macro.add()` options

### Parse Error Status

DoL (Degrees of Lewdity): **4 remaining** oxc parse errors (down from 290)
TRC: **0** parse errors

Remaining 4 in DoL are edge cases in complex embedded JS expressions.

## Remaining Gaps

### Critical

- ⚠️ **User script eval failure** — `__user_script_0` (~45k lines) may fail in browser; cascading failure kills all `window.X = X` assignments. Needs browser testing to identify root cause.

### Parse Edge Cases

- ⚠️ `settings.x` / `setup.x` in `<<case>>` args — should be evaluated dynamically via `evalTwineScript()`, currently falls to Bareword string constant
- ⚠️ `[[...]]` SquareBracket token in `<<case>>` args — not handled (rare in practice)
- ⚠️ Audio macro arg semantics — `<<audio>>`, `<<playlist>>`, `<<cacheaudio>>`, etc. need `parseArgs()` token semantics when those macros are eventually lowered

### Runtime Stubs

- ⚠️ `Scripting.parse()` — returns code unchanged (identity stub)
- ⚠️ `SimpleAudio.select()` — AudioRunner is no-op stub
- ⚠️ `L10n.get()` — returns key as-is
- ⚠️ `Engine.forward()` — no-op (deprecated but called in some games)

### Not Yet Lifted

- ⚠️ Audio system — `<<audio>>`, `<<playlist>>`, `<<cacheaudio>>`, `<<masteraudio>>`, `<<createaudiogroup>>`, etc. currently fall to `Raw`
- ⚠️ `<<append>>`, `<<prepend>>`, `<<replace>>` — DOM manipulation macros
- ⚠️ `<<timed>>` / `<<repeat>>` — time-delayed execution macros

### Passage Rendering Strategy

Not yet implemented: `passage_rendering` manifest option (`auto`/`compiled`/`wikifier`). In `wikifier` mode the emitter would output passage source as string constants for runtime evaluation. Needed for games with dynamically-generated macro content.

## References

- [SugarCube Documentation](https://www.motoslave.net/sugarcube/2/docs/)
- [SugarCube Source (MIT)](https://github.com/tmedwards/sugarcube-2)
