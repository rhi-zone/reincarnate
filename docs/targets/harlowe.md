# Twine — Harlowe

**Status: Active** — Frontend and runtime functional. Test games: 19 titles including arceus-garden, delta-labs-1, shifter-labs-3, artifact.

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

## Dispatch Architecture

Two dispatch paths in `translate.rs`:

- **`emit_macro`** — side-effect context (statement position)
- **`lower_macro_as_value`** — value context (inside expression)

Both have explicit match arms for known macros. The `_` fallback calls `lower_unknown_macro` instead of using `macro_kind()`. This means new macros added to `macros.rs` still fall through unless also wired in `translate.rs`. Fix: use `macro_kind()` in the `_` arm to route changers → `emit_changer`, value macros → `lower_value_macro_as_value`.

## Macro Implementation Status

Legend: ✅ Implemented · ⚠️ Partial/stub · ❌ Missing

### Basics

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(set:)` | — | ✅ | `lower_set` |
| `(put:)` | — | ✅ | `lower_put` |
| `(move:)` | — | ✅ | wired in `emit_macro` |
| `(print:)` | — | ✅ | `emit_print` |
| `(display:)` | — | ✅ | `emit_display` |
| `(if:)` | — | ✅ | `emit_if` |
| `(unless:)` | — | ✅ | `emit_if` |
| `(else-if:)` | `(elseif:)` | ✅ | `emit_if_clauses` (clause of `if` chain) |
| `(else:)` | — | ✅ | `emit_if_clauses` (clause of `if` chain) |
| `(for:)` | `(loop:)` | ✅ | `emit_for` |
| `(either:)` | — | ✅ | `lower_value_macro_as_value` |
| `(cond:)` | — | ✅ | `lower_value_macro_as_value` |
| `(nth:)` | — | ✅ | `lower_value_macro_as_value` |
| `(verbatim:)` | `(v6m:)` | ✅ | changer arm |
| `(verbatim-print:)` | `(v6m-print:)` | ❌ | |
| `(change:)` | — | ❌ | |
| `(enchant:)` | — | ✅ | `lower_enchant_macro` |
| `(enchant-in:)` | — | ✅ | `lower_enchant_macro` |
| `(hooks-named:)` | — | ❌ | needed for dynamic hook targeting |

### Borders

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(border:)` | `(b4r:)` | ❌ | changer; needs CSS border in runtime |
| `(border-colour:)` | `(b4r-colour:)`, `(border-color:)`, `(b4r-color:)` | ❌ | |
| `(border-size:)` | `(b4r-size:)` | ❌ | |
| `(corner-radius:)` | — | ❌ | |

### Colour

