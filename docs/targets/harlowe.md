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
| `(for:)` | `(loop:)` | ✅ | `emit_for`; `loop` alias wired in translate.rs |
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
| `(border:)` | `(b4r:)` | ✅ | changer; CSS border in context.ts `applyChanger` |
| `(border-colour:)` | `(b4r-colour:)`, `(border-color:)`, `(b4r-color:)` | ✅ | changer; CSS border-color in context.ts |
| `(border-size:)` | `(b4r-size:)` | ❌ | |
| `(corner-radius:)` | — | ❌ | |

### Colour

All go through `lower_value_macro_as_value` → `Harlowe.Engine.value_macro("rgb", ...)` via the `macro_kind()` fallback dispatch in both `emit_macro` and `lower_macro_as_value`.

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(hsl:)` | `(hsla:)` | ✅ | dispatched via `value_macro` → `color_op` → `Colors.hsl/hsla` |
| `(rgb:)` | `(rgba:)` | ✅ | dispatched via `value_macro` → `color_op` → `Colors.rgb/rgba` |
| `(lch:)` | `(lcha:)` | ❌ | `color_op` warns; LCH color space not implemented |
| `(complement:)` | — | ❌ | `color_op` warns; complement not implemented |
| `(palette:)` | — | ❌ | |
| `(gradient:)` | — | ❌ | not implemented in `color_op` |
| `(stripes:)` | — | ❌ | |
| `(mix:)` | — | ❌ | `color_op` warns; color mixing not implemented |

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

All go through `lower_value_macro_as_value` → `Harlowe.Engine.value_macro` via the `macro_kind()` fallback once dispatched.

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(a:)` | `(array:)` | ✅ | explicit in `lower_macro_as_value` |
| `(dm:)` | `(datamap:)` | ✅ | explicit |
| `(ds:)` | `(dataset:)` | ✅ | explicit |
| `(all-pass:)` | `(pass:)` | ✅ | `value_macro` → `collection_op` |
| `(altered:)` | — | ✅ | `value_macro` → `collection_op` |
| `(count:)` | — | ✅ | `value_macro` → `collection_op` |
| `(dm-altered:)` | `(datamap-altered:)` | ✅ | `value_macro` → `Collections.dmAltered` |
| `(dm-entries:)` | `(data-entries:)` | ✅ | `value_macro` → `Collections.dataentries` |
| `(dm-names:)` | `(data-names:)` | ✅ | `value_macro` → `Collections.datanames` |
| `(dm-values:)` | `(data-values:)` | ✅ | `value_macro` → `Collections.datavalues` |
| `(find:)` | — | ✅ | `value_macro` → `collection_op` |
| `(folded:)` | — | ✅ | `value_macro` → `collection_op` |
| `(interlaced:)` | — | ✅ | `value_macro` → `collection_op` |
| `(none-pass:)` | — | ✅ | `value_macro` → `collection_op` |
| `(permutations:)` | — | ✅ | `value_macro` → `collection_op` |
| `(range:)` | — | ✅ | `value_macro` → `collection_op` |
| `(repeated:)` | — | ✅ | `value_macro` → `collection_op` |
| `(reversed:)` | — | ✅ | `value_macro` → `collection_op` |
| `(rotated-to:)` | — | ✅ | `value_macro` → `Collections.rotatedTo` |
| `(rotated:)` | — | ✅ | `value_macro` → `collection_op` |
| `(shuffled:)` | — | ✅ | `value_macro` → `collection_op` |
| `(some-pass:)` | — | ✅ | `value_macro` → `collection_op` |
| `(sorted:)` | — | ✅ | `value_macro` → `collection_op` |
| `(subarray:)` | — | ✅ | `value_macro` → `collection_op` |
| `(split:)` | `(splitted:)` | ✅ | `value_macro` → `Collections.splitStr` |
| `(unique:)` | — | ✅ | `value_macro` → `Collections.unique` |
| `(unpack:)` | — | ❌ | needs special assignment semantics |

