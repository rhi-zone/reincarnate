# Twine — Harlowe

**Status: Active** — Frontend and runtime functional. Test games: 19 titles including arceus-garden, delta-labs-1, shifter-labs-3.

Harlowe is the Twine 2 default format. It is fundamentally different from SugarCube — a proper expression language with hook-based layout, changers, lambda syntax, and a content-as-values model. Its runtime is slower and its save system is barely functional, making it a high-value lifting target.

## Format

Same HTML container as SugarCube. Harlowe passage content uses:

- **Hooks**: `[content]`, `|name>[content]`, `<name|[content]`
- **Macros**: `(macro: args)[hook]`
- **Changers**: applied to hooks — `(if: $x)[...]`, `(color: red)[...]`
- **Changer composition**: `(color: red)+(text-style: "bold")[hook]`
- **Variables**: `$story_var`, `_temp_var`
- **`it` shorthand**: refers to the last-assigned/compared variable in expression context
- **Links**: `(link: "text")[hook]` or `[[text]]`

## Implementation Status

### Extraction & Parsing

- ✅ Engine detection from HTML (Harlowe format prefix)
- ✅ Lexer + parser: hooks, macros, expressions, verbatim backtick spans, links
- ✅ `is ... or ...` / `is not ... or ...` shorthand expansion
- ✅ `it` keyword in `(set:)` context resolved to target variable's current value

### State Macros

- ✅ `(set:)`, `(put:)`, `(move:)`, `(unset:)`, `(forget:)`, `(forget-undos:)`

### Navigation

- ✅ `(go-to:)`, `(undo:)`, `(redo:)`, `(history:)`, `(passage:)`

### Output Macros

- ✅ `(print:)`, `(display:)`, `(show:)`, `(append-with:)`, `(prepend-with:)`

### Changers (Implemented)

- ✅ `(if:)`, `(unless:)`, `(hidden:)`, `(else:)`, `(elseif:)`, `(hook:)`, `(css:)`, `(style:)`, `(class:)`, `(attr:)`
- ✅ `(color:)` / `(text-colour:)`, `(background:)`, `(font:)`, `(text-style:)`, `(text-size:)`
- ✅ `(align:)`, `(float-box:)`, `(rotate:)`
- ✅ `(verbatim:)` — raw text via `<tw-verbatim>`
- ✅ `(transition:)` — wraps hook children in `<tw-transition-container>`
- ✅ Changer composition with `+` via `Harlowe.Engine.plus()` (dispatches by type: changers, arrays, datamaps, numbers)

### UI Macros (implemented, untested against real games)

- ✅ `(enchant:)` / `(enchant-in:)` — DOM enchantment via `<tw-enchantment>`
- ✅ `(columns:)` / `(column:)` — column layout via `<tw-columns>` / `<tw-column>`
- ✅ `(meter:)` — progress meter via `<tw-meter>`
- ✅ `(dialog:)` — modal dialog via `<tw-dialog>` / `<tw-backdrop>` / `<tw-dialog-links>`

### Output Quality

- ✅ Text coalescing pass — adjacent string-literal `text()` calls merged (arceus-garden: 2,974 → 1,874 calls, -37%)
- ✅ O(n) batch AST passes for `fold_single_use_consts` and `narrow_var_scope`
- ✅ Full `tw-*` custom element structure

**Correctness status (arceus-garden):** 3 unknown_macro calls (down from 203)

## Remaining Gaps

### Phase 2: Advanced Features

- ⚠️ **Named hooks** — `|name>[content]` and `?name` hook references (required for `(click:)`, `(replace:)`, `(show:)`, `(hide:)`)
- ⚠️ **`(click: ?hook)[hook]`** — event handler targeting named hooks (blocked on named hooks)
- ⚠️ **`(replace:)`, `(show:)`, `(hide:)`** — DOM manipulation hooks (blocked on named hooks)
- ⚠️ **`(for: each _item, ...$arr)[hook]`** — loop lowering
- ⚠️ **Lambda expressions** — `_x where _x > 5` syntax in parser/translator
- ⚠️ **Complex `'s` possessive chains** — `$obj's (str-nth: $idx)` nested macro in possessive
- ⚠️ **Collection operators** — `contains`, `is in`, `'s`, `of` with full Harlowe semantics
- ⚠️ **Collection constructors** — `(a:)`, `(dm:)`, `(ds:)` (runtime done, parser handles basic cases)
- ⚠️ **`(live: Ns)[hook]` + `(stop:)`** — timed interval (basic IR done, runtime present but untested)
- ⚠️ **`(save-game:)` / `(load-game:)`** — save integration (basic runtime done)
- ⚠️ **`(dropdown:)`** — UI macro

### Untested Macros

The following are implemented but no current test game exercises them:
`(enchant:)`, `(enchant-in:)`, `(columns:)`, `(column:)`, `(meter:)`, `(dialog:)`, `(verbatim:)`, `(transition:)`

To verify correctness, find Harlowe games on IFDB/itch.io that use these macros and add to `~/reincarnate/twine/`.

## References

- [Harlowe Manual](https://twine2.neocities.org/)
- [Harlowe Source](https://bitbucket.org/klembot/twinejs)
- [ADR 001: Harlowe Content Emission via `h` Parameter](../adr/001-harlowe-h-parameter.md)