All go through `lower_value_macro_as_value` → `Harlowe.Engine.value_macro("rgb", ...)` if the explicit list in `lower_macro_as_value` is reached; otherwise fall to `lower_unknown_macro`.

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(hsl:)` | `(hsla:)` | ⚠️ | in `macros.rs` Value list; needs dispatch fix |
| `(rgb:)` | `(rgba:)` | ⚠️ | in `macros.rs` Value list; needs dispatch fix |
| `(lch:)` | `(lcha:)` | ⚠️ | in `macros.rs` Value list; needs dispatch fix |
| `(complement:)` | — | ⚠️ | needs dispatch fix |
| `(palette:)` | — | ❌ | |
| `(gradient:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(stripes:)` | — | ❌ | |
| `(mix:)` | — | ⚠️ | needs dispatch fix |

### Custom Macros

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(macro:)` | — | ❌ | high complexity; defines new macros at runtime |
| `(output:)` | `(out:)` | ❌ | used inside `(macro:)` bodies |
| `(output-data:)` | `(out-data:)` | ❌ | |
| `(error:)` | — | ❌ | |
| `(datatype:)` | — | ❌ | |
| `(datapattern:)` | — | ❌ | |
| `(partial:)` | — | ❌ | |

### Data Structure

All go through `lower_value_macro_as_value` → `Harlowe.Engine.value_macro` once dispatched.

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(a:)` | `(array:)` | ✅ | explicit in `lower_macro_as_value` |
| `(dm:)` | `(datamap:)` | ✅ | explicit |
| `(ds:)` | `(dataset:)` | ✅ | explicit |
| `(all-pass:)` | `(pass:)` | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(altered:)` | — | ⚠️ | needs dispatch fix |
| `(count:)` | — | ⚠️ | needs dispatch fix |
| `(dm-altered:)` | `(datamap-altered:)` | ⚠️ | needs dispatch fix |
| `(dm-entries:)` | `(data-entries:)` | ⚠️ | needs dispatch fix |
| `(dm-names:)` | `(data-names:)` | ⚠️ | needs dispatch fix |
| `(dm-values:)` | `(data-values:)` | ⚠️ | needs dispatch fix |
| `(find:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(folded:)` | — | ⚠️ | needs dispatch fix |
| `(interlaced:)` | — | ⚠️ | needs dispatch fix |
| `(none-pass:)` | — | ⚠️ | needs dispatch fix |
| `(permutations:)` | — | ⚠️ | needs dispatch fix |
| `(range:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(repeated:)` | — | ⚠️ | needs dispatch fix |
| `(reversed:)` | — | ⚠️ | needs dispatch fix |
| `(rotated-to:)` | — | ❌ | not in `macros.rs` |
| `(rotated:)` | — | ⚠️ | needs dispatch fix |
| `(shuffled:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(some-pass:)` | — | ⚠️ | needs dispatch fix |
| `(sorted:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(subarray:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(unique:)` | — | ❌ | not in `macros.rs` |
| `(unpack:)` | — | ❌ | needs special assignment semantics |

### Date and Time

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(current-date:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(current-time:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(monthday:)` | — | ⚠️ | needs dispatch fix |
| `(weekday:)` | — | ⚠️ | needs dispatch fix |

### Debugging

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(ignore:)` | — | ✅ | no-op |
| `(test-true:)` | — | ❌ | |
| `(test-false:)` | — | ❌ | |
| `(assert:)` | — | ❌ | |
| `(assert-exists:)` | — | ❌ | |
| `(debug:)` | — | ❌ | |
| `(mock-turns:)` | — | ❌ | |
| `(mock-visits:)` | — | ❌ | |
| `(verbatim-source:)` | `(v6m-source:)` | ❌ | |

### Game State

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(history:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(visited:)` | — | ⚠️ | needs dispatch fix |
| `(passage:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(passages:)` | — | ❌ | not in `macros.rs` |
| `(forget-visits:)` | — | ✅ | `emit_macro` |
| `(forget-undos:)` | — | ✅ | `emit_macro` |
| `(metadata:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(seed:)` | — | ❌ | |

### Input and Interface

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(cycling-link:)` | — | ⚠️ | in `macros.rs` Command; falls to unknown |
| `(seq-link:)` | `(sequence-link:)` | ⚠️ | in `macros.rs` Command; falls to unknown |
| `(input:)` | — | ⚠️ | in `macros.rs` Command; falls to unknown |
| `(force-input:)` | — | ⚠️ | in `macros.rs` Command; falls to unknown |
| `(input-box:)` | — | ⚠️ | in `macros.rs` Command; falls to unknown |
| `(force-input-box:)` | — | ❌ | not in `macros.rs` |
| `(checkbox:)` | — | ⚠️ | in `macros.rs` Command; falls to unknown |
| `(checkbox-fullscreen:)` | — | ❌ | not in `macros.rs` |
| `(dropdown:)` | — | ⚠️ | in `macros.rs` Command; falls to unknown |
| `(meter:)` | — | ✅ | `lower_meter_macro` |

### Links

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(link:)` | `(link-replace:)` | ✅ | `lower_link_macro_as_value` |
| `(link-reveal:)` | `(link-append:)` | ⚠️ | in `macros.rs`; falls to unknown |
| `(link-repeat:)` | — | ⚠️ | in `macros.rs`; falls to unknown |
| `(link-rerun:)` | — | ❌ | not in `macros.rs` |
| `(link-goto:)` | — | ✅ | `lower_link_goto_as_value` |
| `(link-reveal-goto:)` | — | ❌ | not in `macros.rs` |
| `(link-undo:)` | — | ⚠️ | in `macros.rs` Command; falls to unknown |
| `(link-fullscreen:)` | — | ❌ | not in `macros.rs` |
| `(link-show:)` | — | ❌ | not in `macros.rs` |
| `(link-storylet:)` | — | ❌ | not in `macros.rs` |
| `(click:)` | — | ✅ | `lower_click_macro` |
| `(click-replace:)` | — | ✅ | `lower_click_macro` |
| `(click-rerun:)` | — | ❌ | not in `macros.rs` |
| `(click-append:)` | — | ✅ | `lower_click_macro` |
| `(click-goto:)` | — | ❌ | not in `macros.rs` |
| `(click-undo:)` | — | ❌ | not in `macros.rs` |
| `(click-prepend:)` | — | ✅ | `lower_click_macro` |
| `(action:)` | — | ❌ | not in `macros.rs`; modifies interaction type of link |

### Live / Timed

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(live:)` | — | ✅ | `emit_live` / `lower_live_as_value` |
| `(stop:)` | — | ✅ | emits `requestStop` |
| `(event:)` | — | ⚠️ | in `macros.rs` Command; falls to unknown |
| `(after:)` | — | ❌ | not in `macros.rs`; delayed content changer |
| `(after-error:)` | — | ❌ | not in `macros.rs` |
| `(more:)` | — | ❌ | not in `macros.rs` |

### Maths

All go through `lower_value_macro_as_value` once dispatched. Currently not in explicit list in `lower_macro_as_value`.

| Macro | Status | Notes |
|-------|--------|-------|
| `(abs:)`, `(cos:)`, `(exp:)`, `(log:)`, `(log10:)`, `(log2:)`, `(max:)`, `(min:)`, `(pow:)`, `(sign:)`, `(sin:)`, `(sqrt:)`, `(tan:)` | ⚠️ | in `macros.rs`; needs dispatch fix |

### Navigation

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(go-to:)` | `(goto:)` | ✅ | `lower_goto` |
| `(redirect:)` | — | ⚠️ | in `macros.rs`; falls to unknown |
| `(undo:)` | — | ⚠️ | in `macros.rs`; falls to unknown |
| `(restart:)` | `(reload:)` | ❌ | not in `macros.rs` |

### Number

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(ceil:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(floor:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(num:)` | `(number:)` | ✅ | explicit in `lower_macro_as_value` |
| `(random:)` | — | ✅ | explicit in `lower_macro_as_value` |
| `(round:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(trunc:)` | — | ❌ | not in `macros.rs` |
| `(clamp:)`, `(lerp:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |

### Patterns

All `(p:)` / `(p-*:)` macros:

| Status | Notes |
|--------|-------|
| ⚠️ | In `macros.rs` as Value; needs dispatch fix to route through `lower_value_macro_as_value` |

Macros: `(p:)`, `(p-either:)`, `(p-opt:)`, `(p-many:)`, `(p-not:)`, `(p-before:)`, `(p-not-before:)`, `(p-start:)`, `(p-end:)`, `(p-ins:)` and their aliases.

### Popup

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(dialog:)` | `(alert:)` | ✅ | `lower_dialog_macro` |
| `(confirm:)` | — | ✅ | `lower_simple_command` |
| `(prompt:)` | — | ✅ | `lower_simple_command` |

### Revision

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(replace:)` | — | ✅ | `lower_dom_macro` |
| `(append:)` | — | ✅ | `lower_dom_macro` |
| `(prepend:)` | — | ✅ | `lower_dom_macro` |
| `(replace-with:)` | — | ❌ | not in `macros.rs` |
| `(append-with:)` | — | ⚠️ | in emit_macro ("append-with") |
| `(prepend-with:)` | — | ⚠️ | in emit_macro ("prepend-with") |
| `(rerun:)` | — | ✅ | `lower_dom_macro` |

### Saving

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(load-game:)` | `(loadgame:)` | ⚠️ | `lower_load_game` — hyphenless alias missing |
| `(save-game:)` | `(savegame:)` | ⚠️ | `lower_save_game` — hyphenless alias missing |
| `(saved-games:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |

### Showing and Hiding

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(hidden:)` | — | ✅ | changer arm |
| `(hide:)` | — | ✅ | `lower_dom_macro` |
| `(show:)` | — | ✅ | emit_macro |

### Sidebar

| Macro | Status | Notes |
|-------|--------|-------|
| `(icon-undo:)` | ❌ | not in `macros.rs` |
| `(icon-redo:)` | ❌ | not in `macros.rs` |
| `(icon-fullscreen:)` | ❌ | not in `macros.rs` |
| `(icon-restart:)` | ❌ | not in `macros.rs` |
| `(icon-counter:)` | ❌ | not in `macros.rs` |

### Storylet

| Macro | Status | Notes |
|-------|--------|-------|
| `(storylet:)` | ⚠️ | in `macros.rs` as Value; needs dispatch fix |
| `(open-storylets:)` | ❌ | not in `macros.rs` |
| `(exclusivity:)` | ⚠️ | in `macros.rs` as Value; needs dispatch fix |
| `(urgency:)` | ❌ | not in `macros.rs` |

### String

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(str:)` | `(string:)`, `(text:)` | ✅ | explicit in `lower_macro_as_value` |
| `(digit-format:)` | — | ❌ | not in `macros.rs` |
| `(joined:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(lowercase:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(lowerfirst:)` | — | ❌ | not in `macros.rs` |
| `(plural:)` | — | ❌ | not in `macros.rs` |
| `(source:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(split:)` | `(splitted:)` | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(str-find:)` | `(string-find:)` | ❌ | not in `macros.rs` |
| `(str-nth:)` | `(string-nth:)` | ❌ | not in `macros.rs` |
| `(str-repeated:)` | `(string-repeated:)` | ❌ | not in `macros.rs` |
| `(str-replaced:)` | `(string-replaced:)`, `(replaced:)` | ❌ | not in `macros.rs` |
| `(str-reversed:)` | `(string-reversed:)` | ❌ | not in `macros.rs` |
| `(substring:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(trimmed:)` | — | ❌ | not in `macros.rs` |
| `(uppercase:)` | — | ⚠️ | in `macros.rs`; needs dispatch fix |
| `(upperfirst:)` | — | ❌ | not in `macros.rs` |
| `(words:)` | — | ❌ | not in `macros.rs` |

### Styling (Changers)

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(align:)` | — | ✅ | changer arm |
| `(bg:)` | `(background:)` | ✅ | changer arm (`background`) |
| `(box:)` | — | ❌ | in `macros.rs` as Changer; not in `emit_macro` changer arm |
| `(button:)` | — | ❌ | not in `macros.rs` |
| `(char-style:)` | — | ❌ | not in `macros.rs` |
| `(collapse:)` | — | ✅ | changer arm |
| `(css:)` | — | ✅ | changer arm |
| `(float-box:)` | — | ❌ | in `macros.rs` as Changer; not in `emit_macro` changer arm |
| `(font:)` | — | ✅ | changer arm |
| `(hook:)` | — | ⚠️ | in `macros.rs` as Value; not in changer arm |
| `(hover-style:)` | — | ✅ | changer arm |
| `(line-style:)` | — | ❌ | not in `macros.rs` |
| `(link-style:)` | — | ❌ | not in `macros.rs` |
| `(nobr:)` | — | ✅ | changer arm |
| `(opacity:)` | — | ✅ | changer arm |
| `(text-colour:)` | `(colour:)`, `(color:)`, `(text-color:)` | ✅ | changer arm |
| `(text-indent:)` | — | ❌ | in `macros.rs` as Changer; not in changer arm |
| `(text-rotate-x:)` | — | ❌ | in `macros.rs` as Changer; not in changer arm |
| `(text-rotate-y:)` | — | ❌ | in `macros.rs` as Changer; not in changer arm |
| `(text-rotate-z:)` | `(text-rotate:)` | ⚠️ | `text-rotate-z` handled; `text-rotate` alias missing |
| `(text-size:)` | `(size:)` | ✅ | changer arm; `size` alias missing |
| `(text-style:)` | — | ✅ | changer arm |
| `(verbatim:)` | `(v6m:)` | ✅ | changer arm |

### Transitions (Changers)

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(transition:)` | `(t8n:)` | ⚠️ | `transition` handled; `t8n` alias missing |
| `(transition-delay:)` | `(t8n-delay:)` | ❌ | not in `macros.rs`; not in changer arm |
| `(transition-time:)` | `(t8n-time:)` | ⚠️ | `transition-time` handled; `t8n-time` alias missing |
| `(transition-depart:)` | `(t8n-depart:)` | ⚠️ | `transition-depart` handled; `t8n-depart` alias missing |
| `(transition-arrive:)` | `(t8n-arrive:)` | ⚠️ | `transition-arrive` handled; `t8n-arrive` alias missing |
| `(transition-skip:)` | `(t8n-skip:)` | ❌ | not in `macros.rs`; not in changer arm |
| `(animate:)` | — | ❌ | not in `macros.rs`; plays a named CSS animation |

### Window

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(goto-url:)` | — | ❌ | not in `macros.rs` |
| `(open-url:)` | — | ❌ | not in `macros.rs` |
| `(page-url:)` | — | ❌ | not in `macros.rs` |
| `(scroll:)` | — | ❌ | not in `macros.rs`; **NOTE**: this is a Command in Harlowe (scrolls named hooks), NOT a changer |

## Systematic Gaps

### 1. Dispatch shortcut via `macro_kind()`

The `_` fallback in `emit_macro` and `lower_macro_as_value` calls `lower_unknown_macro` unconditionally. Fix: route by `macro_kind()`:
- `MacroKind::Changer` → `emit_changer` / `lower_changer_as_value` (has its own generic fallback)
- `MacroKind::Value` → `lower_value_macro_as_value` (generic `value_macro` dispatch)
- `MacroKind::Command` → keep `lower_unknown_macro` for now until explicit handlers exist

This single change eliminates the majority of ⚠️ items above.

### 2. Missing aliases in `macros.rs` and translate.rs

Many official aliases aren't present:
- `(t8n:)`, `(t8n-time:)`, `(t8n-delay:)`, `(t8n-arrive:)`, `(t8n-depart:)`, `(t8n-skip:)` → transition variants
- `(text-rotate:)` → `(text-rotate-z:)`
- `(size:)` → `(text-size:)`
- `(loop:)` → `(for:)`
- `(savegame:)` → `(save-game:)`, `(loadgame:)` → `(load-game:)`
- `(b4r:)` → `(border:)`, etc.
- `(bg:)` → `(background:)` — already handled; `bg` needs to be an alias

Fix: add all canonical aliases to both `macros.rs` and `translate.rs` dispatch (or handle via normalization in the parser).

### 3. Unimplemented interactive/link macros

Needing both translate.rs wiring and runtime implementation:
- `(link-rerun:)`, `(link-repeat:)`, `(link-reveal:)`, `(link-reveal-goto:)`, `(link-undo:)`, `(link-fullscreen:)`, `(link-show:)`, `(link-storylet:)`
- `(click-rerun:)`, `(click-goto:)`, `(click-undo:)`
- `(cycling-link:)`, `(seq-link:)`
- `(checkbox:)`, `(dropdown:)`, `(input:)`, `(input-box:)`, `(force-input:)`

## Extraction & Parsing Status

- ✅ Engine detection from HTML (Harlowe format prefix)
- ✅ Lexer + parser: hooks, macros, expressions, verbatim backtick spans, links
- ✅ `is ... or ...` / `is not ... or ...` shorthand expansion
- ✅ `it` keyword in `(set:)` context resolved to target variable's current value
- ✅ Named hooks `|name>[...]` and `<name|[...]` top-level (inline-only; enchant targeting partially done)
- ✅ `(else-if:)` / `(else:)` collected across newlines and backslash continuations

## Output Quality

- ✅ Text coalescing pass — adjacent string-literal `text()` calls merged
- ✅ O(n) batch AST passes for `fold_single_use_consts` and `narrow_var_scope`
- ✅ Full `tw-*` custom element structure

**Correctness status (arceus-garden):** 0 unknown_macro warnings
**Correctness status (artifact):** 25 unknown macros (see Systematic Gaps above)

## References

- [Harlowe Manual](https://twine2.neocities.org/)
- [Harlowe Source](https://bitbucket.org/klembot/twinejs)
- [ADR 001: Harlowe Content Emission via `h` Parameter](../adr/001-harlowe-h-parameter.md)