### Date and Time

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(current-date:)` | — | ✅ | `value_macro` → `toLocaleDateString()` |
| `(current-time:)` | — | ✅ | `value_macro` → `toLocaleTimeString()` |
| `(monthday:)` | — | ✅ | `value_macro` → `getDate()` |
| `(monthname:)` | — | ✅ | `value_macro` → `toLocaleString("default", { month: "long" })` |
| `(weekday:)` | — | ✅ | `value_macro` → `getDay()+1` (1=Sunday) |
| `(yearday:)` | — | ✅ | `value_macro` → day-of-year calculation |

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
| `(history:)` | — | ✅ | `value_macro` → `State.historyTitles()` |
| `(visited:)` | — | ✅ | `value_macro` → `State.hasVisited()` |
| `(passage:)` | — | ✅ | `value_macro` → `Engine.current_passage()` |
| `(passages:)` | — | ✅ | `value_macro` → all passage info objects |
| `(saved-games:)` | — | ✅ | `value_macro` → `Engine.saved_games()` |
| `(forget-visits:)` | — | ✅ | `emit_macro` |
| `(forget-undos:)` | — | ✅ | `emit_macro` |
| `(metadata:)` | — | ✅ | `value_macro` → `undefined` (not meaningful at runtime) |
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
| `(link-reveal:)` | `(link-append:)` | ✅ | `lower_link_reveal` in translate.rs; `Engine.link_reveal` |
| `(link-repeat:)` | `(linkrepeat:)` | ✅ | routes to `lower_link_rerun`; `Engine.link_rerun` |
| `(link-rerun:)` | — | ✅ | `lower_link_rerun` in translate.rs; `Engine.link_rerun` |
| `(link-goto:)` | — | ✅ | `lower_link_goto_as_value` |
| `(link-reveal-goto:)` | — | ✅ | `lower_link_reveal_goto` in translate.rs; `Engine.link_reveal_goto` |
| `(link-undo:)` | — | ✅ | `lower_link_undo` in translate.rs; `Engine.link_undo` |
| `(link-fullscreen:)` | — | ✅ | `lower_simple_command` → `Engine.link_fullscreen` |
| `(link-show:)` | — | ❌ | not in `macros.rs` |
| `(link-storylet:)` | — | ❌ | not in `macros.rs` |
| `(click:)` | — | ✅ | `lower_click_macro` |
| `(click-replace:)` | — | ✅ | `lower_click_macro` |
| `(click-rerun:)` | — | ✅ | `lower_click_macro`; in macros.rs as Command |
| `(click-append:)` | — | ✅ | `lower_click_macro` |
| `(click-goto:)` | — | ❌ | not in `macros.rs` |
| `(click-undo:)` | — | ❌ | not in `macros.rs` |
| `(click-prepend:)` | — | ✅ | `lower_click_macro` |
| `(action:)` | — | ⚠️ | changer; marks element with `data-action` attribute; no interaction wired |

### Live / Timed

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(live:)` | — | ✅ | `emit_live` / `lower_live_as_value` |
| `(stop:)` | — | ✅ | emits `requestStop` |
| `(event:)` | — | ⚠️ | in `macros.rs` Command; falls to unknown |
| `(after:)` | — | ✅ | `lower_after_macro` → `Engine.after_macro` (setTimeout) |
| `(after-error:)` | — | ❌ | not in `macros.rs` |
| `(more:)` | — | ❌ | not in `macros.rs` |

### Maths

All go through `lower_value_macro_as_value` via `macro_kind()` fallback, then `value_macro` → `math()`.

| Macro | Status | Notes |
|-------|--------|-------|
| `(abs:)`, `(cos:)`, `(exp:)`, `(log:)`, `(log10:)`, `(log2:)`, `(max:)`, `(min:)`, `(pow:)`, `(sign:)`, `(sin:)`, `(sqrt:)`, `(tan:)` | ✅ | dispatched via `value_macro` → `math()` |

