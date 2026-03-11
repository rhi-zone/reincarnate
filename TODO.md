# TODO

Completed items archived in [COMPLETED.md](COMPLETED.md).

Per-engine roadmaps (gaps, runtime coverage, open work) live in [`docs/targets/`](docs/targets/). This file tracks in-flight and near-term work across all active engines.

## IR-Based Modding Framework (Investigation)

**Status: Investigated. Mod is logic-heavy — full IR mutation API required.**

`~/git/bounty/` is the original game hand-decompiled to JS, then **further modified** as a mod. `git diff 98423b5 HEAD` (first commit → current) shows 2262 insertions / 596 deletions across 115 files.

**Delta breakdown:**

New classes (entirely new objects not in original):
- `classes/overview_button.js` (318 lines) — new overview/character screen
- `classes/overview_main.js`, `classes/overview_finished.js` — overview system
- `classes/appearance_reader.js` (163 lines) — appearance screen
- `classes/debug_main.js`, `classes/debug_button.js` — debug tools
- `classes/name_block.js`, `classes/name_input.js` — custom name input
- New rooms: `rooms/appearance.js`, `rooms/debug.js`

New script functions:
- `scripts/main6.js` (495 lines, new) — gangbang encounter system

Modified existing classes:
- `classes/stats.js` (+144 lines) — new stat fields added to `create()`
- `classes/encounter.js` (+85 lines) — new encounter types
- `scripts/main.js` (+102 changes), `main2.js`, `main3.js`, `main5.js` — mixed logic/text

Data changes:
- `data/0.png` — texture atlas updated (new sprites)
- `data/sprites.js`, `data/textures.js` — new sprite/texture entries
- `data/roomdatas.js` — new room placements

**Conclusion:** Logic-heavy mod. But the correct mod surface under Reincarnate's design is **the emitted TypeScript** — lift once, edit the output, forward-port via cherry-pick if upstream updates. No IR mutation API needed. See [ADR 003](docs/adr/003-project-identity-and-mod-surface.md). **Closed.**

## Planned Engines (not yet started)

Full roadmaps in `docs/targets/<engine>.md`. Summary of where each stands:

