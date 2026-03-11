# Architecture Audit — 2026-03-12

Full end-to-end audit of the Reincarnate pipeline, covering pipeline stages, crate
boundaries, IR completeness, type system, pass ordering, abstractions, and Law compliance.

## Executive Summary

The architecture is **fundamentally sound**. Crate boundaries are clean, frontends and
backends are properly isolated, the IR serves as the sole channel between stages, and
the five Fundamental Laws are mostly upheld. There are no architectural blockers to
adding a second backend or new engines.

**Issues found (by severity):**

| # | Severity | Issue | Location |
|---|----------|-------|----------|
| 1 | HIGH | Module is a per-engine kitchen sink (6 GML + 4 Twine fields) | `module.rs` |
| 2 | HIGH | No aggregate constants — frontends bypass IR for data files | `module.rs`, `AssetCatalog` |
| 3 | HIGH | `abstract_members` is a 4-element tuple, not a struct | `module.rs:123` |
| 4 | MEDIUM | Harlowe.H rewrite in core (TODO says "removed" — still present) | `control_flow.rs:164` |
| 5 | MEDIUM | Flash Dictionary/Object handling hardcoded in core linearizer | `emit.rs:375-612` |
| 6 | MEDIUM | `Dynamic` conflates "unknown" and "truly dynamic" | IR type system |
| 7 | LOW | `ClassRef` emitted as `any` (workaround, not fix) | `types.rs:51` |
| 8 | LOW | StructDef fields are tuple `(String, Type, Option<Constant>)` | `module.rs:33` |