### Navigation

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(go-to:)` | `(goto:)` | ✅ | `lower_goto` |
| `(redirect:)` | — | ✅ | falls to unknown in translate.rs, but routes to `Navigation.goto` via unknown_macro fallback |
| `(undo:)` | — | ✅ | explicit arm in `emit_macro` → `Navigation.undo()` |
| `(restart:)` | `(reload:)` | ✅ | explicit arm in `emit_macro` → `Navigation.restart()` |

### Number

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(ceil:)` | — | ✅ | `value_macro` → `math("ceil")` |
| `(floor:)` | — | ✅ | `value_macro` → `math("floor")` |
| `(num:)` | `(number:)` | ✅ | explicit in `lower_macro_as_value` |
| `(random:)` | — | ✅ | explicit in `lower_macro_as_value` |
| `(round:)` | — | ✅ | `value_macro` → `math("round")` |
| `(trunc:)` | — | ❌ | not in `macros.rs` |
| `(clamp:)`, `(lerp:)` | — | ✅ | `value_macro` → `math()` |

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
| `(load-game:)` | `(loadgame:)` | ✅ | `lower_load_game`; both aliases handled |
| `(save-game:)` | `(savegame:)` | ✅ | `lower_save_game`; both aliases handled |
| `(saved-games:)` | — | ✅ | `value_macro` → `Engine.saved_games()` |

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
| `(str:)` | `(string:)`, `(text:)` | ✅ | explicit in `lower_macro_as_value`; `text` alias in macros.rs |
| `(digit-format:)` | — | ❌ | not in `macros.rs` |
| `(joined:)` | — | ✅ | `value_macro` → `collection_op` → `Collections.joined` |
| `(lowercase:)` | — | ✅ | `value_macro` → `collection_op` → `Collections.lowercase` |
| `(lowerfirst:)` | — | ✅ | `value_macro` → `str_op("lowerfirst")` |
| `(plural:)` | — | ❌ | not in `macros.rs` |
| `(source:)` | — | ✅ | `value_macro` → `""` (source not available at runtime) |
| `(split:)` | `(splitted:)` | ✅ | `value_macro` → `Collections.splitStr` |
| `(str-find:)` | `(string-find:)` | ❌ | not in `macros.rs` |
| `(str-nth:)` | `(string-nth:)` | ❌ | not in `macros.rs` |
| `(str-repeated:)` | `(string-repeated:)` | ❌ | not in `macros.rs` |
| `(str-replaced:)` | `(string-replaced:)`, `(replaced:)` | ❌ | not in `macros.rs` |
| `(str-reversed:)` | `(string-reversed:)` | ❌ | not in `macros.rs` |
| `(substring:)` | — | ✅ | `value_macro` → `collection_op` → `Collections.substring` |
| `(trimmed:)` | — | ❌ | not in `macros.rs` |
| `(uppercase:)` | — | ✅ | `value_macro` → `collection_op` → `Collections.uppercase` |
| `(upperfirst:)` | — | ✅ | `value_macro` → `str_op("upperfirst")` |
| `(words:)` | — | ❌ | not in `macros.rs` |