| Engine | Blocker / next step |
|--------|---------------------|
| [GameMaker 8.x](docs/targets/gamemaker8.md) | New container parser for `.wad`/GM8 format + opcode adjustments; reuses GMS1 translator — test game: Hotline Miami (`~/reincarnate/gamemaker/hotlinemiami/`) |
| [GameMaker 5/6](docs/targets/gamemaker5.md) | Unpack data from PE exe, then parse GM5/6 format; older/simpler opcode set — test game: Seiklus (`~/reincarnate/gamemaker/seiklus/`) |
| GameMaker YYC | YYC-compiled games have no CODE chunk — logic is in native binary. Requires native decompiler pipeline (out of scope for now). Affects: Katana Zero, Picayune Dreams |
| [Director/Shockwave](docs/targets/director.md) | Format parsing (RIFX/Lingo bytecode) — ProjectorRays and ScummVM are references |
| [Ren'Py](docs/targets/renpy.md) | `.rpa` extractor → `.rpyc` decompile (unrpyc) → Ren'Py AST → IR |
| [RPG Maker VX Ace](docs/targets/rpgmaker.md) | `Scripts.rvdata2` extractor (Ruby Marshal) → Ruby AST → IR |
| [RPG Maker MV/MZ](docs/targets/rpgmaker.md) | JSON event command compiler → IR |
| [Inform (Z-machine/Glulx)](docs/targets/inform.md) | Story file parser → bytecode decoder → IR; well-documented specs |
| [Ink by Inkle](docs/targets/ink.md) | `.json` container reader → IR (knots/stitches → functions, choices → Yield) |
| [Visual Basic 6](docs/targets/vb6.md) | PE/VB6 header parser → P-Code decoder → IR |
| [Java Applets](docs/targets/java-applets.md) | JAR/class file parser → JVM bytecode decoder → IR; JVM spec is thorough |
| [Silverlight](docs/targets/silverlight.md) | XAP extractor → PE/CLI parser → IL decoder → IR + XAML parser |
| [HyperCard](docs/targets/hypercard.md) | Stack binary parser → HyperTalk text parser → IR (scripts are source, not bytecode) |
| [WolfRPG](docs/targets/wolfrpg.md) | `.wolf` decryption → event command compiler; `wolfrpg-map-parser` Rust crate exists |
| [SRPG Studio](docs/targets/srpg-studio.md) | `data.dts` decryption → NW.js API shim (engine is already JS) |
| [RAGS](docs/targets/rags.md) | NRBF/SDF decryption → game data extractor; `rags2html` is the reference impl |
| [QSP](docs/targets/qsp.md) | `.qsp` decoder → QSP-lang parser → IR; open-source `libqsp` is reference |
| [PuzzleScript](docs/targets/puzzlescript.md) | Source parser → rule compiler → IR (rule semantics are formally specified) |

---

## Platform Interface Redesign

**Design phase complete** (2026-03-04 through 2026-03-04 follow-up). Canonical interface for all concerns is now in `architecture.md`: Timing, Input, Persistence, Images, Graphics 2D, Graphics 3D, Audio, Window, Clipboard, Network. Three audit rounds (consistency, gaps, devil's advocate × 2) applied.

**Implementation complete** (2026-03-04). Both runtimes updated:
- GML: all six platform concerns rewritten to canonical API; shims updated (requestFrame, per-sound loadAudio, bufferDuration, codeToGmlKeyCode)
- Flash: all six platform concern modules added to `runtime/flash/ts/shared/platform/`; audio aligned (bufferDuration, onVoiceEnd, groups, lazy init); shims converted to classes in `FlashShims` container; `initFlash(canvas)` replaces hardcoded `document.getElementById` at import time
- Flash emitter now threads `FlashShims` through all class constructors (2026-03-04): `_shims` param on every constructor, generic shim system calls rewritten to `this._shims.<system>.<method>()`, `Flash.Object.construct` injects `this._shims`, `Flash.Class.constructSuper` injects `_shims`; backward-compat `export let` bindings and `initFlash()` removed from `runtime/flash/ts/index.ts`

---

## Engine-Specific Logic in `reincarnate-core` (found 2026-03-09 audit)

CLAUDE.md rule: "Frontend/backend specific logic never belongs in `reincarnate-core`."
All of the following violate it and need to move to the respective frontend crates.

- [x] **`type_infer.rs` lines 473–514: Flash and GML dispatch in the shared type inference pass.** (2026-03-11)
  Replaced with `Module.system_call_type_rules` — frontends register `SystemCallTypeRule` entries
  (ResolveClassName, ConstructFromFirstArgType, ResolveGlobalType). Type inference reads the
  map instead of hardcoding engine names. Commit: `4e88003`.

- [x] **`linear.rs` lines 765, 1638–1669: Flash-specific rewrites in the shared linearizer.** (2026-03-11)
  `is_scope_lookup_op()` now parameterized via `LoweringConfig::scope_lookup_systems` (Flash backend
  sets `["Flash.Scope"]`). `Flash.Object deleteProperty/hasProperty` dead code removed (was
  unreachable — Dictionary maps to `Type::Map`, not `Type::Struct`). Law 2 satisfied.

- [x] **`linear.rs` line ~1454–1465: GML `ClassRef as any` widening in the shared linearizer.** (2026-03-11)
  Now gated on `LoweringConfig::wrap_class_refs_as_any` (GML backend sets `true`). Law 2 satisfied.

- [x] **`ast_passes.rs` lines 1804–2140: Flash `ForOfRewrite` / `HasNext2` pattern in shared AST passes.** (2026-03-11)
  Gated behind `LoweringConfig::foreach_rewrite` (Flash backend sets `true`; other engines skip).
  Code stays in shared `ast_passes.rs` but is inert unless opted in — Law 2 satisfied.

- [x] **`ast_passes.rs` line ~170: Harlowe-specific dispatch in `lower_output_nodes`.** (2026-03-11)
  No longer present — `lower_output_nodes` and `Harlowe.H` dispatch removed from ast_passes.rs.

- [x] **`CastKind::AsType` in core IR is AS3-specific.** (2026-03-11)
  Already renamed to `CastKind::NullableCoerce` with language-agnostic doc: "Nullable cast —
  returns null if the value is not an instance of the target type (e.g. AS3 `as`, Kotlin `as?`, C# `as`)".

- [x] **`CmpKind::LooseEq` / `LooseNe` in core IR are JS-specific.** (2026-03-11)
  Renamed to `CoercingEq` / `CoercingNe` with language-neutral docs. Coercing equality
  is a valid cross-language semantic concept (JS, PHP, Perl) — not engine-specific.

- [x] **`datawin` crate missing `reincarnate-` prefix.** (2026-03-11)
  Already named `reincarnate-datawin` in Cargo.toml — the `reincarnate-` prefix is present.

- [ ] **IR lacks aggregate constants — root cause of all "data file" pipeline bypasses.**

  The IR `Constant` enum only has scalar values: `Null`, `Bool`, `Int`, `Float`, `String`.
  There is no `Constant::Array` or `Constant::Map`. Because frontends can't express structured
  compile-time data as IR, they fall back to writing raw TypeScript source blobs into
  `AssetCatalog` and bypassing the entire backend.

  **The correct design:**

  1. Add `Constant::Array(Vec<Constant>)` and `Constant::Map(Vec<(String, Constant)>)` to the
     core IR. These are plain typed data — no engine knowledge required.

  2. Frontends emit compile-time tables as IR constants. GML frontend: instead of calling
     `generate_objects` to write `data/objects.ts`, it emits an IR constant:
     `module.add_const("Classes", Type::ConstMap(String, u32), Constant::Map(pairs))`.
     The GML-specific knowledge (OBJT indices, sprite metadata, room tables) stays in the
     frontend. The IR just sees a typed constant.

  3. The TypeScript backend, when it encounters a `ConstMap<String, u32>` module constant,
     emits `export const Name = { "key": value, ... } as const`. No GML knowledge needed.
     A Rust backend emits `static NAME: &[(&str, u32)] = &[...]`. Neither backend is
     engine-specific.

  4. `AssetCatalog` returns to binary-only (textures, audio). TypeScript source files are
     never stored as blobs — they're always emitted by the backend from IR.

  **What goes away:** `module.object_names: Vec<String>`, `module.sprite_names: Vec<String>`,
  `generate_data_files` in the GML frontend, and all the intermediate "metadata" fields that
  exist only because the IR can't hold the data directly. The `AssetCatalog` TypeScript blob
  hack (`data/objects.ts`, `data/asset_ids.d.ts`, `data/textures.ts`, etc.) is deleted.

  **Current workaround (2026-03-10):** `generate_objects` uses quoted string keys
  (`"3platgen": 515`) to handle GML names that aren't valid TypeScript identifiers. This is
  a local fix; the real fix requires this whole design change.

- [ ] **`Module` struct is a per-engine kitchen sink — Fundamental Law 1 violated.**
  `module.rs` already has 6 GML-specific fields (`room_creation_code`, `initial_room_name`,
  `sprite_names`, `object_names`) and 4 Twine-specific fields (`passage_names`, `passage_tags`,
  `passage_sources`, `passage_storylets`). Each new engine adds more. The IR-as-sole-channel
  law is dead in practice once fields that only one engine reads start accumulating.
  **Fix:** migrate engine-specific metadata into the aggregate-constants design above, or add
  a `Module::metadata: HashMap<String, Box<dyn Any>>` typed-extension slot. Track: every
  new `Module` field added for a new engine is a regression.

- [x] **Flash `NewActivation` emitted as `Record<string,any>` heap object — wrong IR.**
  *(2026-03-11)* For functions without closures: frontend emits `Alloc` per activation slot,
  intercepts `GetSlot`/`SetSlot` → `Load`/`Store`. Mem2Reg promotes to SSA locals.
  `eliminate_dead_activations` removes the now-unread activation object. Zero `newActivation`
  calls in emitted output; all activation slots are direct `let` declarations.
  Functions WITH closures retain `SystemCall("Flash.Scope","newActivation")` + backend
  rewrite pipeline — closures need the scope-chain object until `MakeClosure.captures`
  is implemented.

- [x] **`emit.rs` Flash contamination — remaining `EngineKind::Flash` branches.** (2026-03-11)
  All 9/9 items addressed: traits→`emit_flash_traits.rs`, QN_KEY/registerClass/ctor shim/
  forwarding_setters→`emit_flash_traits`, bang→`ClassDef.zero_initialized`, index sigs→
  `ClassDef.needs_index_signature`, warn_unmapped→`rewrites::flash`, cinit→`MethodKind::StaticInit`.
  Only 3-line call site + guard remains in emit.rs.

- [ ] **`coalesced_decl_types` widening to `Dynamic` is a suppression, not a fix (Law 4).**
  When two branch arms produce different types for the same out-of-SSA variable, widening to
  `Dynamic` silences TS2322/TS2739 but commits to "this is truly dynamically typed" without
  distinguishing it from "our inference inferred wrong types" or "naming collision in the
  coalescer." A type conflict is a diagnostic signal. The correct fix: produce `Type::Union`
  of the conflicting types and let the backend emit a TypeScript union annotation
  (`TimeModel | DefaultDict`). If the union is unsound, the TS error that remains is
  telling us something real about inference quality. (2026-03-11 fix: commit 9f80254.
  Should be revisited once union types are added to the IR.)

---

## Flash Output Quality (2026-03-11 audit)

Audit of generated `.ts` files for human maintainability (see `~/reincarnate/flash/cc/out/`).
Game-logic files (CoC.ts, ConsumableLib.ts) are excellent — <1% artifact names, readable methods.
Library files (List.ts, StyleManager.ts) are poor — 22–40% artifact names, activation record objects.

- [x] **`GetIndex`/`SetIndex` with `Constant::String` key → emit dot notation.** (2026-03-11)
  `is_js_ident()` in `linear.rs`; GetIndex and SetIndex both emit `obj.field` when key is a
  valid JS identifier.

- [x] **IR fields to replace `EngineKind::Flash` branches in class emission.** (2026-03-11)
  `ClassDef.zero_initialized`, `ClassDef.needs_index_signature`, `MethodKind::StaticInit` all added.

- [x] **Recover switch discriminant from chained ternary chains.** (2026-03-11)
  `try_recover_switch_discriminant` + `extract_ternary_chain` in `ast_passes.rs`. Recovers
  both literal constants (e.g. `"["`, `"]"`, `"|"` in Parser.ts) and non-literal expressions
  (e.g. `Keyboard.UP`, `CockTypesEnum.ANEMONE`) as case labels. `JsStmt::Switch` cases are
  `Vec<(JsExpr, Vec<JsStmt>)>` — case labels are arbitrary expressions, not just `Constant`.

- [x] **Format `registerClassTraits(...)` as multi-line.** (2026-03-11)
  Already implemented in `emit_flash_traits.rs` — one trait object per line with 2-space indent.

- [x] **Separate `registerClass`/`registerClassTraits` into a companion file.** (2026-03-11)
  `emit_class` writes registration calls to `traits_out` buffer; file-mode call site generates
  `ClassName_traits.ts` companions with smart imports (registry lookup for in-module interfaces,
  `external_imports` for runtime interfaces, disambiguated TS names). 429 companions emitted;
  barrel exports include them. Scaffold tsconfig change still needed (TODO below).

- [x] **Dead code in Parser.ts `recParser` — structurizer failure.** (2026-03-11)
  Root cause: two bugs in `structurize.rs` when processing general loops (both BrIf targets
  in the loop body).
  1. `find_merge` computed the loop exit block (outside loop body) as the merge point for
     in-loop branches, then `structurize_region(merge_block)` consumed it — adding it to
     `emitted` so `structurize_loop`'s post-loop continuation found it already emitted.
     Fix: reject merge points outside the loop body when inside loop body processing.
  2. `loop_exit_shape` consumed multi-predecessor exit blocks (convergence points for
     multiple break paths). Fix: require exactly 1 predecessor for the first block in
     the exit chain.
  Follow-up: 3 TS2304 (`v54` scoping) fixed in same session — extended
  `compute_cross_scope_defs` to handle multi-use values, not just SE inlines.
  Also still open: `Parser.ts:603–610` dead assignments in switch arms.

## Next Session: Remaining Audit Fixes

Monster function splits — launch 2-3 parallel agents:

1. **`translate_op` (1464 lines)** — `crates/frontends/reincarnate-frontend-gamemaker/src/translate.rs`.
   Single match on GML opcodes. Split by category (arithmetic, stack, comparison, control flow,
   variable access, type ops) as methods on `TranslateCtx` or free functions in sub-modules.

2. **`emit_module_to_dir` (589 lines)** — `crates/backends/reincarnate-backend-typescript/src/emit.rs`.
   God function: file I/O, import collection, module splitting, class grouping, barrel exports.
   Extract `collect_imports()`, `split_modules()`, `write_barrel_exports()`, `write_scaffold()`.

3. **`linear.rs` (4434 lines)** — `crates/reincarnate-core/src/ir/linear.rs`.
   Linearizer, resolver, expression building. Split into `linear/mod.rs`, `linear/resolve.rs`,
   `linear/expr_builder.rs`.

Also: core `pub` → `pub(crate)` visibility tightening, error handling consistency redesign
(save for dedicated session), minor items listed below.

---

## Architecture & Adversarial Audit (2026-03-11) — mostly complete

Ran 3 parallel Opus subagents. 15/18 findings fixed. Remaining items below.

### Law 2 Violations — Engine-Specific Logic in Core (HIGH)

Items already tracked and fixed above (type_infer.rs, linear.rs, ast_passes.rs) are marked [x].
New findings from this audit:

- [x] **`type_infer.rs:590` — `build_global_types` hardcodes `"GameMaker.Global"/"set"`.** (2026-03-11)
  Added `SystemCallTypeRule::GlobalStore { name_arg, value_arg }`. GML frontend registers it.
  `build_global_types` reads from `store_rules` map — no hardcoded strings in core.

- [x] **`int_to_bool.rs:319` — `function_has_with_instances` hardcodes `"GameMaker.Instance"/"withInstances"`.** (2026-03-11)
  Added `Module::callback_return_calls: BTreeMap<(String, String), ()>`. GML frontend
  registers `("GameMaker.Instance", "withInstances")`. `int_to_bool` reads the map.

- [x] **`linear.rs:2456` — `xml_construct_string_coerce()` inserts `.toString()` for Flash XML.** (2026-03-11)
  Gated behind `LoweringConfig::construct_string_coerce` (default false, Flash backend sets true).

- [x] **`call_site_flow.rs:97` — ClassRef narrowing guard comment updated to be engine-neutral.** (2026-03-11)

### Structural — Dead Code & Config Bugs (CRITICAL/HIGH)

- [x] **System traits in `reincarnate-core::system/` are 100% dead code (~350 lines).** (2026-03-11)
  12 traits (`Audio`, `Graphics`, `Input`, `Timing`, `Persistence`, `Dialog`, `Files`,
  `Images`, `Layout`, `Network`, `SaveUi`, `SettingsUi`) — zero implementations, zero
  consumers. These were aspirational API surface that was superseded by the platform
  interface design. Delete or move to `docs/` as design reference.

- [x] **`VALID_PASS_NAMES` missing `call-site-arity-widen`.** (2026-03-11)
  `transform.rs:48-60` — pass exists in pipeline and `PassConfig` but absent from
  `--dump-ir-after` validation list. Users can't dump IR after this pass.

- [x] **`--skip-pass int-to-bool-promotion` silently does nothing.** (2026-03-11)
  `config.rs:144` — docstring lists it as valid, but no `PassConfig` field exists and
  the match arm falls through to `_ => {}`. The pass comes via `extra_passes` from the
  GML frontend, so the skip mechanism can't reach it. Either add a field or remove from docs.

- [x] **Duplicated `--skip-pass` match logic in `config.rs`.** (2026-03-11)
  `PassConfig::from_skip_list` (line 144) and `Preset::resolve` (line 281) have identical
  copy-pasted match blocks. Adding a new pass requires updating both. Extract shared method.

### Panics on Malformed Input (HIGH)

- [x] **53 `args.pop().unwrap()` in system call rewriters** (2026-03-11)
  Extracted `take_arg(args, "call_name")` helper in `rewrites/mod.rs`. All 53 call sites
  in gamemaker.rs, sugarcube.rs, and engine.rs now use it with descriptive call names.

- [x] **Flash translator stack underflow panics** (2026-03-11)
  `resolve_property` and `translate_op` now return `Result<_, String>`. Errors propagate
  to `translate_method_body` which already returned `Result`.

### Code Quality — Monster Functions (MEDIUM, mostly done)

| File | Function | Lines | Status |
|------|----------|-------|--------|
| ~~`translate.rs` (GML)~~ | ~~`translate_instruction`~~ | ~~730~~ | DONE 2026-03-11 — split into 9 themed helpers |
| ~~`emit.rs`~~ | ~~`emit_module_to_dir`~~ | ~~589~~ | DONE 2026-03-11 — split into 6 focused functions |
| `structurize.rs` | `structurize_region_inner` | 494 | Recursive CFG recovery |
| `translate.rs` (GML) | `translate_push_variable` | 498 | GML variable access |
| `translate.rs` (GML) | `translate_pop` | 423 | GML variable store |
| `emit.rs` | `emit_class` | 389 | Class emission + field layout + methods + traits |

Also done: ~~`ast_passes.rs`~~ (2026-03-11 — `ast_passes/` with `AstPass` trait),
~~`linear.rs`~~ (2026-03-11 — `linear/{mod,linearize,resolve,emit,tests}`).
Also done: ~~`runtime.ts` GML~~ (2026-03-11 — split into `object.ts`, `room.ts`, `particles.ts`; 4463→4115 lines).

### Code Quality — Inconsistent Error Handling (MEDIUM, partially fixed 2026-03-11)

- [x] `CoreError` dead variants removed (`UnsupportedFormat`, `Type`, `Codegen`) (2026-03-11)
- [x] `CoreError::Translate` added — distinguishes translation errors from file parse errors (2026-03-11)
- [x] CLI `anyhow!("{e}")` → plain `?` operator — preserves typed `CoreError` in error chain (2026-03-11)
- Frontends still use `Result<_, String>` internally. The 5 error sites (1 Flash, 4 GML)
  produce bytecode-offset messages; a typed `TranslateError` per frontend would add
  structure but the benefit is marginal given so few sites.
- Transform panics are deliberate ICE-style assertions (same pattern as LLVM/rustc).
  These guard internal invariants assumed from prior passes — converting to `Result`
  would propagate error handling through the entire pipeline for bugs that should never
  reach users. Correct as-is.

### ~~Code Quality — `sanitize_ident` Violates CLAUDE.md Rule~~ (FIXED 2026-03-11)

Now uses `unicode_ident::is_xid_start`/`is_xid_continue` per CLAUDE.md rule.

### ~~Code Quality — Silent `_ => {}` Catch-Alls~~ (FIXED 2026-03-11)

Replaced with explicit variant lists in emit.rs type/import collection, call_site_flow.rs,
and mem2reg.rs. New Op/Type variants now trigger compiler errors.

### Visibility — Overly Broad `pub` (LOW, partially fixed 2026-03-11)

- [x] Flash frontend `lib.rs` — internal modules changed to `pub(crate)` (2026-03-11)
- [x] GML frontend `naming` module — changed to `pub(crate)` (2026-03-11)
- [x] Twine frontend internal modules — changed to `pub(crate)` (2026-03-11)
- [x] `reincarnate-core` visibility tightening (2026-03-11):
  `ir::ast_passes` module → `pub(crate)` (all items internal-only).
  `ir::structurize` re-exports: removed `build_cfg`, `compute_dominators_lt`,
  `dominates`, `Cfg`, `Shape`, `BlockArgAssign` (all internal-only). Only
  `structurize()` remains re-exported.

### ~~Duplicated Platform Directories~~ (FIXED 2026-03-11)

Unified 3 diverging files (audio.ts, images.ts, persistence.ts) — GML copies updated
to match Flash's stricter types. All 7 platform files now byte-identical. Drift risk
remains if edited independently; consider symlink or build-time copy if it recurs.

### Test Coverage Gaps (LOW — already partially tracked)

- GML translator: 13 tests for 4021 lines (improved 2026-03-12, up from 5)
- Flash translator: 14 tests for 2302 lines (improved 2026-03-12, up from 4)
- GML rewrites: 4 tests for 1482 lines
- No end-to-end snapshot tests (tracked at "Snapshot tests for both frontends" above)

### Minor

- [x] `datawin` crate: workspace package metadata (`version.workspace = true` etc.) (2026-03-11)
- [x] `datawin` crate: renamed from `reincarnate-datawin` to `datawin` (2026-03-11)
- [x] `unicode-ident` dep in backend: now uses `workspace = true` (was hardcoded `"1"`) (2026-03-11)
- [x] `Preset` unit struct → `resolve_preset()` free function (2026-03-11)
- [x] `Linker` unit struct → `link_modules()` free function (2026-03-11)
- `eprintln!` diagnostics in `builder.rs:436-498` — debug-only (`cfg!(debug_assertions)`),
  5 warnings for stack-depth mismatches. Too few to justify a logging framework; fine as-is.
- ~~`Module` and `Function` derive `Clone` — accidental deep copies~~ Audited 2026-03-11:
  7 clone sites, all necessary (coroutine lowering, test helpers, CLI backend input).
- ~~Bool-coercion passes~~ — analyzed: Flash and GML passes solve different root causes
  (Flash: bool comparisons + void handling; GML: bool arithmetic + block param mismatches).
  Only ~15 lines of `insert_cast_before` helper are duplicated. Not worth extracting until
  a third similar pass emerges.

---

## Warning Diagnostics for Game-Author Bugs

- [x] **Diagnostic infrastructure** — `Diagnostic` type (level: Warning/Info, source location,
  message) threaded through pipeline stages and reported in `check` output. Implemented.
- [x] **`dedup_object_keys` AST pass** — RC0002 diagnostic for duplicate keys in object literals.
  Flash `newObject` rewrite preserves duplicates; AST pass deduplicates with warning.
- [x] **Duplicate-case check on all switch statements** — `check_switch_duplicate_cases` runs
  as the final step of `recover_switch_statements`, catching duplicates in native switches
  AND those introduced by `try_recover_switch_discriminant`. Verified on TelAdre.ts (CC).
- [ ] **Emit diagnostics from more passes** — Audit existing passes for similar
  opportunities (dead code, type mismatches that are game-author bugs not inference failures).
- [ ] **Switch recovery passes belong in core, not TS backend** — `try_recover_switch_discriminant`,
  `try_recover_nested_if_else`, `try_recover_sequential_ifs` operate on the AST without
  TypeScript knowledge. They should be core `ast_passes`, not TS-backend-specific.

---

## Developer Experience / Tooling Gaps (HIGH PRIORITY)

- [x] **Session tooling review** — Completed 2026-02-28. Found gaps below (items added as separate entries).

- [x] **`--dump-function` doesn't match class-qualified names** — Fixed 2026-02-28. `should_dump()` now tries: (1) case-sensitive substring, (2) case-insensitive substring, (3) split-part matching on `.`/`::` — so `"Gun.step"` matches `"Gun::event_step_2"`. Flag ordering was a non-issue (clap handles it fine).

- [x] **`list-functions` subcommand** — Implemented 2026-02-28. `reincarnate list-functions [--filter <pattern>]` prints all IR function names, using same matching as `--dump-function`.

- [x] **`--dump-ir-after <pass>` flag** — Implemented 2026-02-28. Runs pipeline up through named pass, dumps IR (honoring `--dump-function`), then exits. Special value `frontend` dumps raw IR before any transforms. Valid pass names listed in `VALID_PASS_NAMES`.

- [x] **Bytecode disassembler subcommand** — Implemented 2026-02-28. `reincarnate disasm [--function <filter>]` disassembles GML bytecode directly from DataWin (no IR pipeline). Resolves variable names, strings, function names, instance types, and break signal names. Same `--function` filter matching as `--dump-function`. Without `--function`, lists all CODE entry names.

- [ ] **TypeScript error archaeology tool** — No way to go from a specific TS error (file + line) back to the IR value that caused it. An `explain-error <file> <line>` subcommand could resolve the error back to the IR value ID and the GML bytecode offset that produced it.

- [x] **`reincarnate check` workflow adoption** — Documented in CLAUDE.md 2026-02-28.

- [x] **`reincarnate check` curated diagnostic output** — Implemented 2026-02-28. Default output shows counts by code + up to 3 deduplicated examples per code. `--examples N`, `--filter-code`, `--filter-file`, `--filter-message` all implemented. Filters compose (AND), show matching/total counts, affect `--json` output. Pass/fail and baseline comparison always use unfiltered totals.

- [x] **Pipeline fixpoint stress tester** — Implemented 2026-02-28. `reincarnate stress [--runs N] [--skip-pass P] [--preset P]` runs the pipeline N times (default 5), reports fixpoint convergence or oscillation (detects matches against any prior run, not just adjacent), and per-run changed function count.

## TODO.md Staleness Audit (HIGH PRIORITY)

- [x] **Audit TODO.md for stale items** (2026-03-11)
  Sweep complete. Found 4 stale items marked done: flash/memory.ts, project registry,
  HAL audio library, feature-gate CLI. Remaining open items are genuinely unimplemented.

---

## End-to-End Regression Tests

- [ ] **Snapshot tests for both frontends** — No snapshot infrastructure yet.
  - **Flash**: 15 new vN identifiers regressed in `91fe86e` (MethodCall
    refactor). Pre-existing 5 vN (hasNext2 one-shot, split-path phi).
  - **GML**: Reference decompilation at `~/git/bounty/`.

## Unit Test Infrastructure

### GML Translator Tests

- [ ] **Narrow `DataWin` dependency in `TranslateCtx`** — `translate_code_entry`
  takes `ctx.dw: &DataWin` but only uses it for string resolution (3 call sites:
  local var name resolution × 2, push-string operand × 1). Replace with a
  `resolve_string: &dyn Fn(u32) -> &str` field (or equivalent) so tests can
  construct a `TranslateCtx` without a real `DataWin`.

- [x] **GML translator unit tests** — 13 tests added (2026-03-12): 2D array write
  (scalar + non-scalar), compound 2D assignment, argument mapping, PushEnv/PopEnv
  with-block, self-field read/write, call opcode, arithmetic, cmp, bf→br_if,
  push string, push double. `DataWin` dependency narrowed via `string_table` field.

### Flash Translator Tests

- [x] **Flash translator unit tests** — 14 tests added (2026-03-12): return void,
  push int+return, add two bytes, getlocal/setlocal, iffalse/iftrue branch,
  get/set property, callproperty, newarray, newobject, coerce/convert, pushstring,
  pushdouble, dup+swap. Minimal `AbcFile` builder (`make_abc`, `build_abc`,
  `pool_with_qname` helpers) included — no real SWF files needed.

## Type System

### Open

- [ ] **GML instance ID type propagation** — When `instance_create_depth(x, y, d, OFoo)` is
  called, the return type should be inferred as `OFoo` (or `InstanceRef<OFoo>`), not `any`.
  This type must flow through assignments and field accesses — `let enemy = instance_create(..., OEnemy);
  enemy.health -= 1` should type `health` as a field of `OEnemy`. Also: `withInstances(inst, cb)`
  should type `_self` in `cb` as the same type as `inst`. This is critical for emitting maintainable
  code — without it, every cross-instance field access is `any`-typed. Requires the type inference
  pass to understand `instance_create_*` return types, and to propagate the object class through
  the type system as `Struct(className)`.

- [ ] **Flow-sensitive narrowing** — Narrow types after guards
  (`if (x instanceof Foo)` → `x: Foo` in then-branch). Requires per-block type
  environments rather than the current single `value_types` map.
- [ ] **Flash frontend: emit concrete types** — AVM2 bytecode has type
  annotations on locals, parameters, fields, return types. `resolve_type`
  failures cause unnecessary `Dynamic` entries.
- [ ] **Untyped frontend validation** — Test the inference pipeline against a
  fully-untyped IR (simulating Lingo/HyperCard).

### Remaining `:any` analysis (Flash cc-project, 541 total)

Measured after TypeInference + ConstraintSolve + Alloc refinement.

| Category | Count | Root cause |
|----------|-------|------------|
| `any[]` arrays | 185 | Array element type unknown — no element-type inference yet |
| Parameter `: any` | 186 | Untyped function params from ABC metadata gaps |
| Return `: any` | 79 | Functions returning Dynamic (unresolved return types) |
| Field `: any` | 80 | Struct fields without type info (empty class defs, external supers) |
| `let` locals | 11 | Block params where incoming args don't all agree |
| `const` locals | 9 | Genuinely untyped values from calls/block params |

- [ ] **Enum constants not initialized (CockTypesEnum)** — Two-part bug:
  1. `is_redundant_static_assign` strips cinit assignments to const fields even when the
     field declaration has no initializer (`default == None`). Fix: only strip when
     `default.is_some()`. One-line change in `emit.rs:4263`.
  2. But unmasking the cinit body produces `new this(this._shims, "human")` in a static
     block — `this._shims` doesn't exist on the class constructor. The Flash `construct`
     rewrite (`Flash.Object.construct → new ...`) always injects `this._shims` as the first
     arg, but static init context has no instance. Need: detect static init context in the
     rewrite and either skip `_shims` injection or use a different mechanism.
  Game code references `CockTypesEnum.HUMAN`, `CockTypesEnum.ANEMONE`, etc. Runtime error:
  "Can't have an enum without any constants".

### Known Issues

- **Multi-typed locals** — Some Flash locals are assigned different types in
  different branches (e.g. `race` initialized to `0.0` as a sentinel, then
  assigned `this.player.race()` which returns `string`). These correctly stay
  `Dynamic` / `:any` today. For TypeScript this is ugly but functional. For
  Rust emit this is a hard blocker. Options:
  - **Split into separate variables** per SSA def when types disagree
  - **Enum wrapper** — tagged union for the specific types observed
  - **Sentinel elimination** — recognize sentinel-then-overwrite → `Option<T>`
  - Emit union type annotation (`number | string`) instead of `any`

## Runtime Audits (Evergreen)

These are recurring health-checks, not one-off fixes. Run them periodically and update the "last audited" date.

### Module-level mutable state — last audited 2026-03-09

State at module scope prevents two game instances from coexisting on the same page. Any `let` or lowercase-named singleton `const` at the top level of a `.ts` file is a smell.

```bash
# Top-level let declarations (mutable by definition)
grep -rn "^let \|^export let " runtime/ --include="*.ts" | grep -v node_modules

# Top-level singleton instances (lowercase name + new = likely singleton)
grep -rn "^const [a-z].* = new " runtime/ --include="*.ts" | grep -v node_modules
```

Known violations as of 2026-03-09:
- `runtime/flash/ts/flash/memory.ts` — `heap`/typed array views at module scope (AVM2 Alchemy domain memory); fix requires moving to `FlashRuntime` + updating emitter to call `this._rt.memory.load_i8(...)` instead of free function imports

### GML runtime stubs — silent returns audit — last audited: (never)

Many GML built-in stubs added in 2026-02 silently return `0`, `""`, `false`, or `-1` for
functions that require real implementations (collision, path-finding, particle systems,
video, vertex buffers, DS operations, etc.). Per the CLAUDE.md rule, these should throw
`Error("name: not yet implemented")` instead of returning wrong values silently.

```bash
# Find suspicious silent-return stubs in GML runtime
grep -n "{ return 0; }\|{ return \"\"; }\|{ return false; }\|{ return -1; }" \
  runtime/gamemaker/ts/gamemaker/runtime.ts | grep -v "// genuine"
```

Functions to audit (partial list):
- `mp_grid_*` — pathfinding (returns -1/void, but caller uses return value for grid ID)
- `collision_point/circle/ellipse` — collision (returns -4 but caller may iterate over results)
- `path_start/path_get_length` — path following
- `part_*` — particle system state
- `instance_deactivate_all`, `instance_furthest`, `instance_position`
- `ds_priority_*` — DS priority queue (partially implemented)
- `buffer_*` async variants
- `surface_copy`, `vertex_*` — graphics
- `layer_get_depth`, `layer_x`, `layer_y` — layer state queries

Confirmed untracked silent stubs (found 2026-03-09 audit):
- [x] **`asset_get_tags` / `asset_has_tags`** — now `throw Error("... not yet implemented")` (2026-03-10).
- [x] **`file_find_first` / `file_find_next`** — now throw (2026-03-10).
- [x] **`directory_exists`** — now returns `false` with explicit no-op comment (2026-03-10).
- [x] **`buffer_async_group_end`** — now throws (2026-03-10).

### Runtime type widening — last audited: (never)

Review runtime files for type signatures widened to silence TypeScript errors from game code (e.g. `string` → `any`, required param made optional). Such changes hide real bugs.

```bash
# Hunt for any-typed params/returns that shouldn't be
grep -rn ": any\b" runtime/ --include="*.ts" | grep -v node_modules | grep -v "Record<string, any>"
```

Audit targets: `runtime/twine/ts/harlowe/`, `runtime/twine/ts/sugarcube/`.

The rule: never widen types to accommodate buggy game code — TypeScript catching game author mistakes is correct behavior.

---

## Runtime Architecture — High Priority

### Flash runtime: module-level singletons (multi-instance blocker)

All mutable state in the Flash runtime lives at module scope, which means two Flash games cannot run on the same page and a second `createFlashRuntime()` call would stomp on the first. All of the following need to move onto the `FlashRuntime` instance (analogous to how `GameRuntime` was designed from the start).

- [x] **`flash/display.ts` — exported drag state** (`_dragTarget`, `_dragBounds`, `_dragLockCenter`, `_dragOffsetX`, `_dragOffsetY`). Replaced with `_dragStateByStage: WeakMap<Stage, DragState | null>` — now keyed by Stage so each FlashRuntime has isolated drag state.
- [x] **`flash/display.ts` — `_displayState` singleton** — fixed (2026-03-09)
- [x] **`flash/display.ts` — `_timelineFactories` map** — fixed (2026-03-09)
- [x] **`flash/text.ts` — `_textState` singleton** — fixed (2026-03-09)
- [x] **`flash/deflate.ts` — `_deflateState` singleton** — fixed (2026-03-09)
- [x] **`flash/timing.ts` — `TimingState` singleton** — fixed (2026-03-09)
- [x] **`flash/input.ts` — `InputState` singleton** — fixed (2026-03-09)
- [x] **`flash/audio.ts` — `AudioState` singleton** — fixed (2026-03-09)
- [x] **`flash/renderer.ts` — hardcoded canvas** — fixed (2026-03-09)
- [x] **`flash/memory.ts` — shared heap** (2026-03-11, found done in staleness audit)
  `FlashMemory` class with per-instance `ArrayBuffer` heap; instantiated via `FlashShims.create()`.

### Twine platform: module-level singletons (multi-instance blocker)

- [x] All Twine platform singletons fixed (2026-03-09): `save.ts`, `_overlay.ts`, `input.ts`, `layout.ts`, `save-ui.ts`, `settings-ui.ts`

### `flash/utils.ts` — module-level class registries (multi-instance, emitter change required)

`_interfaceRegistry` (WeakMap<Function, Set<Function>>) and `_traitRegistry` (WeakMap<Function, ClassTraits>) are already keyed by constructor identity and thus effectively per-game. But `_classRegistry` (Map<string, Function>) is keyed by qualified name string, meaning `getDefinitionByName("com.example::Foo")` would return whichever game registered that name last.

- [ ] **`flash/utils.ts` — `_classRegistry` string-keyed map** (`getDefinitionByName` returns wrong class if two games use the same qualified name). Fix requires the emitter to generate per-runtime registration calls (`_rt.registerClass(Foo)` instead of module-level `registerClass(Foo)`) — non-trivial emitter change. Low priority: Flash games running simultaneously is rare.
- [ ] **`flash/utils.ts` — `_bindCache` WeakMap** — `as3Bind()` caches bound functions globally across all games. Since it's keyed by `(function, thisArg)` pairs which are game-specific objects, no cross-game pollution occurs in practice. Safe to leave.

### `flash/vector.ts` — `Array.prototype` mutation

- [ ] **Prototype pollution** — `vector.ts` adds `removeAt` and `insertAt` to `Array.prototype` (lines 17–27) as a side effect of import. This bleeds across the entire page. Correct fix: either use a typed `Vector<T>` wrapper class (correct but invasive), or at minimum scope the side effect to only apply when the Flash runtime is initialized rather than on module load.

---

## Third-Party Engine Libraries

Many real-world games embed third-party macro/plugin libraries alongside the engine.
These appear as `unknown_macro` warnings (Twine) or unresolved calls (other engines)
but are *not* missing built-ins — they are authored libraries distributed alongside
the game. Each needs to be identified, understood, and either implemented against the
platform layer or stubbed with a clear diagnostic.

**General strategy:** For each library, read its source embedded in the game, understand
its API, and decide:
1. **Implement** — map to the platform layer (e.g. audio library → platform audio)
2. **Stub** — no-op with a runtime warning (purely visual effects, dev tools)
3. **Warn at compile time** — emit a named diagnostic instead of per-call-site noise

**Detection:** During extraction, scan for library registration patterns
(e.g. `Chapel.Macros.add(` in Harlowe, `Macro.add(` in SugarCube) and record
discovered library names + macro lists in `FrontendOutput` metadata. This enables
precise warnings ("uses HAL audio — implement platform audio shim") rather than
generic unknown-call spam.

**Known libraries observed in the wild:**

- [x] **HAL (Harlowe Audio Library)** (2026-03-11, found done in staleness audit)
  All 7 macros implemented in `harlowe/translate.rs`: `masteraudio`, `track`, `playlist`,
  `group`, `newtrack`, `newplaylist`, `newgroup`.


## Format Spec (game_maker_data.ksy)

- [ ] **Submit to kaitai_struct_formats** — PR to `kaitai-io/kaitai_struct_formats`
  under `game/game_maker_data.ksy`. This is the "never RE it again" step — the spec
  is only findable by the community once it's in the format gallery.
  *Low priority — gated on polish (full chunk coverage, fixture-validated, clean doc strings).*

- [x] **gml_bytecode.ksy** — Separate Kaitai spec for GML instruction encoding.
  Covers: opcode layout (v14 vs v15+ numbering), operand formats (Double/
  Int32/Int64/String/Variable/Int16), Break signal encoding (GMS2.3+ extended
  signals including pushref/chknullish/isstaticok), Dup type-size semantics,
  variable_ref bit layout, instance_type enum, branch offset encoding.
  Lives at `crates/formats/datawin/gml_bytecode.ksy`.

- [ ] **datawin fixture tests (ongoing — goal: 100% format coverage)** — 15 synthetic
  fixtures covering all 20 chunk types now exist in `crates/formats/datawin/tests/fixtures/`
  (57 Rust tests, validated by Kaitai). The goal is complete coverage of every variant the
  format specifies: every field, every conditional branch, every version difference. Keep
  expanding when there's time: GMS2 TXTR entries (currently only GMS1 format tested),
  OBJT with actual events + event actions, ROOM with object instances, physics vertices,
  BC≥17 OBJT `_managed` field, SPRT with multiple tpag entries, multi-entry FONT with
  multiple glyphs, SEQN with GMS2.3+ full entries, etc. Each new case: add builder in
  `gen_fixtures.rs`, regenerate (`cargo run -p datawin --bin gen_fixtures`), add tests in
  `fixture_tests.rs`, update `kaitai_validate.py`. Real-game tests in `tests/read_files.rs`
  still provide broader coverage when run with `--include-ignored`.

## CLI — Project Registry

- [x] **Project registry** (2026-03-11, found done in staleness audit)
  `ProjectRegistry` in `registry.rs` with `~/.config/reincarnate/projects.json`, versioned schema,
  `add`/`remove`/`list` subcommands, bare path positional args, `--all`/`--parallel` emit.

## CLI — Build Configuration

- [x] **Feature-gate frontends, backends, and checkers in the CLI** — All 5 plugin crates are behind
  Cargo feature flags (`frontend-flash`, `frontend-gamemaker`, `frontend-twine`, `backend-typescript`,
  `checker-typescript`), all enabled by default. Build with `--no-default-features` for a minimal binary.

## Future

- [ ] **Game icon extraction → web favicon** — Extract game icon from engine-specific location and
  set as `<link rel="icon">` in the emitted `index.html`. Current state: nothing exists.
  - GameMaker: icon stored in OPTN chunk of data.win; OPTN parser reads flags/constants but not icon binary. Needs Win32 .ico / bitmap parsing. ~200 LOC.
  - Flash: icons are regular image assets (DefineBits/DefineBitsLossless) — no semantic distinction from sprites. Heuristic: smallest/first image.
  - Phase 1 (trivial): Add favicon tag to HTML scaffold (`scaffold.rs`), accept optional icon path param.
  - Phase 2 (easy): Add `icon: Option<AssetMapping>` field to `ProjectManifest`; wire through emission.
  - Phase 3 (medium): Parse GameMaker OPTN icon; add `AssetKind::Icon` variant; mark in Flash extractor.

- [ ] **Split OPFS out of localStorage backend** — `runtime/gamemaker/ts/shared/platform/persistence.ts` currently bakes OPFS in as a fire-and-forget side effect of localStorage writes. Per the persistence design, OPFS should be its own backend composed via `tee(localStorage, opfs)`. Split into `persistence/localstorage.ts` + `persistence/opfs.ts`, wire with `tee()` in `platform/index.ts`. Twine platform (`runtime/twine/ts/platform/persistence.ts`) is localStorage-only and will benefit from the same split.

- [ ] **Cloud save backends** — Platform persistence interface already abstracts save/load/remove; swapping the backend is just a different platform implementation. Candidates: OneDrive, Google Drive, Dropbox, S3/R2/B2. Design: `platform/onedrive.ts`, `platform/gdrive.ts`, etc., each re-exporting the same persistence interface. Config: deployer switches backend by changing re-export in `platform/index.ts`.

- [x] **IR-level closure representation** — `Op::MakeClosure { func, captures }`, `CaptureMode::ByValue`/`ByRef`, `CaptureParam`, and `add_capture_params` on `FunctionBuilder` are all implemented. SugarCube, Harlowe, and GML frontends emit `MakeClosure` with explicit capture lists. TypeScript backend rewrites to IIFE-with-captures (by value) or plain arrow (no captures). DCE tracks captures as uses. `<<capture>>` is a correct no-op — our IIFE pattern already snapshots by value, making SugarCube's workaround unnecessary. Remaining gaps: (1) Flash closures still use `MethodKind::Closure` + TS lexical closure rather than `Op::MakeClosure` — see "Inline closures" below; (2) `CaptureMode::ByRef` is defined but unused.
- [x] **GML default argument recovery pass** — `GmlDefaultArgRecovery` detects the GMS2.3+
  `if (arg === self.undefined) arg = default` IR pattern and folds constant defaults into
  `FunctionSig.defaults`. Also sets type-matched defaults for variadic script params
  (post-inference, reads narrowed types from `value_types`): `""` for string, `false` for
  bool, `0` for number. Dead Estate TS2554: 2069→859, TS2555: 149→0. Bounty TS2555: 251→0.
- [x] **GML param type inference gaps — cross-function call-site narrowing** — 76% of GML
  function `argumentN` params remain `: any` after inference. Dead Estate: 1407/1855 `any`.
  Bounty: 53/77 `any`. Root cause: ConstraintSolve runs per-function and only propagates
  callee param types → caller arg types, never the reverse. Call sites are the biggest
  untapped source of type info.
  **Implemented**: `CallSiteTypeFlow` pass in `transforms/call_site_flow.rs`. Runs between
  TypeInference and ConstraintSolve. Collects argument types from all `Op::Call` and
  `Op::MethodCall` sites, narrows `Dynamic` params when all callers agree on a concrete type.
  Skips self-calls, `CallIndirect`, `SystemCall`, and `Dynamic` args.
- [ ] **CallSiteTypeFlow: union type narrowing (intentionally deferred)** — When callers
  disagree on types (e.g. one passes `string`, another `number`), the param stays `Dynamic`
  rather than being narrowed to `string | number`. Adding union narrowing would require
  downstream work in ConstraintSolve (union unification) and emitter (union type annotations).
  Deferred indefinitely — the single-type narrowing covers the majority of cases where all
  callers agree, and disagreement genuinely suggests `any` is appropriate.
- [ ] **GML fixpoint + CallSiteTypeFlow: instance ID narrowing causes TS2339 spike** —
  With `--fixpoint` enabled on Dead Estate, TS2339 "Property does not exist" errors jump from
  44 (baseline) to 217. Root cause: GML instance IDs are `Float(64)` at the call site (returned
  by `instance_create`, `instance_find`, etc.), so fixpoint propagates `Float(64)` → `number`
  into callee `self` params via ConstraintSolve. The callee then accesses `self.field` directly,
  which TypeScript rejects on `number`. `run_once` on CallSiteTypeFlow helps slightly (217 → 217,
  same pattern) but the bulk comes from ConstraintSolve re-propagating the first round's
  narrowing in subsequent iterations. Two potential fixes:
  1. CallSiteTypeFlow: skip narrowing `Float(64)` → `Float(64)` for GML (instance IDs are
     semantically opaque handles, not plain numbers). But this would block all float narrowing.
  2. Deeper fix: GML instance IDs need a distinct IR type (e.g. `InstanceId`) separate from
     `Float(64)`, so they don't conflate "this is a float" with "this is an object handle".
  The remaining 16 new TS2339s are `string.push` errors — array params narrowed to `string`
  from a control-flow path that initialises as a string. Likely game author bugs surfaced by
  narrowing.
- [ ] **Derive `function_signatures` from `runtime.ts` at build time** — `runtime.json` function
  signatures are manually maintained and drift from `runtime.ts` as the source of truth. The fix:
  add a build step (e.g. `bun` script or `build.rs`) that parses `runtime.ts` with `oxc` or
  `ts-morph`, extracts all public method names + parameter/return types, and generates the
  `function_signatures` section of `runtime.json` automatically. The manual entries then become
  the generated baseline; overrides for special cases (e.g. `classref` params) can be a separate
  `function_signature_overrides` map. This eliminates the gap where new runtime methods are typed
  Dynamic until someone notices.

- [ ] **Frontend-controlled pass ordering** — `extra_passes` are currently appended after the
  entire default pipeline. Frontends should be able to specify where their passes run (e.g.
  "after constraint-solve but before mem2reg"). Current approach works for IntToBoolPromotion
  and GmlLogicalOpNormalize which are fine running last, but won't scale.
- [ ] Rust codegen backend (emit `.rs` files from typed IR — **blocked on multi-typed locals**)
- [ ] wgpu + winit renderer system implementation
- [ ] Web Audio system implementation
- [ ] Native binary decompilation (C/C++, DirectX games with no scripting layer) — requires disassembly + decompilation pipeline rather than bytecode decoding; far harder than any current target; no timeline

## CLI — `reincarnate check` (language-agnostic output validation)

- [x] **`reincarnate check` subcommand** — Implemented: Checker trait in core, TsChecker crate
  (tsgo via bunx), CLI wiring with `--no-emit`, `--json`, `--all` flags. Supports registry names,
  paths, and bare positional args. Prints per-code and per-file breakdown.
- [x] **`--baseline <file>` for check** — `--save-baseline <path>` saves check results as JSON;
  `--baseline <path>` compares against it and reports per-code/per-file deltas. Exits non-zero on regression.

## Diagnostics

- [ ] **External type member validation** — 90 member warnings from types
  inheriting Flash stdlib classes. Need structured member metadata from runtime
  type definitions to validate these.

### Discarded AVM2 Metadata

The Flash frontend discards several categories of ABC metadata that could
improve output fidelity:

- [ ] **Exception handler metadata** — `from`/`to` byte offsets,
  `variable_name`, `type_name`. No try/catch in IR yet.
- [ ] **Class flags** — `is_sealed`, `is_final`.
- [ ] **Protected namespace** — Per-class protected namespace for `protected`
  member visibility.
- [ ] **Trait metadata annotations** — `[Embed]`, `[Bindable]`, custom.
- [ ] **Trait `is_override` / `is_final` flags** — `override` keyword in TS.
- [ ] **Slot/dispatch IDs** — AVM2 vtable layout, irrelevant to decompilation.
- [ ] **`DebugLine` source line info** — Could emit `// line N` or source maps.

## Flash Output Quality

### Correctness

- [ ] **Complex loop decompilation** — Some while-loop bodies have unreachable
  code after `continue`, wrong variable assignments.

### Optimizations — Safe (no semantic change)

- [x] **Redundant type casts** — `strip_redundant_casts` AST pass eliminates
  `as number` when VarDecl/param type already matches. 584 → 108 in Flash.
  Remaining 108 are field accesses where field types aren't tracked.
- [ ] **Constant `rand(n)` where n <= 1** — `rand(1)` always returns 0. Only 1
  known instance (PhoukaScene).
- [ ] **Dead store elimination** — Remove assignments whose values are never
  read. Requires liveness analysis.
- [ ] **Condition inversion** — Structurizer sometimes inverts conditions.
  Heuristic to match original branch polarity.

### Optimizations — Requires alias/purity analysis

- [ ] **Cross-side-effect const sinking** — Sink `const v = expr` past
  side-effecting statements when `expr` is provably pure and unaliased.
- [ ] **Method reference inlining** — `const v = this.method; ... v(args)` →
  `this.method(args)`. Only safe if `method` is not a getter.
- [ ] **Field read deduplication** — `this.x` read twice → read once, reuse.

### Optimizations — Requires control flow analysis

- [ ] **Inline closures** — Some Flash closures still fall back to
  `this.$closureN` field references when `compile_closures()` fails to
  compile them (e.g. dynamic features). These should be diagnosed and fixed
  case-by-case. Twine closures are now fully inlined as of `26ecc6a`.
- [x] **Loop variable promotion** — Fixed `match_compound_assign` and
  `is_var_update` to look through `AsType` casts. Flash: ~65 additional
  while→for promotions. Remaining while-loops use class fields, parameters,
  pre-increment patterns, or complex multi-step increments.

## GameMaker — Version-Gating Audit (HIGH PRIORITY)

The GML frontend does not pass `BytecodeVersion` to any translator, decoder, or rewrite.
Every behavior runs unconditionally regardless of whether the game is GMS1, GMS2, or GMS2.3+.
Many behaviors are version-specific and applying them to the wrong version can produce silent
wrong output. The `BytecodeVersion` is already extracted from GEN8 and available on `DataWin`
(`dw.bytecode_version()`). It needs to be threaded into `TranslateCtx` and all relevant code
paths that make version-sensitive decisions.

**What to audit** — every file under `crates/frontends/reincarnate-frontend-gamemaker/src/`:

- [ ] **`lib.rs`** — `gml_GlobalScript_` skip (added for GMS2.3+ migration pattern; safe for
  GMS1/GMS2 since those games don't have `gml_GlobalScript_*` CODE entries in SCPT, but should
  be documented and guarded with a version assertion). Also: FUNC chunk translation, GLOB chunk
  translation, `scan_code_refs` — all may need version guards.

- [ ] **`decode.rs`** — The `DataType` and `OpCode` decoding may differ between v14 and v15+
  instruction formats — confirm `has_new_instruction_format` is respected.

- [x] **`translate.rs`** — Partial. Completed 2026-02-28:
  - [x] `bytecode_version: BytecodeVersion` field added to `TranslateCtx`; threaded through all
    4 construction sites (3 in lib.rs, 1 in object.rs) plus the with-body inner context.
  - [x] `is_gms23_plus()` added to `BytecodeVersion` (bc >= 17).
  - [x] Break signal -10 (chknullish, 0xFFF6): version guard added — logs warning if seen on
    bytecode_version < 17.
  - [x] Break signal -11 (pushref, 0xFFF5): version guard added — logs warning if seen on
    bytecode_version < 17.
  - [x] Dup swap-mode encoding (`dup_extra != 0`): comment added explaining it is safe
    unconditionally because older versions always have high byte == 0.
  - Remaining items still open:
  - [ ] `filter_reachable` — only needed for GMS2.3+ shared CODE entries
  - [ ] `scan_body_argument_indices` + `argument` captures in with-body — may not apply to GMS1
  - [ ] `InstanceType::Stacktop` (-9) as struct method self-reference — GMS2.3+ construct
  - [ ] `args_count & 0x7FFF` masking — 0x8000 flag meaning differs between versions
  - [ ] Negative instance IDs below -9 (e.g. -16 for `Arg`) — confirm range is version-stable

- [ ] **`object.rs`** — Event type encoding may differ between GMS1 and GMS2; object/event
  structure differences (e.g. `persistent`, `visible` fields, parent indices).

- [ ] **`data.rs`** — Sprite/texture/audio asset structures differ between versions. TXTR
  external textures (GMS2.3+), SEQN/TAGS/ACRV/FEDS chunks — guard against parsing these on
  older versions.

**Action**: Add `bytecode_version: BytecodeVersion` to `TranslateCtx`; add version-check
helper methods (`is_gms23_plus()`) to `BytecodeVersion`; replace any implicit version
assumptions with explicit version guards. Log a warning when a GMS2.3+ feature is detected
on a game that reports an older version.

## GameMaker — New Game Failures (discovered 2026-02-22)

Batch-emitting 7 new games from the Steam library exposed 4 distinct bugs:

### 1. `argument` inside `with`-body panics — blocks 10SecNinjaX, 12BetterThan6, VA-11 HALL-A

- [x] **`argument[N]` accessed inside a `with`-body closes over wrong param index** — **FIXED**.
  Guards added to `translate_push_variable` and `translate_pop` for both the named (`argument0`)
  and stacktop (`argument[N]`) forms: check `locals` map for `_argumentN` capture first (with-body
  case), then bounds-check before calling `fb.param()`. Unblocked 10SecNinjaX, 12BetterThan6.

### 2. TXTR external textures panic — blocks Downwell

- [x] **`txtr.rs:102` slice-end underflow when textures are stored externally** — **FIXED**.
  `texture_data()` now returns `None` when `end < start || end > data.len()`. Downwell now emits
  (remaining errors are runtime API gaps, not parse bugs).

### 3. PE-embedded `data.win` not supported — blocks Momodora RUtM

- [x] **Reader requires FORM at offset 0, but Momodora embeds it in a PE exe** — **FIXED**.
  `DataWin::parse` detects MZ magic and scans all FORM occurrences for the first one whose
  declared size fits within the file (avoids false positives in PE sections). Also fixed 0-size
  CODE/VARI/FUNC chunks for YYC-compiled games (early-return empty structs). Momodora now emits.

### 4. Forager parse error at EOF / Risk of Rain CODE chunk empty

- [ ] **Forager `game.unx` hits unexpected EOF while parsing** — the reader reaches absolute
  file offset 81446624 (= EOF) and attempts a 4-byte read. All top-level chunks parse correctly;
  the failure is inside a chunk's content parser. Likely the CODE chunk bytecode decoder reading
  a function entry whose stated length extends to exactly EOF, then attempting to read past.
  Needs `--dump-ir` + targeted investigation.

- [x] **Risk of Rain `game.unx` has empty CODE/VARI/FUNC chunks (YYC-compiled)** — **FIXED**
  by the same early-return guards added for Momodora (Bug 3). Both GMS1 and GMS2 YYC games now
  parse correctly with empty bytecode chunks.

### 5. Sprite name bracket notation missing for access side — blocks Nubby's, Mindwave, MaxManos2

- [x] **`Sprites.3DPegBase` emitted instead of `Sprites["3DPegBase"]`** — **FIXED** in commit
  `9e1b5d7`. Both `resolve_sprite_constant` (emit.rs) and `try_resolve_sprite_assign` (rewrites/
  gamemaker.rs) apply `is_valid_js_ident` and use bracket notation when false. Remaining errors
  in Nubby's/MINDWAVE/MaxManos2 are Bug 7 (struct field access on Number) and runtime API gaps.

### 6. pushac/popaf array capture coerces array to int — Schism syntax errors

- [ ] **`int(argument0)[FxDoomApply.gunMod] = 0` produces TS1005 syntax error** — `pushac`
  captures `int(argument0)` as the "array reference", but `argument0` is actually an array
  passed by reference. The `coerce v1, i32` from a preceding `Push Variable(argument0,
  type=Int32)` converts the array to an integer before `pushac` saves it. `popaf` then calls
  `set_index(int_value, array_value, 0)` which the TS printer emits as `int(argument0)[array]
  = 0` — invalid JS. Root cause: type mismatch between the Int32 push type and the actual
  array type of argument0. Fix requires either: (a) not coercing when the value is used as a
  pushac target, or (b) the TS printer detecting integer-as-collection in SetIndex and routing
  to a GameMaker.setIndex runtime call. Only 6 errors in Schism, low priority.

### 7. Dead Estate / GML open bugs

- [ ] **Inject `+expr` for Bool operands in GML arithmetic (TS2362/TS2365)**
  In `rewrites/gamemaker.rs`, detect `JsExpr::BinOp` nodes where one operand has a `Bool`-typed IR value
  and the operator is arithmetic (`+`, `-`, `*`, `/`, `%`, etc.). Wrap the Bool operand with a unary
  `+` expression (`JsExpr::UnaryOp("+", expr)`). This is a TypeScript-emission-layer fix only — no IR
  changes needed. Must check that the unary `+` is inserted at the correct sub-expression (the Bool
  operand, not the whole expression).

- [ ] **GmlLogicalOpNormalize false positive (3 instances: OAdultLevi:98, MainMenu:381, _init.ts:3428)**:
  else_target ≠ merge_target so the guard doesn't fire, but the pattern is still an if-then-else
  not `||`. The fundamental problem: GmlLogicalOpNormalize runs AFTER TypeInference, so replacing
  a block arg changes the arg type without updating `value_types` for the block param. Refreshing
  `value_types` fixes the declaration but then passing `boolean | number` to `number` params causes
  TS2345 regressions (net +1 error). Needs a smarter guard OR fixing at the TypeInference level
  (re-run TypeInference after extra passes, or propagate type changes through the pass).

- [ ] **Direct bool→number field/variable assignment (3 instances: PassageGate:27, NewCharacterSelect:551, ParentMenu:204)**:
  Game author assigns a comparison result to a `number`-typed field (e.g. `image_index = y > height/2`).
  This is a GML idiom (comparison gives 0/1). Our `image_index` struct field is declared `number` in the
  runtime types. Fix would require widening affected field types to `number | boolean` in the runtime,
  or accepting these 3 errors as intended diagnostics (game author using bool as int).

- [ ] **TS2367: `return value` inside `with` block (5 errors)** — Our `withInstances` callback
  model doesn't propagate return values — the function is typed as `void` even though it should return
  a number. Correct fix: detect exit PopEnv within body_insts (Branch offset ≈ -4194304 sentinel).
  Truncate body_insts there; keep post-break code in the outer function. Requires the outer function
  to branch to the post-break code after `withInstances` call (needs block map entries for post-break
  offsets). Complex because the post-break code must also return the value captured in the local.

- [ ] **TS2554: GML loose calling convention (5 errors)** — `loadSetting`, `getScreenType`,
  `getPiecesWidth`, `getPiecesHeight`, `drawTextPieces` called with more args than their signatures
  declare. GML extra args accessible via `argument[N]`. Fix: `CallSiteArityWiden` IR transform pass
  — scan all `Op::Call` sites; if any passes more args than the declared param count, append
  `argumentN: T = default` params to the `FunctionSig`.

- [ ] **Unreachable code after return/continue in switch cases (TS7027)** — The switch-case emitter
  emits both a `continue`/`return` AND a trailing `break` for the same case. Fix: suppress the
  auto-generated `break` when the case body already ends with `return`, `continue`, or `break`.

- [ ] **Undeclared variables — `vNNN` and named vars (TS2304)** — Three sub-patterns:
  1. SSA register names in ternary chains — structurizer's ternary/select lowering doesn't ensure
     all referenced values have declarations in the enclosing scope.
  2. `spd` in with-body (Barnacle.ts:142-143) — linearizer or translator lost the assignment target.
  3. `pass` in controller objects (OAnya*Controller.ts) — same category as `spd`.
  All are correctness bugs — emitted code references variables that don't exist.

- [ ] **TS7053: `int(x)[field]` indexing number** — 2D array access patterns where the array variable
  is coerced to `int` before being used as an array target. The `coerce i32` instruction wraps the
  array in `int()` when it should access the array directly. Fix in GML translator or as a rewrite pass.

- [ ] **TS7053: struct field access on number-typed vars** — `v['fieldName']` where `v` is typed as
  `number` rather than `any`. Root cause: instance variables that hold object references (e.g. created
  by `instance_create_depth`) are typed as `number` (the raw instance ID) rather than as the class type.
  Fix requires GML instance ID type propagation.

- [ ] **TS2304: `dynamic` identifier (Dead Estate)** — GML `dynamic` keyword used as an identifier
  in GML 2.3+ struct definitions. Translator emits `dynamic` as a bare identifier which TypeScript
  doesn't recognize. Needs investigation — may be a translator keyword lookup bug.

- [ ] **TS2345: OBJT class constructors passed to user scripts typed as `number`** — Game-author
  pattern: user scripts inferred `argument0: number` from arithmetic context now receive `typeof
  ClassName` (class constructor). Future: CallSiteTypeFlow could propagate `typeof GMLObject` into
  user script params.

- [ ] **TS2339: Missing particle system properties** — `part_system_depth` and
  `part_system_automatic_draw` not declared on the particle system type. Need to add to GMLObject
  type definition or particle system type.

- [ ] **TS2552: `sarr` in with-body** — `sarr.length` on `_init.ts:6198` should be `self.sarr.length`.
  Adjacent references correctly use `self.sarr[...]` — one reference dropped the `self.` prefix.
  With-body codegen bug; needs investigation in GML translator.

- [ ] **TS2308/TS2300/TS2440: struct constructor / object class name collision in barrel** —
  `_init.ts` exports `function TextEffect(...)` (GML struct ctor) and `objects/TextEffect.ts` exports
  `class TextEffect extends GMLObject`. Same for `___struct___127/128`. Fix: when a struct ctor name
  collides with an object class of the same name, skip emitting the struct ctor as a separate export.

- [ ] **`instance_type_flow.rs` line 165: `Ne` TypeCheck case not handled.**
  The `Ne` comparison falls through to the same `TypeCheck` rewrite as `Eq`, which is wrong —
  `Ne` should produce a `Not(TypeCheck(...))`. Currently any `obj != SomeClass` check gets the
  same type narrowing as `obj == SomeClass`, yielding wrong narrowed types in the else branch.
  Untracked `// TODO: handle Ne case properly.` in source.

- [ ] **TS2345/TS2322/TS2365 (~80 errors): GmlInstanceTypeFlow arithmetic widening** —
  When a class constructor (`Struct(ClassName)` / `typeof ClassName`) participates in arithmetic
  (Add, Sub, Mul, Div, etc.), the result type should NOT inherit the constructor type. The game uses
  GML object indices as plain integers in arithmetic (e.g. `return Game - self.cameraX`, or
  field `creatorObject` narrowed to `typeof Player` then used in comparisons). Fix options:
  (a) In GmlInstanceTypeFlow, don't narrow fields/variables to class constructor types when they
  appear in arithmetic operand positions — use `Dynamic` instead. (b) In type inference, when
  an arithmetic op has a `Struct` or class-ref operand, widen the result to `Dynamic`/`Float`.
  Also affects return type inference: `getUIMouseX` returns `typeof Game` instead of `number`.

### 8. RunLoader::step stack underflow on Popz at 0x19c — Dead Estate translation error

- [ ] **`compute_block_stack_depths` uses `or_insert` — first path to reach a join point wins** —
  `RunLoader::step` fails with `0x19c: stack underflow on Popz`. The depth pre-computation walk
  sets `terminated = true` on `Bt`/`Bf`, which is correct since fall-through is always a block
  start. But `or_insert` means if two converging paths have different stack depths, only the first
  recorded depth is used. If the true runtime path arrives with fewer items than the recorded depth,
  subsequent pops underflow. Investigation needs bytecode dump of RunLoader::step to see which
  join point has inconsistent depths. Consider: (a) asserting stack depth consistency at join
  points, (b) using a worklist-based depth propagation instead of linear walk.

### 10. GMS1 integer object indices not resolved to class references — FIXED 2026-03-09

**Fixed.** Bounty TS2345 errors dropped from 48 → 13 (all TS2345 eliminated; 13 remaining are
pre-existing game-author type errors: TS2322/TS2362/TS2365/TS2366). Dead Estate also improved
from 248 → 186 (GMS2.3+ games sometimes use integer literals for object indices too).

**Secondary fix:** `parse_type_notation("classref")` in `ir/ty.rs` now maps to `Type::Dynamic`
(not the default `Type::Struct("classref")`). The `"classref"` string in runtime.json param types
was leaking into IR via `ConstraintSolve`'s `external_function_sigs` table → `parse_type_notation`
→ `Type::Struct("classref")` → callee params typed as `classref` → `ts_type` rendering as the TS
type annotation `classref` (e.g. `argument0: classref = 0.0`). Fix: treat `"classref"` as a
backend-only annotation, mapping to `Dynamic` at IR level.

**Root cause:** In GMS1, object type names are compile-time integer constants. `Push Int32(4)` +
`Conv v->v` creates `Cast(Const(Int(4)), Dynamic)` in the IR — no ClassRef type. Backend rewrite
had no mechanism to resolve these to class names.

**Fix (initial, 2026-03-09):** Backend rewrite pass `resolve_classref_args` in `rewrites/gamemaker.rs`
ran after `coerce_bool_args` for both free functions and class methods, replacing `JsExpr::Literal(Int(N))`
or `JsExpr::Cast { Literal(Int(N)), Dynamic }` at classref positions with `JsExpr::Var(object_names[N])`.
Import scanner in `collect_type_refs_from_function` extended with `cast_const_ints` map and `Op::Call`
handler to register newly-introduced class names.

**Architectural fix (2026-03-09):** Replaced backend rewrite with `GmlClassRefResolve` IR transform
pass in `crates/frontends/reincarnate-frontend-gamemaker/src/classref_resolve.rs`. The IR pass inserts
`Op::GlobalRef(name)` instructions typed `Type::ClassRef(name)` at classref-typed argument positions,
so every backend sees already-resolved class references in the IR. Backend `resolve_classref_args`,
`cast_const_ints` map, and associated import-scanner code removed from TypeScript backend.

**runtime.json changes:** Updated 31 functions with `"classref"` param type:
- instance_create/depth/layer (param[2]/[3])
- instance_number/find/change/nearest/furthest (param[0]/[0]/[0]/[2]/[2])
- instance_exists/activate_object/deactivate_object
- object_is_ancestor/get_name/exists/get_sprite/get_parent
- distance_to_object, place_meeting/position_meeting
- instance_place/position/place_list/position_list
- collision_point/rectangle/line/circle/ellipse + *_list variants
- mp_grid_add_instances

### 9. Pushref type_tag mapping is version-dependent — wrong for pre-2024.4 games

`build_asset_ref_names` currently uses the GM 2024.4+ type_tag layout. Pre-2024.4 games use a
different mapping where types 4–13 are shuffled (e.g. 4=Background, 6=Script, 8=Timeline,
10=Shader). Types 0–3 (Object, Sprite, Sound, Room) are the same in both versions, so games
using only those assets are unaffected. Need:
- [ ] Version detection (check `IsVersionAtLeast(2024, 4)` equivalent — likely in GEN8 chunk)
- [ ] Dual mapping in `build_asset_ref_names` based on detected version
- [ ] Same dual mapping in `generate_asset_ids`

Reference: UndertaleModTool `AdaptAssetType` / `AdaptAssetTypeId` in `UndertaleCode.cs`.

### New game inventory

| Game | Source | Status |
|------|--------|--------|
| 10 Second Ninja X | `data.win` 134MB | ⚠️ emits (TS errors TBD) |
| 12 is Better Than 6 | `game.unx` 179MB | ⚠️ emits (TS errors TBD) |
| Cauldron | `data.win` 169MB | ❌ YYC |
| CookServeDelicious2 | `game.unx` 805MB | ❌ EOF parse error in CODE (same as Forager) |
| Dead Estate | `data.win` 192MB | ⚠️ 112 TS errors + 1 translation error (2026-03-10) |
| Downwell | `data.win` 27MB | ❌ TXTR external textures |
| Forager | `game.unx` 78MB | ❌ EOF parse error in CODE |
| Just Hit The Button | `data.win` 1MB | ✅ emits (TS errors TBD) |
| Max Manos | `data.win` 47MB | ⚠️ 2 TS errors (local var pop raw index) |
| Max Manos 2 | `data.win` 10MB | ⚠️ 4 TS errors (local var pop raw index) |
| MINDWAVE Demo | `data.win` 324MB | ⚠️ ~26k TS errors (runtime API gaps) |
| Momodora RUtM | `.exe` 36MB | ❌ PE-embedded FORM |
| Nova Drift | `data.win` 415MB | ❌ YYC |
| Nubby's Number Factory | `data.win` 66MB | ⚠️ ~77k TS errors (runtime API gaps) |
| Risk of Rain | `game.unx` 34MB | ❌ YYC (empty CODE chunk) |
| Rocket Rats | `data.win` 2MB | ❌ YYC |
| Schism | `data.win` 77MB | ⚠️ emits (TS errors TBD) |
| Shelldiver | `data.win` 2MB | ❌ YYC |
| Soulknight Survivor | `data.win` 35MB | ❌ YYC |
| VA-11 HALL-A | `game.unx` 212MB | ⚠️ emits (TS errors TBD) |

---

## GameMaker — Runtime Platform Layer (HIGH PRIORITY)

The GameMaker runtime has several major API families that need platform-layer implementations:

### Audio (`platform/audio.ts`)
All `audio_*` functions are currently unimplemented and throw. Audio belongs in the platform
layer per the three-layer architecture. Needs:
- `platform/audio.ts` — Web Audio API implementation (AudioContext, AudioBufferSourceNode)
- Wire audio asset loading from `GameConfig` (asset table → audio buffer)
- `GameRuntime.audio_play_sound` → delegates to platform audio
- `GameRuntime.audio_is_playing` / `audio_stop_sound` / `audio_stop_all` / etc.

### Surfaces (`platform/surface.ts` or in draw layer)
`surface_*` / `draw_surface*` require off-screen rendering via WebGL or OffscreenCanvas.
Currently throw. Dead Estate uses surfaces extensively for post-processing effects.

### Shaders / GPU State
`shader_*` / `gpu_*` require WebGL shader compilation and state management. Currently throw.
Dead Estate uses shaders for visual effects (fog, color grading, etc.).

### Particle System
`part_*` require a real particle simulation system. Currently throw.
Dead Estate uses particles for effects (dust, sparks, etc.).

### GML Simulation Correctness Bugs (found 2026-03-09 audit)

**CRITICAL — `room_speed` completely ignored:**
The game loop calls `requestAnimationFrame` unconditionally every vsync frame. GML's `room_speed`
declares target steps/second (commonly 30, 60, 120). A game with `room_speed = 30` on a 60Hz
monitor runs 2× too fast. All per-step logic (alarms, physics, `xprevious`/`yprevious`) is
affected. Fix: accumulate elapsed time and step N times per frame to match `room_speed`.

- [ ] **Implement `room_speed`-based step accumulator in `_runFrame()`**

**CRITICAL — Built-in instance physics missing entirely:**
GML's built-in motion model (`speed`, `direction`, `hspeed`, `vspeed`, `friction`, `gravity`,
`gravity_direction`) is applied to `x`/`y` each step by the runner. None of these properties
exist on `GMLObject` and the step loop doesn't apply them. Games using built-in motion are
fully stationary.

- [ ] **Add `speed`, `direction`, `hspeed`, `vspeed`, `friction`, `gravity`, `gravity_direction`, `image_speed`, `image_angle` to `GMLObject`**
- [ ] **Apply built-in motion in the step loop** (before step event): apply gravity to speed along `gravity_direction`, apply friction to decelerate `speed`, decompose `speed`/`direction` into `hspeed`/`vspeed`, add to `x`/`y`

**CRITICAL — `_partUpdate`: system position offset applied as per-step velocity:**
`runtime.ts` line ~1174: `p.x += p.vx + s.pos[0]; p.y += p.vy + s.pos[1]` — `s.pos` is the
system origin offset, not a velocity. Adding it every step causes exponential drift. Fix:
`p.x += p.vx; p.y += p.vy;` during update; apply `s.pos` offset only at draw time.

- [ ] **Fix `_partUpdate` to apply `s.pos` at draw time only**

**WARN — `part_type_color_hsv` computes one color at type-definition time, not per-particle:**
GML's HSV range parameters are per-particle randomization at spawn time. Current impl calls
`randf` once and stores one color in `t.colors[0]` for all particles of the type.

- [ ] **Fix `part_type_color_hsv` to store HSV range params; randomize per spawn in `_spawnParticle`**

**WARN — `string_insert` off-by-one (GML is 1-based):**
`string.ts`: `s.slice(0, index) + sub + s.slice(index)` — at `index=1`, slices first character
instead of inserting before it. GML `string_insert(sub, s, 1)` inserts at the very start.
Fix: `s.slice(0, index - 1) + sub + s.slice(index - 1)`.

- [ ] **Fix `string_insert` 1-based indexing**

**WARN — `buffer_wrap` kind (2) silently drops writes past end:**
`buffer_write` grows for kinds 1/3 (grow/fast-grow), early-returns for kinds 0/4 (fixed/vbuffer),
but for kind 2 (wrap) it also silently drops without wrapping the position. GML `buffer_wrap`
semantics require position to wrap to `tell % size`.

- [ ] **Implement `buffer_wrap` wraparound in `buffer_write`/`buffer_read`**

**WARN — `clipboard_get_text()` returns stale internal cache:**
Only updates from `clipboard_set_text()` calls within the game. External clipboard content
(e.g. user copying a save code from elsewhere) is never read. `navigator.clipboard.readText()`
is async — needs a pre-fetch mechanism or async init path for games that use clipboard save codes.

- [ ] **Implement `clipboard_get_text()` with async clipboard read + cached result**

**WARN — `collision_circle` ignores `prec=true`:**
Uses AABB-vs-circle test regardless of the `prec` parameter. When `prec=true`, GML uses
pixel-precise sprite masks. The `_prec` parameter is currently accepted and ignored.

- [ ] **Track as known limitation; add `prec=true` TODO comment in source**

**WARN — `draw_rectangle_color` / `draw_triangle_color` use only first corner color:**
Three of four rect corner colors and two of three triangle vertex colors are ignored. Canvas 2D
doesn't natively support per-vertex colors, but a linear gradient approximation is feasible.

- [ ] **Implement gradient approximation for `draw_rectangle_color` and `draw_triangle_color`**

**WARN — `GetField` on `Union` type always returns `Dynamic`:**
`type_infer.rs` `infer_result_type` for `Op::GetField` handles `Struct(name)` and `ClassRef(name)`
but not `Union([...])`. Field access on any union-typed value yields `Dynamic`, losing type info
after any branch that produces different object types.

- [ ] **Add Union arm to `GetField` type inference: resolve field type for each union member, join results**

### Import side: free-function and asset references
`loadAnyaDataExt`, `AnyaSticker2A`, etc. appear as bare TS2304 names because the import
generator only adds imports for `Op::Call` callee positions — not for function references
used as values (via `@@pushref@@` / `GlobalRef`). Fix: scan all `GlobalRef` / `JsExpr::Var`
nodes in the emitted function body and add any that resolve to `_init.ts` exports or
asset-table names to the file's import set.

## GameMaker Frontend

### GML `with` Statement Bugs — FIXED (2026-02-22)

Discovered via Bounty reference comparison (2026-02-22). The GML `with(obj) { ... }` statement
had two distinct bugs in the frontend translator, both now fixed:

- [x] **`with` callback uses outer `self` instead of iterated instance** — Fixed by adding
  `_withSelf: any` parameter to the arrow function and replacing all `JsExpr::This` in the
  body with `JsExpr::Var("_withSelf")` via `replace_this_in_stmts` in `rewrites/gamemaker.rs`.
  Commit: 3ba5814. Note: field accesses on the iterated instance that went through `v0`
  (IR-level) are correctly replaced because `v0` (named "self") emits as TypeScript `this`,
  which is then caught by the replacement.

- [x] **Post-`with` code not captured** — Fixed by changing `PopEnv`'s loop-back case from
  `resolve_branch_target` (emits back-edge to body, leaving fall-through unreachable) to
  `resolve_fallthrough` (falls through to continuation). The GML iteration is handled entirely
  by `withInstances()` in the runtime — the IR doesn't need to model the loop. Both the
  loop-back case (sentinel >= 0) and the break-out case (sentinel < 0) now fall through.
  Commit: 7832b3a.

**Design debt — RESOLVED**: The `withBegin`/`withEnd` bracket anti-pattern turned out to already be fixed — the GML frontend already emitted `Op::MakeClosure` + `withInstances` directly (not `withBegin`/`withEnd`). `collapse_with_blocks` was dead code only exercised by its own tests. Deleted `collapse_with_blocks`, `try_extract_with_loop_body`, `make_with_instances`, `replace_this_in_stmts`, `replace_this_in_expr`, the `withBegin` canonicalization map entry, and all associated tests.

### GML Short-Circuit AND Condition Bug — FIXED (2026-02-22)

- [x] **`if (a && b && c)` emits ternary `a ? b : c` instead of conjunction** — Root cause was
  `GmlLogicalOpNormalize` incorrectly normalizing the shared else-block when multiple BrIf
  instructions share the same else target. The pass replaced `br merge(const_0)` with
  `br merge(cond_outer)`, making the condition `(cond_inner) ? (cond_innermost) : cond_outer`
  instead of the correct falsy short-circuit. Fix: skip normalization when the trivial block has
  more than one BrIf predecessor (commit 41671f2). Output is now semantically correct:
  `(pressed === 2) ? (locked === 0) : 0` (equivalent to `pressed===2 && locked===0`).
  The switch detection also fires correctly now, producing `switch(this.type)` blocks.

### GML 2D Array Compound-Assignment Bug — FIXED (commit b3db317)

Discovered via Bounty reference comparison (2026-02-22). Fixed 2026-02-22.

- [x] **2D array `+=` wrote `inventory[sum] = const` instead of `inventory[idx] = sum`** —
  Root cause: compound assignment uses the Dup pattern: `push dim2, push dim1, Dup, VARI-read,
  arithmetic, VARI-write`. After the read+arithmetic, the stack is `[dim2, dim1, new_value]`
  with new_value on top — opposite of simple assignment `[value, dim2, dim1]` with dim1 on top.
  Fix: added `compound_2d_pending` flag, set by `translate_push_variable` when originals remain
  after 2D VARI read (`stack.len() >= 2`). `translate_pop` uses reversed pop order when set.
  Output: `self.inventory[(int(i)*32000)+1] += argument1` (correct compound assignment).

### Other GML Logic Bugs Found in Bounty Comparison

- ~~**Missing `do_gangbang_encounter` function**~~ — Confirmed custom code added after the port
  (`~/git/bounty/scripts/main6.js`). Not a missing translation. N/A.

- [ ] **`save_game` missing INI writes** — The following fields are not written to the save file:
  `name`, all `a_*` appearance fields (`a_eye_color`, `a_skin_color`, `a_height`, `a_weight`,
  `a_hair_color`, `a_hair_length`, `a_hair_straightness`, `a_hair_style`, `a_other`, `a_racial`),
  `o_prostitution`, `o_self_defense`. Reference is `~/git/bounty/scripts/main.js:save_game`.

- [ ] **`roll_d6` hardcoded Y-coordinate** — Reference: `y = 480 - 20 * size` (scales with
  dice size). Emitted: hardcoded `460`. Dice render at wrong Y position when dice size ≠ 1.

- [ ] **Stats::create broken advantages/inventory loop** — The loop initializer in create()
  emits `i = i < 20` (assigns boolean comparison result to counter) instead of the loop
  running properly. All 20 advantages slots and inventory entries may not initialize correctly.
  Also missing: `negotiate_*` fields (6 bool fields), `o_prostitution`, `o_self_defense`,
  and all 12 `a_*` appearance fields. Also missing `user0()`/`user1()` events and debug
  `keypress70`/`keypress72`/`keypress76`/`keypress77` handlers.

- [ ] **Location::create scroll condition inverted** — Reference: `if (i > 440)` sets
  `scroll = true` and skips destroying LocationScroll; `else` destroys LocationScroll.
  Emitted: condition is `<= 440` (inverted) and `scroll` is set to 1 unconditionally after
  the branch. Also text width for scale computation is 380 instead of 390.

- [ ] **Location::step/draw debug check missing** — Reference: `if (instance_exists(obj_location_scroll) && obj_stats.debug)` before drawing debug overlay. Emitted: missing the `&& obj_stats.debug` guard.

- [ ] **Cross-object writes inside `with`-bodies emitted to `_self` instead of outer self** —
  When a `with(OtherObj)` body writes to the enclosing instance (e.g. `obj_race_reader.advantage = self.number`),
  the emitted code incorrectly assigns to `_self.advantage` (the iterated instance) instead of
  `outerSelf.advantage` (the captured outer self). Root cause: the with-body closure translator
  doesn't distinguish `InstanceType::Other` (outer self) from `InstanceType::Own/_self` (iterated
  instance). Fix: inside `translate_with_body`, capture the outer `self` parameter as an extra
  capture, and emit writes with `instance == other / outer-object-id` as `captured_self.field`
  rather than `_self.field`.
  Affects: `EquipReader::step` (`_self.type = _self.type` → should be `this.type = _self.type`),
  `RaceReader::step` (`_self.advantage = _self.number`), `OptionsReader::step` (`_self.type = _self.number`),
  `MainLocMain::step` (`_self.no_use = 1`).

- [ ] **GeneralGotoMain::step missing debug mode cleanup** — Reference sets `obj_stats.debug = false`
  and deletes `obj_stats.alarm[0]` when navigating from the debug room. Both ops are absent.
  Additionally TS has an extra `save_config()` call not in the reference.

- [ ] **`a && b` compiled as `a ? b : 0` ternary instead of logical AND** — When GML bytecode
  encodes `a && b` as a diamond CFG (eval `a`; if false skip; eval `b`; merge), the structurizer
  sometimes emits `a ? b : 0` instead of `a && b`. The two are semantically equivalent in a
  boolean `if`-condition context (both falsy when `a` is false), but the ternary form is harder
  to read and should ideally be normalized to `&&`. The `GmlLogicalOpNormalize` pass handles
  the simple 2-operand case but misses the 3+ operand case (two consecutive BrIf instructions
  where the second guard shares the else-target of the first). A post-structurize AST pass
  should detect `(cond ? inner_cond : 0)` / `(cond ? inner_cond : false)` and rewrite to
  `cond && inner_cond`. Affected: `StoreButton`, `TravelButton`, `TravelMain` in Bounty.
  See also: `GmlLogicalOpNormalize` in `rewrites/gamemaker.rs` commit 41671f2.

### Boolean / Short-Circuit Detection (open)

- [ ] **Numeric booleans: `=== 1` / `=== 0` instead of boolean tests** —
  GML compiles `if (self.active)` as `push self.active; pushi 1; cmp.eq; bf`.
  Requires heuristics to identify fields only assigned 0/1/true/false across
  all functions, then replace `=== 1` with bare test and `=== 0` with `!`.

- [ ] **Enum detection (string and numeric)** — Many GML games use string
  constants as enum values. Could extract into `const` objects during type
  inference. The reference code uses `Advantages.none`, `MouseButtons.pressed`,
  etc., showing these were originally named constants.

### Missing Runtime Functions

All previously listed functions have been implemented. Check `function_modules`
in runtime.json for any newly referenced but unimplemented functions.

## IR Architecture

### IR Invariant Violations (found 2026-03-09 audit)

- [ ] **`FunctionBuilder::br()` / `br_if()` don't validate arg count vs target block param count.**
  A mismatch is silently accepted at construction time and causes index-out-of-bounds panics or
  silent wrong-value assignments when `branch_assigns()` zips params with args during structurization.
  Fix: add a debug-mode assertion (or always-on check) in `br()` / `br_if()` that
  `args.len() == func.blocks[target].params.len()`. Add an IR verifier pass for catching this class
  of error during testing.

- [ ] **`ValueId`s are unscoped — a value from function A can be silently embedded in function B.**
  `ValueId` is a plain `u32` newtype with no function-scope binding. If a frontend accidentally
  passes a foreign `ValueId` into `fb.br(target, &[foreign_id])`, it silently accesses the wrong
  value (or an in-range-by-coincidence value). Consider a debug-mode `FuncId`-tagged `ValueId`
  wrapper, or at minimum document this invariant as caller responsibility and add a verifier check.

- [ ] **`null_sentinel_values` is `#[serde(skip)]` — lost across IR serialization.**
  If IR is ever serialized post-Mem2Reg (e.g. for incremental builds, caching, or `print-ir`
  roundtrip), sentinel information is lost and the emitter would emit spurious null-initialization
  assignments. Not a current bug (no serialization roundtrip of transformed IR exists), but
  the field should either be serialized or the sentinel logic should be reconstructable from
  the IR itself.

- [ ] **Closure support with variable capture** — The IR has
  `MethodKind::Closure` for marking closures, and the backend can inline them
  as `JsExpr::ArrowFunction`, but there's no mechanism for closures to capture
  variables from the enclosing scope. Currently Twine arrows work because all
  state is runtime-managed (`State.get`/`State.set`), so there's nothing to
  capture. Proper closure support would need:
  - `Op::MakeClosure { func_name, captures: Vec<ValueId> }` — creates a
    closure binding captured values from the current scope
  - `Function.captures: Vec<CaptureInfo>` — records which parent values
    each closure parameter maps to
  - Backend: when inlining, strip captured params and use parent variable
    names directly (JS native closures handle the rest)
  - Touches every transform pass (new Op variant requires match arms)

## Twine Frontend

- [x] **Use a proper HTML parser for extraction** — Replaced `extract_tagged_blocks` and `extract_format_css` with `extract_script_style_blocks`, a tokenizer-based implementation using `html5ever`. Returns `TokenSinkResult::RawData(ScriptData/Rawtext)` on script/style start tags, so the tokenizer handles raw content exactly as a browser would — no manual closing-tag search, no cross-element contamination.

- [ ] **Passage rendering strategy** — Implement `passage_rendering`
  manifest option (`auto`/`compiled`/`wikifier`). In `wikifier` mode,
  Rust emitter emits passage source as string constants instead of compiled
  functions. `auto` mode scans scripts for `Wikifier.Parser` references.

### SugarCube Runtime Errors (DOL)

Two runtime errors block DOL (Degrees of Lewdity) from running:

- [ ] **User script eval failure** — The emitted `__user_script_0` is ~45k
  lines compiled into a single `new Function(...)` call. If this eval fails
  (SyntaxError or runtime error), all subsequent `window.X = X` assignments
  inside it are lost. `resolve("allClothesSetup")` returns `undefined` because
  `allClothesSetup` was defined inside the eval but the eval died before
  reaching the `window.allClothesSetup = allClothesSetup` assignment. The
  improved error reporting in `evalCode()` (split SyntaxError vs runtime
  errors, logged to console) should surface the root cause when run in the
  browser — **needs testing**.
  - Cascading failure: `get("NPCNameList")` returns `undefined` in
    `widget_npcPregnancyUpdater` because StoryInit didn't complete.
  - Potential fix directions: split user scripts into smaller eval chunks,
    or identify the specific code pattern that causes the eval to fail.

- [x] **Un-parenthesized single-param arrow parsing** — Fixed in `06a0dce`.
  The parser now handles `x => expr` in addition to `(x) => expr`. DOL:
  596 → 817 inline arrows (+221 previously broken).

### SugarCube Translator Bugs

- [ ] **CRITICAL: eliminate per-function runtime service aliases** — Every exported function
  currently emits 7 `const SugarCube_X = _rt.X;` aliasing lines at the top (DOM, Engine,
  Input, Navigation, Output, State, Widget). These aliases are unnecessary — all call sites
  should use `_rt.State`, `_rt.Output`, etc. directly. The aliases add noise to every function,
  make diffs harder to read, and cause TypeScript to allocate extra locals for every call frame.
  Fix: emit `_rt.State.get(...)`, `_rt.Output.break()`, etc. at every call site; drop the
  alias-emission block entirely. This is a backend emitter change (TypeScript backend, `emit.rs`
  or the SugarCube-specific emitter).

- [ ] **`_rt` parameter threading in trc (1935 errors)** — Passage functions are emitted as
  `(_rt: SugarCubeRuntime) => void` but some callback sites (e.g. link targets, widget calls)
  pass them without the `_rt` argument, producing TS2345 "not assignable to `() => void`".
  Root cause: the translator threads `_rt` through passage closures but the call-site types
  don't reflect it. Either passage callbacks must always accept `_rt` consistently, or the
  closure form must be `() => void` with `_rt` captured from outer scope.

- [x] **`radiobutton` emits 1 arg instead of 2 in trc** — Fixed in `3079adb`. Input macros
  (`textbox`, `textarea`, `numberbox`, `checkbox`, `radiobutton`) now use `split_case_values()`
  → `MacroArgs::CaseValues` so each arg is a discrete token. Also fixed `checkbox` arg order
  (was `checkedValue, uncheckedValue`; SugarCube is `uncheckedValue, checkedValue`) and added
  `...flags` rest param to handle `checked`/`autocheck` bareword flags.

### SugarCube Type Emission Bugs (DoL)

- **`never[]` errors in DoL (game author errors)** — `[][_namecontroller] = x` (writing
  to a throw-away empty array literal), `[].pushUnique(...)`, `[].pluck(...)`, and
  `traits: never[]` from TypeScript's union inference on inline push args with empty
  `traits: []` in objects with inconsistent shapes. All are game author bugs in the original
  SugarCube source — reincarnate faithfully reproduces them. No fix warranted.

- [x] **`Property '0'/'1' does not exist on type '{}'` — fixed** — `Record<string, unknown>`
  object literal annotation made field reads return `unknown`; null-check narrowing then
  produced `{}`, and `{}[numeric]` is TS7053. Fixed by annotating object literals as
  `Record<string, any>` instead of `Record<string, unknown>`. Game objects are inherently
  dynamic — `any` is the accurate annotation when no source-level type info exists.

- [x] **jQuery `@types` missing in DoL runtime** — Already in `dev_dependencies` in `runtime.json`; was a stale entry.

### SugarCube Remaining Stubs

- [ ] **Scripting.parse()** — Returns code unchanged (identity function).
- [ ] **L10n.get()** — Returns key as-is. Low impact.
- [ ] **SimpleAudio.select()** — AudioRunner returned is a no-op stub.
- [ ] **Engine.forward()** — No-op (deprecated in SugarCube v2).
- [x] **SCEngine.clone()** — Fixed in `8e17415`. `clone(x)` now rewrites to standalone pure function import; was emitted as `SCEngine.clone(x)` method call. Eliminated 643 TS2339 errors.
- [x] **SCEngine.iterate() / iterator_has_next() / iterator_next_value() / iterator_next_key()** — Fixed in `8e17415`. Same fix; standalone pure function imports. Eliminated ~969 TS2339 errors.
- **TS2447 `|` on booleans** (1610 errors in DoL) — DoL game authors use `|` where they meant `||`: `(v === "Kylar") | (v === "Bailey")`. This is a **game author error** in the original source. Reincarnate correctly emits the game's code. These errors are expected and no fix is appropriate — changing `|` to `||` would alter semantics, and any other suppression would hide a real bug in the game.

### SugarCube oxc Parse Errors

Status: DoL 290→4, TRC 974→0. Fixed: default-to-Raw, LinkTarget, preprocessor context
(identifiers, property names, object literals), HTML entity decoding (html-escape crate),
UTF-8 preservation in preprocessor, template literal `${...}` preprocessing, `<<run>>`
statement parsing, trailing comma stripping in case values.

- [ ] **`settings.x` / `setup.x` in `<<case>>` args** — SugarCube's `parseArgs()`
  evaluates `settings.x` and `setup.x` barewords via `evalTwineScript()`, not as string
  constants. Currently falls through to `Bareword` in `classify_case_token`.

- [ ] **`[[...]]` SquareBracket token in `<<case>>` args** — SugarCube converts `[[text|passage]]`
  in arg position to a link object. Currently not handled by `classify_case_token` at all.
  Rare in practice as a case value (object identity comparison with `===` would never match).

- [ ] **`parseArgs()` semantics for media/audio macros** — When `<<audio>>`, `<<playlist>>`,
  `<<cacheaudio>>`, `<<track>>` etc. are eventually lowered to the platform audio layer, their
  args must use `parseArgs()` token semantics (discrete values: track ID, command string, volume)
  not the full-expression path. Currently fall to `Raw` which is safe but means audio is not
  preserved in output.

- [x] **CRITICAL: Extract `Macro.add()` from JavaScript passages** — Implemented in
  `sugarcube/custom_macros.rs`. Scanner extracts block/self-closing kind (`tags:` property)
  and `skipArgs: true` semantics. Registry built from user scripts before passage parsing.
  Custom entries shadow built-ins (DoL redefines `button`, `link`). Dynamic registrations
  (variable names) skipped silently. No `<<switch>>` override exists in DoL.

- [x] **CRITICAL: `assets/styles/user_0.css` for DoL contains JavaScript** — Fixed in `57e02a9`.
  `extract_tagged_blocks` preferred `</script>` unconditionally over `</style>`, so the user
  stylesheet block captured 1.5 MB of JS. Fix: pick whichever closing tag appears first.

### Harlowe Correctness Bugs

- [ ] **`(sorted: via lambda)` — `$dm's (it)` inside via lambda** — e.g.
  `(sorted: via $players's (it), ...(dm-names: $players))` uses `(it)` as a
  property key inside a `via` lambda. Our translator lowers `(it)` as a variable
  reference. Need to verify `$dm's (it)` translates correctly in lambda context
  (should produce `get_property(dm, it)`).

- [x] **`(sorted:)` via-lambda `its X` and implicit any (rogue-time-agent, 1 error)** —
  Two bugs: (1) `its year` was parsed as `Ident("its")` → `const_string("its")` instead
  of `Possessive(It, "year")`. Fixed in `expr.rs`: `its` desugars to `it's`. (2) `sorted`
  was in `is_predicate_op` causing `infer_param_types: true` on the via-lambda, but
  `Collections.sorted` takes `...args: unknown[]` so TS can't infer the item type → TS7006.
  Fixed by removing `sorted` from `is_predicate_op`. rogue-time-agent: 1 → 0 errors.

- [ ] **Unreachable code after `(goto:)`/`(stop:)` (equivalent-exchange, 8 errors)** —
  Code emitted after `goto`/`stop` IR returns is flagged as TS7027 unreachable by
  TypeScript. The structurizer or emitter should suppress statements following a
  guaranteed-terminating call. Alternatively, the frontend should mark blocks after
  unconditional goto/stop as dead and DCE should prune them.

- [ ] **TS2304 `vNNNN` across Dispatch case boundaries (DoL, 1501 errors)** —
  Values (e.g., `iterate()` results) are declared in one `case {}` block of a
  Dispatch/switch and used in a sibling `case {}`. TypeScript sees them as
  undeclared because each case is a separate block scope. Repro: `v5242` declared in
  case 742 (`const v5242 = iterate(...)`) used in case 745 (`iterator_next_value(v5242)`).
  Fix: during Dispatch emission, detect values defined in one case but referenced in
  another, and hoist their declarations (without init, with `Assign` at definition site)
  to before the switch statement — same approach as `block_params_preamble()`.

- [ ] **Unresolved temp vars / used-before-assigned (equivalent-exchange, DoL)** —
  Two related TS errors from temp var scoping gaps:
  - **TS2304 "Cannot find name `_x`"** — `_enemycockz`/`_cockz` (equivalent-exchange),
    `_swarmamounts`/`_arrayClothes`/`_clothing` (DoL): temp vars referenced in passage
    bodies but not declared in scope at the reference site. Likely a closure capture
    ordering problem — the lambda captures the var before its alloc is visible.
    Also: `_slot`/`_tentacleColour`/`_ii`/`_outfit`/`_kylar`/`__part` etc. (21 errors).
  - **Root cause for `_args = State.get("_args")` without `let` (TS2304)** —
    When a single-store alloc has 2+ loads (e.g. `_args` loaded in both a display
    expression and an if condition), after single-store promotion the stored value (v1)
    gets use_count >= 2. `emit_or_inline(v1, count≥2)` takes the `Assign` path +
    adds to `referenced_block_params`. But v1 is NOT a block param, so
    `collect_block_param_decls` never emits `let _args;`. Fix: in `emit_or_inline`,
    when count >= 2 AND value has a name AND is not yet in referenced_block_params,
    emit `VarDecl { name, init: Some(expr) }` (not bare Assign). Track that the
    first emission was a VarDecl so subsequent uses just reference the name.
  - **TS2454 "Variable `_x` used before being assigned"** — `_hooks`/`_them` (DoL):
    var IS declared (hoisted by `hoist_allocs()`) but has no initializer, and a code
    path reaches the use before the `(set:)` assignment. Fix: initialize hoisted allocs
    to `undefined` (or the Harlowe undefined sentinel) so no path is "unassigned".

- **TS2872 in artifact (game author error)** — `(set: $cond to (cond: ...))` where
  `(cond:)` is used as a value macro. This is a game author misuse of `(cond:)` —
  Harlowe's `(cond:)` is a value macro that picks between two values, not a
  condition object. Reincarnate correctly emits the call; the TS error reflects the
  type mismatch in the source. No fix warranted.

- **TS2363/TS2367 game author comparison errors** — Boolean or type-mismatched operand
  in a comparison that TypeScript rejects. Observed in the-national-pokedexxx (TS2363)
  and equivalent-exchange line 44703 (TS2367: `boolean` vs `number`). Game author errors
  in the original source. No fix warranted.

- [x] **Temp variable block-scope leak** — Fixed in `62bcd79`. Harlowe temp vars (`_var`) have
  passage-level scope, but `Op::Alloc` was placed inside nested blocks producing block-scoped `let`.
  Fix: `Function::hoist_allocs()` moves all allocs to the entry block before structurize. Resolved
  `_twelve` (artifact), `_sex`/`_victory` (equivalent-exchange).

- [x] **Backtick verbatim spans not handled in parser** — Fixed in `cfb1796`.
  Parser now handles backtick-delimited verbatim spans; `]` inside backticks
  no longer closes hooks. arceus-garden: 203 → 3 unknown_macro calls.

- [x] **`is ... or ...` shorthand not expanded** — Fixed in `d4bcf29`.
  `maybe_distribute_comparison()` wraps bare values in matching comparison
  when `or`/`and` follows `is`/`is not`.

- [x] **Changer `+` composition emits JS `+`** — Fixed in `155af06`.
  All Harlowe `+` routed through `Harlowe.Engine.plus()` runtime call
  which dispatches by type (changers, arrays, datamaps, numbers).

- [x] **`it` in `(set:)` context** — Fixed in `5223050`. Confirmed via
  Harlowe source (`setIt()` calls `.get()` on target VarRef): `it` inside
  `(set:)` refers to the target variable's current value. `set_target` field
  on TranslateCtx substitutes `It` with a read of the target variable.
  arceus-garden: 244 `get_it()` calls → 0.

- [x] **Missing macros** — `(obviously)` was prose misparsed as macro (parser
  fix: require colon). `(forget-undos:)` implemented in engine.ts + state.ts.

### Harlowe Output Quality

- [x] **Text coalescing** — Fixed in `66ade45`. New `coalesce_text_calls`
  AST pass merges adjacent string-literal text() calls into a single call.
  arceus-garden: 2,974 → 1,874 text() calls (-37%), 16,329 → 15,429 lines.

### Harlowe Performance

- [x] **O(n²) AST pass regression (12x → 2.4x)** — The declarative content
  tree refactor created more AST statements, exposing O(n²) behavior in
  `fold_single_use_consts` and `narrow_var_scope`. Both used a
  one-at-a-time loop pattern (scan body per candidate). Fixed with batch
  passes that precompute variable reference counts in one O(n) pass.
  arceus-garden: 1.1s → 0.22s (release). Remaining 2.4x vs baseline is
  from higher statement count (content-as-values), not algorithmic.

### Harlowe DOM Fidelity — Missing `tw-*` Custom Elements

Format CSS is now extracted from `<style title="Twine CSS">` in the story HTML
and emitted as `assets/styles/format_harlowe.css`. Scaffold uses `<tw-story>`
root for Harlowe (SugarCube keeps `<div id="passages">`).

**Structural (done):**
- [x] `<tw-story>` — root container (scaffold)
- [x] `<tw-passage>` — wraps current passage (navigation.ts, `tags` attribute)
- [x] `<tw-sidebar>` — sidebar with undo/redo (navigation.ts)
- [x] `<tw-icon>` — clickable sidebar icons (navigation.ts)

**Content (done):**
- [x] `<tw-link>` — clickable links (context.ts)
- [x] `<tw-broken-link>` — links to nonexistent passages (context.ts)
- [x] `<tw-hook>` — changer-styled content wrapper (context.ts `styled()`)
- [x] `<tw-expression>` — macro output wrapper (context.ts `live()`)
- [x] `<tw-collapsed>` — collapsed whitespace sections (context.ts `collapse()`)
- [x] `<tw-align>` — aligned content (context.ts `align()`)
- [x] `<tw-consecutive-br>` — consecutive line break normalization (context.ts `br()`)
- [x] `<tw-include>` — embedded passage content via `(display:)` (context.ts)

**Content (done — macro support added, untested against real games):**
- [x] `<tw-verbatim>` — raw/verbatim text via `(verbatim:)` changer (context.ts)
- [x] `<tw-enchantment>` — enchanted element wrapper via `(enchant:)`/`(enchant-in:)` (engine.ts)
- [x] `<tw-columns>` / `<tw-column>` — column layout via `(columns:)`/`(column:)` (engine.ts, context.ts)
- [x] `<tw-meter>` — progress meter via `(meter:)` (engine.ts)
- [x] `<tw-colour>` — color swatch display in `printVal` (context.ts)

**Dialog system (done — untested against real games):**
- [x] `<tw-dialog>` — modal dialog container via `(dialog:)` (engine.ts)
- [x] `<tw-backdrop>` — dialog backdrop overlay (engine.ts)
- [x] `<tw-dialog-links>` — dialog close link container (engine.ts)

**Content (done — transition wrapping):**
- [x] `<tw-transition-container>` — wraps hook children when `(transition:)` changer is applied (context.ts)

**Testing:** None of the 21 current test games use these macros. To verify
correctness, find a Harlowe game on IFDB/itch.io that exercises these macros
and add it to `~/reincarnate/twine/`.

**CSS animation keyframes:** Now extracted from format CSS (no runtime injection).

**Error/debug (skip):** `tw-error`, `tw-debugger`, `tw-eval-*`, etc.

**Data (extraction input only):** `tw-storydata`, `tw-passagedata`, `tw-tag`

### Harlowe Phase 2 (Advanced Features)

- [x] **`(for: each _item, ...$arr)[hook]`** — Loop lowering (done)
- [x] **`(live: Ns)[hook]` + `(stop:)`** — Timed interval (IR lowering, runtime `live()`/`stopLive`, navigation cleanup all done; regression test present)
- [x] **`(click: ?hook)[hook]`** — Event handler targeting named hooks (done — `click_macro` in engine.ts handles selector + hook callback)
- [x] **Collection constructors** — `(a:)`, `(dm:)`, `(ds:)` (done — runtime + frontend complete)
- [x] **Collection operators** — `contains`, `is in`, `'s`, `of` with full Harlowe semantics (done)
- [x] **Lambda expressions in collection ops** — `each _x where _x > 5` as predicate callback for
  `(find:)`, `(some-pass:)`, `(all-pass:)`, `(none-pass:)` etc. (done — `build_lambda_callback`)
- [x] **Lambda `via` expressions + `(sorted-by:)` + fold lambdas** — `via expr` tokenized and lowered
  as `ViaLambda`; `each _x making _acc via expr` parsed as `FoldLambda` for `(folded:)`;
  `(sorted-by:)` added to frontend + runtime; `interlaced`, `repeated`, `folded` runtime functions added.
- [x] **Dynamic macro calls** — `($varName: args)` expression-position calls; `(macro: type _p, [body])`
  closures; `ExprKind::DynCall`; `ExprKind::MacroDef`; `extract_macro_definition()` in lexer;
  `parse_macro_definition_args()` in parser; `build_macro_closure()` in translator.
- [x] **Changer composition with `+`** — `(color: red) + (text-style: "bold")` (fixed in `155af06`)
- [x] **`(save-game:)` / `(load-game:)`** — Save integration (basic runtime done; already implemented)
- [x] **`(replace:)`, `(show:)`, `(hide:)`** — DOM manipulation hooks (done)
- [x] **`(meter:)`, `(dialog:)`, `(dropdown:)`, `(checkbox:)`, `(input-box:)`** — UI macros (implemented)
- [x] **`(verbatim:)[...]`** — Raw text pass-through via `<tw-verbatim>` element
- [x] **`(enchant:)` / `(enchant-in:)`** — Apply changers to matching elements via `<tw-enchantment>`
- [x] **Named hooks** — `|name>[hook content]` and `?name` hook references (done)
- [x] **Complex `'s` possessive chains** — `$obj's (str-nth: $idx)` nested macro in possessive
  (already handled: `parse_prefix` falls to `try_parse_inline_macro` after `'s`)