Issues 1, 2, 3 are already tracked in TODO.md. Issue 4 is a stale TODO entry (claims
fixed but isn't). Issues 5-8 are known trade-offs documented here for completeness.

---

## 1. Pipeline Stages

### Stage Inventory

```
Frontend.extract() → FrontendOutput { modules, assets, extra_passes }
                                    ↓
         inject external_type_defs + external_function_sigs from runtime.json
                                    ↓
         Transform Pipeline: 12 standard passes + extra_passes
                                    ↓
         link_modules() → SymbolTable (validation)
                                    ↓
         Backend.emit(BackendInput) → generated code + diagnostics
                                    ↓
         Checker.check() → type check results (optional)
```

### Pass Ordering

The 12 standard passes run in this order:

1. **TypeInference** — forward dataflow, recovers concrete types
2. **CallSiteTypeFlow** — interprocedural arg→param narrowing (`run_once`)
3. **ConstraintSolve** — bidirectional body-constraint narrowing (`run_once`)
4. **CallSiteTypeWiden** — undo over-narrowing from ConstraintSolve (`run_once`)
5. **CallSiteArityWiden** — extend sigs for over-applied calls
6. **ConstantFolding** — algebraic simplification
7. **CfgSimplify** — unreachable block removal
8. **CoroutineLowering** — async/yield to state machine
9. **Mem2Reg** — Alloc/Store/Load → SSA
10. **ConstantFolding (2nd)** — post-Mem2Reg opportunities
11. **RedundantCastElimination** — identity/nested cast cleanup
12. **DeadCodeElimination** — dead instructions and blocks
13. **[extra_passes]** — engine-specific (e.g., GmlLogicalOpNormalize, BoolArithCoerce)

**Implicit contracts:**
- Passes 2-4 must run in strict order (flow → solve → widen); they share `run_once`
  semantics to prevent circular reinforcement in fixpoint mode
- Mem2Reg (9) must run after CoroutineLowering (8) — coroutine lowering introduces
  Alloc/Store/Load chains for state variables
- Extra passes run after DCE — they see clean IR but types may be stale
  (GmlLogicalOpNormalize modifies block args without updating `value_types`)

**Assessment:** Pass ordering is correct but fragile. The implicit contract between
passes 2-4 should be documented in code (it's only in MEMORY.md). The extra_passes
stale-type issue (GmlLogicalOpNormalize) is a known bug tracked in TODO.md.

### Information Loss

| Transition | Lost | Impact |
|-----------|------|--------|
| Frontend → IR | Engine-specific AST structure | Intentional — IR is the abstraction |
| TypeInference | `Type::Var` placeholders resolved | Correct — vars are internal |
| Mem2Reg | Alloc/Store/Load chains | Intentional — promoted to SSA |
| IR → Backend AST | Block structure, value IDs | Structurizer reconstructs from dominators |
| Backend AST → TS | IR type annotations | Embedded as TS type annotations |

No unintentional information loss found. The intentional losses are by design.

---

## 2. Crate Boundaries

### Dependency Graph

```
datawin (standalone)      reincarnate-core (standalone)
     ↓                         ↓
     └──── frontend-gamemaker ──┘
                               ↓
           frontend-flash ─────┘
           frontend-twine ─────┘
           backend-typescript ──┘
           checker-typescript ──┘
                               ↓
                          reincarnate-cli (depends on all)
```

**Verified clean properties:**
- ✅ No circular dependencies
- ✅ Frontends never import backends or other frontends
- ✅ Backend never imports frontends
- ✅ Core imports nothing from workspace
- ✅ All frontends use only public APIs from core
- ✅ CLI uses trait objects exclusively (no direct engine-specific calls)
- ✅ All plugin crates are feature-gated in CLI

### Types Crossing Crate Boundaries

Frontend → Core: `Module`, `Function`, `ClassDef`, `StructDef`, `Type`, `Op`, `Constant`,
`FrontendOutput`, `AssetCatalog`, `Transform` (trait object)

Core → Backend: `BackendInput` (contains `Module`, `AssetCatalog`, `LoweringConfig`,
`RuntimePackage`, `DebugConfig`)

These are all public types in `reincarnate-core` — the API surface is correct.

---

## 3. IR Completeness

### What the IR Carries

| Data | Representation | Sufficient? |
|------|---------------|-------------|
| Functions | `PrimaryMap<FuncId, Function>` with SSA blocks | ✅ |
| Types | `Type` enum (18 variants) | ⚠️ No generics, limited unions |
| Classes | `ClassDef` + `StructDef` (fields) | ✅ |
| Constants | `Constant` enum (5 variants: Null/Bool/Int/Float/String) | ❌ No aggregates |
| Control flow | Block-based CFG with block args | ✅ |
| Closures | `MakeClosure` op with capture params | ✅ |
| Coroutines | `Yield`/`AwaitYield` ops, `CoroutineLowering` pass | ✅ |
| System calls | `SystemCall(system, method, args)` | ✅ |
| Entry point | `EntryPoint::ConstructClass` / `CallFunction` | ✅ |
| Diagnostics | `Vec<Diagnostic>` on Module | ✅ |

### What the IR Lacks

**Aggregate constants** (tracked in TODO.md) — `Constant` has no `Array` or `Map`
variant. Frontends bypass the IR entirely for structured data (sprite tables, object
registries, room definitions) by writing raw TypeScript into `AssetCatalog`. This is
the single largest Law 1 violation in the project.

**Generic/parameterized types** — `Array(Box<Type>)` and `Map(K, V)` are concrete, not
generic. Flash `Vector.<T>` loses type info. User-defined generics can't be represented.

**Flow-sensitive narrowing** — single `value_types` map per function, no per-block type
environments. After `if (x instanceof Foo)`, `x` retains its pre-guard type.

**Discriminated unions** — `Union(Vec<Type>)` is untagged. No variant tags, no pattern
matching support.

---

## 4. Type System Assessment

### Capabilities

The type system handles monomorphic, non-generic code well:
- Primitive types with bit widths (Int(8), Float(64), etc.)
- Named struct/enum types with field definitions
- Class hierarchy (single inheritance + interfaces)
- Function types with full signatures
- Nullable types (`Option(T)`)
- Union types (untagged `Union(Vec<Type>)`)
- Coroutine types (`Coroutine { yield_ty, return_ty }`)
- Type inference variables (`Var(TypeVarId)`)

### Workarounds in the Backend

| IR Type | TS Emission | Why |
|---------|------------|-----|
| `ClassRef(_)` | `any` | Class-as-value semantics don't map to `typeof X` cleanly |
| `Struct("Object")` | `Record<string, any>` | AS3 Object is a property bag |
| `Struct("Class")` | `any` | Metaclass; arbitrary static access |
| `Dynamic` | `any` | Conflates "unknown" with "truly dynamic" |

### `abstract_members` Representation

Currently a 4-element tuple:
```rust
pub abstract_members: Vec<(String, Type, Vec<Type>, MethodKind)>
//                        (name, return_ty, param_types, kind)
```

This is the same structural problem that `static_fields` had before the `StaticField`
refactor. It should be a named struct:
```rust
pub struct AbstractMember {
    pub name: String,
    pub return_ty: Type,
    pub params: Vec<Type>,
    pub kind: MethodKind,
}
```

### `StructDef.fields` Representation

Same issue:
```rust
pub fields: Vec<(String, Type, Option<Constant>)>
```

Should be:
```rust
pub struct FieldDef {
    pub name: String,
    pub ty: Type,
    pub default: Option<Constant>,
}
```

---

## 5. Fundamental Law Compliance

### Law 1: Pipeline Stage Isolation — ⚠️ ONE VIOLATION

**Violation:** Aggregate constants bypass (tracked in TODO.md). Frontends write raw
TypeScript data files into `AssetCatalog` because `Constant` can't represent arrays
or maps. The IR is bypassed for: `data/objects.ts`, `data/asset_ids.d.ts`,
`data/textures.ts`, `data/sprites.ts`, etc.

**Everything else is clean.** Module is the sole channel. No side channels found
outside the aggregate constant issue.

### Law 2: Engine Specificity at Boundaries — ⚠️ THREE REMAINING VIOLATIONS

Most violations were fixed in the 2026-03-11 audit. Three remain:

**1. Harlowe.H rewrite in core** (`control_flow.rs:164`)
- TODO.md line 128 says "No longer present" — **this is wrong**, the code is still there
- `SystemCall("Harlowe.H", method, args)` → `h.method(args)` rewrite runs in core
- Should be a Twine frontend extra_pass or gated behind a LoweringConfig flag

**2. Flash Dictionary handling** (`emit.rs:375-612`)
- `is_dictionary()` matches `"Dictionary"` string in core linearizer
- `Flash.Object` system calls for `deleteProperty`/`hasProperty` on dictionaries
- Partially addressed (TODO says `Flash.Object` dead code removed) but Dictionary
  detection and Map→`.get()` rewrite remain
- Map→`.get()` could be argued as engine-agnostic (all Map types in any engine
  would need `.get()`), but the `"Dictionary"` string check is Flash-specific

**3. Flash.Iterator foreach rewrite** (`control_flow.rs:582`)
- Already gated behind `LoweringConfig::foreach_rewrite` (Flash backend sets true)
- Code lives in core but is inert unless opted in
- Declared "Law 2 satisfied" in TODO.md — this is a reasonable judgment call

### Law 3: Behavioral Equivalence — ✅ PASS

No violations found. Source bugs are preserved correctly. Type errors in emitted code
reflect real source-language issues.

### Law 4: Honest Representation — ✅ PASS

IR types reflect source semantics. GML booleans are `Bool`, not `Float`. Instance IDs
have `ClassRef` types where resolved. The `Dynamic` conflation (unknown vs truly dynamic)
is a limitation, not a representation error.

### Law 5: Instantiability — ✅ PASS

All mutable runtime state on runtime instances. Known module-level violation
(`flash/memory.ts` heap) was fixed in 2026-03-11. No new violations found.

---

## 6. Module Struct — Kitchen Sink Problem

The `Module` struct has 22 fields. Engine-agnostic fields (10):

```
name, functions, structs, enums, globals, imports, classes, entry_point,
diagnostics, external_type_defs (+ external_function_sigs, external_imports,
system_call_type_rules, callback_return_calls)
```

Engine-specific fields (8):

| Field | Engine | Purpose |
|-------|--------|---------|
| `room_creation_code` | GML | Room init functions |
| `initial_room_name` | GML | First room to load |
| `sprite_names` | GML | Sprite ID → name map |
| `object_names` | GML | Object ID → name map |
| `passage_names` | Twine | Passage display→function map |
| `passage_tags` | Twine | Passage tag registry |
| `passage_sources` | Twine | Raw passage text |
| `passage_storylets` | Twine | Storylet condition functions |

Each new engine will add more. The correct fix is the aggregate constants design
(TODO.md lines 142-176): encode these as typed IR constants that backends emit
generically, eliminating the need for engine-specific Module fields.

---

## 7. Recommendations

### Immediate (this session or next)

1. **Fix stale TODO entry** — `control_flow.rs` Harlowe.H rewrite is NOT removed.
   Update TODO.md line 128 to reflect reality. Either gate behind LoweringConfig
   or move to Twine extra_pass.

2. **`abstract_members` → `AbstractMember` struct** — Same refactor as StaticField.
   Only Flash uses this; ~5 construction sites.

3. **`StructDef.fields` → `FieldDef` struct** — ~30 construction sites across
   frontends, transforms, and tests.

### Medium-term

4. **Aggregate constants** — Add `Constant::Array(Vec<Constant>)` and
   `Constant::Map(Vec<(String, Constant)>)`. Migrate GML data files and Twine
   passage registries from AssetCatalog blobs to IR constants.

5. **Document pass ordering contracts in code** — Add doc comments to
   `default_pipeline()` explaining why passes 2-4 must run in order and why
   extra_passes run last.

6. **Second backend as forcing function** — Adding a GDScript, Lua, or Rust backend
   will immediately surface any remaining TypeScript assumptions in the pipeline.
   The audit confirms the architecture can support this without major restructuring.

### Long-term

7. **Flow-sensitive type narrowing** — Per-block type environments for `instanceof`
   guards. Required for clean GML instance-typed code.

8. **Generic types** — Needed for Flash `Vector.<T>` and any future engine with generics.

9. **Distinguish Dynamic vs Unknown** — `Dynamic` means "truly dynamic" (e.g., GML
   `var`), `Unknown` means "we couldn't infer it". Backend could emit different
   annotations or diagnostics.

---

## 8. Second Backend Readiness

The pipeline is ready for a second backend. Specifically:

- **No TypeScript assumptions in core** — all TS-specific logic is in the backend crate
- **LoweringConfig is backend-controlled** — each backend sets its own flags
- **Runtime packages are per-engine-per-language** — `runtime/<engine>/<lang>/`
- **Backend trait is minimal** — `emit(BackendInput) → BackendOutput`
- **Scaffold generation is backend-specific** — `index.html` is TypeScript backend only

The main friction point is the aggregate constants bypass: a second backend would need
to duplicate the AssetCatalog TypeScript blob generation, which is exactly the kind of
engine-in-backend leak that aggregate constants would fix.

---

## Conclusion

The architecture is well-designed and has been systematically cleaned up over the past
week. The remaining issues are known, tracked, and have clear fix paths. The single
most impactful improvement is aggregate constants — it fixes the largest Law 1 violation,
eliminates engine-specific Module fields, and unblocks clean second-backend support.