### Styling (Changers)

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(align:)` | — | ✅ | changer arm |
| `(bg:)` | `(background:)` | ✅ | `bg` and `background` both in macros.rs Changer; dispatch via fallback |
| `(box:)` | — | ⚠️ | in macros.rs Changer; `applyChanger` has no `box` case |
| `(button:)` | — | ❌ | not in `macros.rs` |
| `(char-style:)` | — | ⚠️ | in macros.rs Changer; `applyChanger` has no `char-style` case |
| `(collapse:)` | — | ✅ | changer arm |
| `(css:)` | — | ✅ | changer arm |
| `(float-box:)` | — | ⚠️ | in macros.rs Changer; `applyChanger` has no `float-box` case |
| `(font:)` | — | ✅ | changer arm |
| `(hook:)` | — | ⚠️ | in `macros.rs` as Value; treated as a named-hook changer when used with `[hook]` — needs explicit dispatch |
| `(hover-style:)` | — | ✅ | changer arm |
| `(line-style:)` | — | ❌ | not in `macros.rs` |
| `(link-style:)` | — | ❌ | not in `macros.rs` |
| `(nobr:)` | — | ✅ | changer arm |
| `(opacity:)` | — | ✅ | changer arm |
| `(text-colour:)` | `(colour:)`, `(color:)`, `(text-color:)` | ✅ | changer arm |
| `(text-indent:)` | — | ⚠️ | in macros.rs Changer via fallback; `applyChanger` has no `text-indent` case |
| `(text-rotate-x:)` | — | ⚠️ | in macros.rs Changer via fallback; `applyChanger` has no `text-rotate-x` case |
| `(text-rotate-y:)` | — | ⚠️ | in macros.rs Changer via fallback; `applyChanger` has no `text-rotate-y` case |
| `(text-rotate-z:)` | `(text-rotate:)` | ✅ | both aliases in macros.rs and translate.rs changer arm |
| `(text-size:)` | `(size:)` | ✅ | both `text-size` and `size` in macros.rs Changer list |
| `(text-style:)` | — | ✅ | changer arm |
| `(verbatim:)` | `(v6m:)` | ✅ | changer arm |

### Transitions (Changers)

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(transition:)` | `(t8n:)` | ✅ | both in macros.rs and translate.rs changer arm |
| `(transition-delay:)` | `(t8n-delay:)` | ✅ | both in macros.rs and translate.rs changer arm |
| `(transition-time:)` | `(t8n-time:)` | ✅ | both in macros.rs and translate.rs changer arm |
| `(transition-depart:)` | `(t8n-depart:)` | ✅ | both in macros.rs; `t8n-depart` added; dispatch via fallback |
| `(transition-arrive:)` | `(t8n-arrive:)` | ✅ | both in macros.rs; `t8n-arrive` added; dispatch via fallback |
| `(transition-skip:)` | `(t8n-skip:)` | ✅ | both in macros.rs and translate.rs changer arm |
| `(animate:)` | — | ✅ | explicit arm in `emit_macro` → `Engine.animate_macro` |

### Window

| Macro | Aliases | Status | Notes |
|-------|---------|--------|-------|
| `(goto-url:)` | `(open-url:)`, `(openurl:)` | ✅ | explicit arm in `emit_macro` → `Engine.goto_url` |
| `(page-url:)` | — | ❌ | not in `macros.rs` |
| `(scroll:)` | — | ✅ | explicit arm in `emit_macro` → `Engine.scroll_macro` |

## Systematic Gaps

### 1. Dispatch shortcut via `macro_kind()` — ✅ DONE

Both `emit_macro` and `lower_macro_as_value` have a `macro_kind()` fallback in their `_` arm that routes:
- `MacroKind::Changer` → `emit_changer` / `lower_changer_as_value`
- `MacroKind::Value` → `emit_value_macro_standalone` / `lower_value_macro_as_value`
- `MacroKind::Command` → `lower_unknown_macro`

### 2. Missing aliases in `macros.rs` — ✅ DONE

All canonical aliases have been added. Remaining ❌ items above are genuinely unimplemented, not alias gaps.

### 3. Unimplemented interactive/link macros — partially done

Still missing:
- `(link-show:)`, `(link-storylet:)` — no translate.rs wiring or runtime
- `(click-goto:)`, `(click-undo:)` — not in macros.rs
- `(cycling-link:)`, `(seq-link:)` — not in runtime
- `(input:)`, `(force-input:)`, `(force-input-box:)`, `(force-checkbox:)` — partially wired via `input_macro` but runtime incomplete

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
