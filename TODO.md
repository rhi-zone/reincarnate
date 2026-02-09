# TODO

## Next Up

- [x] **IR builder API** — Convenience layer in `reincarnate-core` for constructing functions, blocks, and instructions without manually managing entity IDs. Every frontend needs this.
- [x] **IR printer** — Human-readable text format for dumping IR (like LLVM `.ll` or Cranelift CLIF). Essential for debugging frontends and transforms.
- [x] **CLI scaffolding** — `reincarnate-cli` crate with clap. Parse a project manifest, load source files, print info. Wire up the pipeline trait plumbing.
- [x] **Flash frontend** — `reincarnate-frontend-flash` crate. AVM2 bytecode extraction and decompilation using Ruffle's `swf` crate (MIT/Apache-2.0). First real target.

## Future

- [x] Type inference pass — forward dataflow (refine `Dynamic` via propagation)
- [x] Receiver-aware method resolution (class hierarchy walk, unique bare name fallback)
- [x] Redundant cast elimination pass (`Cast(v, ty)` → `Copy(v)` when types match)
- [x] Coroutine lowering transform (IR coroutine ops → state machines)
- [ ] Rust codegen backend (emit `.rs` files from typed IR — **blocked on alloc type refinement + multi-typed locals**)
- [x] TypeScript codegen backend
- [x] Dead code elimination pass
- [x] Constant folding pass
- [x] CFG simplification pass (merge redundant blocks, thread jumps)
- [x] Mem2Reg pass (promote single-store alloc/store/load chains, eliminate copies)
- [x] Structured control flow reconstruction (if/else, while, for from block CFG)
- [x] Transform pipeline fixpoint iteration (re-run until no changes)
- [x] Cross-module linking pass (resolve string imports, build global symbol table)
- [x] Asset extraction pipeline (images, audio, fonts from SWF/etc.)
- [ ] wgpu + winit renderer system implementation
- [ ] Web Audio system implementation

## Type System — Constraint-Based Inference

The current type inference is forward dataflow with fixed-point iteration. It
refines `Dynamic` when it can see the answer locally (constants, struct fields,
known function return types). This is enough for Flash (AVM2 has type
annotations) but insufficient for untyped frontends (Lingo, HyperCard, VB6
P-Code) where the IR starts as all-`Dynamic`.

A Rust backend makes this critical — Rust has no `any` escape hatch, so every
value needs a concrete type. This isn't a polish pass; it's a prerequisite.

### What exists today
- [x] Forward dataflow with fixed-point iteration
- [x] Receiver-aware method resolution (class hierarchy walk)
- [x] Cross-function return type propagation (module-level method index)
- [x] Select type inference
- [x] Redundant cast elimination

### What's needed
- [x] **Constraint-based solving** — `ConstraintSolve` pass generates equality
  constraints from operations and solves via union-find unification. Runs after
  forward `TypeInference` to propagate types backward (e.g., call argument used
  as `number` constrains the caller's variable). Reduced `:any` in Flash test
  output from 454 → 445.
- [ ] **Flow-sensitive narrowing** — narrow types after guards
  (`if (x instanceof Foo)` → `x: Foo` in then-branch). Requires per-block type
  environments rather than the current single `value_types` map. SSA form helps
  here — the BrIf arms can carry different type contexts.
- [ ] **Flash frontend: emit concrete types** — AVM2 bytecode has type
  annotations on locals, parameters, fields, return types. `resolve_type`
  failures cause unnecessary `Dynamic` entries. Fix the frontend to preserve
  what the source already knows, so inference only needs to handle what's
  genuinely untyped.
- [x] **Flash frontend: extract local variable names** — Done. Extracts from
  `Op::Debug` opcodes (not HAS_PARAM_NAMES, which has stale indices in this
  SWF). Register offset corrected for instance methods (`this` skipped).
  Names propagate through Mem2Reg and appear in TypeScript output.
- [ ] **Alloc type refinement** — The single biggest remaining `:any` source
  (~390 of 445). The Flash frontend creates `alloc dyn` for all locals. Even
  when every Store to an alloc writes the same concrete type (e.g. `Function`,
  `f64`), the alloc's own value_type stays `Dynamic`. The emitter declares
  locals using the alloc's type. Fix: if all stores to an `alloc dyn` agree on
  type, refine the alloc value_type to that type. Could live in the forward
  pass (`build_alloc_types` already computes the info but only uses it for Load
  refinement) or as a dedicated micro-pass.
- [ ] **Untyped frontend validation** — test the inference pipeline against a
  fully-untyped IR (simulating Lingo/HyperCard) to verify it can reconstruct
  useful types from usage patterns alone.

### Remaining `:any` analysis (Flash test, 445 total)

Measured on CoC.ts (36k lines) after TypeInference + ConstraintSolve.

| Category | Count | Root cause |
|----------|-------|------------|
| `const` locals | ~390 | Multi-store `alloc dyn` — alloc value_type stays Dynamic even when all stores agree. Emitter uses alloc type for declaration. |
| `let` locals | ~34 | Same as above but for Mem2Reg-promoted block params where incoming args don't all agree yet. |
| `any[]` arrays | 6 | Array element type unknown because elements are Dynamic (cascading from alloc issue). |
| Struct fields | 4 | Empty struct definitions (e.g. `struct Camp {}`) — fields accessed via GetField have no type info. Flash frontend doesn't populate all struct fields. |
| Multi-typed locals | ~5 | Genuinely different types assigned in different branches (e.g. `race = 0.0` then `race = "human"`). See Known Issues below. |

### Known Issues

- **Multi-typed locals** — Some Flash locals are assigned different types in
  different branches (e.g. `race` initialized to `0.0` as a sentinel, then
  assigned `this.player.race()` which returns `string`). These correctly stay
  `Dynamic` / `:any` today. For TypeScript this is ugly but functional. For
  Rust emit this is a hard blocker — Rust has no `any` type. Options:
  - **Split into separate variables** — SSA already distinguishes the defs, but
    the emitter coalesces them back into one mutable local. Could emit separate
    variables for each SSA def when types disagree.
  - **Enum wrapper** — generate a `Value2<A, B>` or tagged union for the
    specific types observed. Heavy, but correct.
  - **Sentinel elimination** — many cases are `0` or `null` sentinels followed
    by the real value. A pass that recognizes sentinel-then-overwrite patterns
    could use `Option<T>` instead of a union.
  - For TypeScript, the pragmatic fix is to emit a union type annotation
    (`number | string`) instead of `any`, which at least preserves type safety.

## Output Quality — FFDec Comparison

Compared our TypeScript output against JPEXS FFDec's ActionScript decompilation
of the same SWF. Parameter names now match. Detailed notes with specific method
examples in the test project (`~/cc-project/comparison-notes.md`).

### High Priority (correctness)

- [x] **`["rt:?"]` runtime property access** — Fixed. Runtime multinames now
  resolve to proper indexed access (`array[index]`).
- [x] **Instruction reordering** — Fixed. Side-effecting inline expressions
  (Call/SystemCall results) are now flushed at block boundaries to preserve
  evaluation order.
- [ ] **Negative constant resolution** — At least one `Math.max` clamp emits
  a wrong positive constant instead of the correct negative value.

### Medium Priority (output quality)

- [x] **Early returns via control flow inversion** — Done. Guard clause
  detection flattens `if/else` when one branch terminates.
- [x] **Default parameter values** — Done. HAS_OPTIONAL defaults emitted.
- [x] **Dead variable declarations** — Fixed. DCE Phase 5 eliminates unused
  block parameters at the IR level (iterative non-branch-arg analysis). Emitter
  buffers output and skips declaring params not referenced in the body. 31%
  reduction (12k → 8.4k) across the test project.
- [ ] **Complex loop decompilation** — Some while-loop bodies have unreachable
  code after `continue`, wrong variable assignments, and confused array
  accesses. Related to the `["rt:?"]` bug.

### Medium-High Priority (readability)

These are the main gaps between our output and ffdec-quality decompilation,
identified by comparing `takeDamage` / `reduceDamage` in Player.ts.

- [x] **Out-of-SSA variable coalescing** — Done. Mem2Reg propagates alloc debug
  names to stored values. Structurizer uses ValueId-only identity skip (not name
  matching). Lowerer detects shared names and emits assignments instead of const
  declarations, with self-assignment detection in branch-arg handlers.

- [x] **SE inline flush architecture** — Solved via AST-level single-use const
  folding (`fold_single_use_consts`). Instead of fixing the lowerer's flush
  mechanism, we let it produce conservative named variables, then fold
  single-use `const x = expr; use(x)` → `use(expr)` as a post-pass. This
  trivially recovers all inline opportunities lost by block-boundary flushing.

- [x] **Compound assignment detection** — AST-to-AST pass to rewrite
  `x = x + y` → `x += y`, `x = x - 1` → `x -= 1`, etc. Straightforward
  pattern match on `Stmt::Assign` where the value is a `Binary` with one
  operand equal to the target. Also `HP = HP - v` → `HP -= v`.

- [x] **Block-param decl/init merging** — When the ternary rewrite converts
  `if (c) { x = a } else { x = b }` → `x = c ? a : b`, the variable `x`
  is already declared as `let x: T;` at the top (block-param decl system).
  The result is a split `let x; ... x = c ? a : b` instead of a combined
  `let x = c ? a : b`. Post-pass merges uninit decls with their first
  dominating assignment. 43% of split let decls merged (1768/4081).

### Remaining `vN` Identifiers — Full Elimination Plan

After the fixpoint pass pipeline (narrow → merge → fold), 293 `vN` declaration
instances remain across 90 files (227 unique name strings). Goal: zero.

Current pass order: ternary → minmax → eliminate_self_assigns →
[fixpoint: narrow_var_scope → merge_decl_init → fold_single_use_consts] →
compound_assign.

#### Category A: Dead ternary dispatch tables (144 `let`, dead)

```typescript
let v4919: number = (0 !== select) ? ((1 !== select) ? ... : 1) : 0;
}  // end of function — v4919 never read
```

AVM2 `lookupswitch` lowered to a ternary that maps the discriminant to a case
index. The actual dispatch is the if/else chain *above* this expression. The
ternary result is never consumed.

**Fix**: `fold_single_use_consts` already eliminates dead `const` decls
(total_refs == 0 && pure init). Extend it to also eliminate dead `let` decls
with pure init (total_refs == 0 after excluding the declaration itself). This is
the `let` counterpart of the existing dead-const elimination. Alternatively, add
a dedicated dead-statement pass that removes any `VarDecl` with zero reads and
a pure init.

**Complexity**: Trivial — ~5 lines in `try_fold_one_const` or a new 10-line
function.

#### Category B: Dead impure consts (10 `const`, dead)

```typescript
const v736 = this.player.statusAffectv1(StatusAffects.Exgartuan) !== 2;
// v736 never read
```

Condition eagerly evaluated but never consumed. Init contains method calls that
may have side effects, so the declaration can't be blindly removed.

**Fix**: Convert `const vN = expr;` to bare `expr;` (expression statement) when
the result is unused. This preserves side effects while dropping the binding.
The expression statement itself may then be eliminable by a subsequent pass if
the call is provably pure, but that's a separate concern.

**Complexity**: ~15 lines — new pass or extension to fold. Match `VarDecl` with
init, zero reads, impure init → replace with `Stmt::Expr(init)`.

#### Category C: Single-use impure consts, non-adjacent (19 `const`, single-use)

```typescript
const v89 = this.player.findPerk(PerkLib.Resolute) < 0;    // DECL
damage = this.player.takeDamage(damage);                     // INTERVENING (impure)
if (v89) {                                                   // USE
```

All 19 have exactly one intervening side-effecting statement between decl and
use. Fold correctly refuses because the impure init can't be moved past the
intervening side effect.

**Fix**: These require proving that reordering is safe — the init expression
doesn't alias the intervening mutation. Two approaches:

1. **Sink the condition below the intervening stmt** — if the init reads fields
   that the intervening stmt doesn't write, sinking is safe. In practice, most
   are `findPerk()`, `hasCock()`, `rand()` whose results don't depend on
   `takeDamage()`'s side effects. But proving this requires alias/purity analysis
   on method calls.
2. **Source-level reordering at the frontend** — AVM2 evaluates these conditions
   before the intervening call. Our structurizer hoists them into consts. If we
   instead defer condition evaluation to the branch point (lazier structurizing),
   the const disappears naturally. This is the root cause fix.
3. **Accept the const** — `const v89 = expr; if (v89)` is readable and correct.
   The only cost is the synthetic name. A rename pass could assign meaningful
   names based on the init expression (e.g., `isResolute`, `hasCock`).

**Complexity**: Approach 1 is hard (alias analysis). Approach 2 is medium
(structurizer change). Approach 3 is cosmetic only (rename pass, ~50 lines).

#### Category D: Single-use pure consts, non-adjacent (11 `const`, single-use)

```typescript
const v16 = storage[slotNum].itype;
_loc5.quantity -= 1.0;                                       // INTERVENING (impure)
this.inventory.takeItem(v16 as ItemType, ...);               // USE
```

Same pattern as Category C but with pure inits. Fold refuses because the
intervening statement has side effects. In all 11 cases the init reads a field
that the intervening stmt doesn't modify, so reordering would be safe — but we
don't have alias analysis to prove it.

Two special cases with gap=163/164 (`Lottie.ts`): `const v24 = this.hugeFunc;
const v19 = this.otherFunc;` — these are method references cached at function
entry but used far later. Folding is safe but the distance means many
intervening side effects.

**Fix**: Same approaches as Category C. Pure inits are slightly easier to
reason about — if the init is a simple field read (`x.y`) and no intervening
stmt writes to `x.y`, sinking is safe. A conservative heuristic: allow sinking
pure non-call inits past assignment statements that don't write to the same
receiver.

**Complexity**: Medium — ~40 lines for a conservative heuristic, or full alias
analysis for complete coverage.

#### Category E: Multi-use consts (49 total: 31 ternary, 4 impure, 14 pure)

```typescript
// 31 ternary consts: phi-input strings used in 2+ branch assignments
const v90 = ("turns her " + ((flags !== 0) ? "lips" : "muzzle")) + " and ";
v93 = v90;   // phi-assign branch 1
v93 = v90;   // phi-assign branch 2 (duplicate structurizer edge)
```

The 31 ternary consts feed into `let` uninit phi variables (Category G) via
duplicate branch edges. The const is multi-use because the structurizer emits
the same assignment in both branches of a diamond.

The 4 impure and 14 pure consts are genuinely multi-use — cached values read
2+ times (NPC check flags, method references, property lookups).

**Fix for the 31 ternary consts**: Deduplicate structurizer branch edges. When
both arms of an if/else assign the same value to the same phi variable, emit
the assignment once after the if/else instead of in both branches. This makes
the const single-use, enabling fold. Or: detect `v = x; v = x;` (identical
assigns in both branches) and collapse to a single assign after the merge.

**Fix for the 18 genuinely multi-use**: These are correct — a named variable is
the right representation when a value is used multiple times. Elimination
requires either (a) duplicating the init expression at each use site (only safe
for pure inits without large expressions) or (b) a rename pass for cosmetics.

**Complexity**: Duplicate-edge fix is medium (~30 lines in structurizer or as
an AST pass). Genuinely multi-use consts are irreducible without expression
duplication.

#### Category F: Single-use uninit lets / forwarding stubs (3 `let`, single-use)

```typescript
let v392: boolean;
v408 = v392;   // forwarding: read uninitialized, assign to another phi
```

These are structurizer artifacts from empty else-branches. `v392` is declared,
never assigned, and its undefined value is forwarded to another phi variable.
This is a codegen bug — the else branch should either not exist or should assign
a meaningful value.

**Fix**: Detect `VarDecl` (uninit) immediately followed by `vN = vM;` where
`vM` is the just-declared uninit var. Eliminate both statements (the decl and
the forwarding assign). The receiving phi `vN` already has a value from the
other branch. Or fix the structurizer to not emit empty-branch phi assignments.

**Complexity**: Easy — ~10 lines as an AST pass, or fix in the structurizer's
branch-arg emission.

#### Category G: Multi-use uninit lets — 1 assign, 1 read (7 `let`)

```typescript
let v58: boolean;
if (cond) {
    v58 = expr;     // assigned in one branch only
} else {
    // different control flow (return, etc.)
}
if (v58) { ... }    // read after merge
```

Assigned in one if-branch, not assigned in the other (the else has early return
or different logic). Read after the merge point. The variable may be
`undefined` if the non-assigning branch is taken — matching original
ActionScript semantics.

**Fix**: `narrow_var_scope` can't push these because the use is outside the if.
`merge_decl_init` can't merge because the assign is inside an if-body. Two
approaches:

1. **Hoist the init with a default** — rewrite to
   `let v58: boolean = false; if (cond) { v58 = expr; }` then fold the init+use
   if single-use. But we don't know the correct default.
2. **Conditional expression** — if the else branch always returns/breaks (i.e.,
   the code after the if is only reachable from the then-branch), the if is
   really a guard clause. The assign dominates the use. Convert to
   `const v58 = expr;` after the if. Requires dominator analysis on the AST.
3. **Accept the let** — these 7 are semantically correct. A rename pass could
   give them meaningful names.

**Complexity**: Approach 2 is medium (AST-level dominator check, ~40 lines).
Approach 3 is cosmetic.

#### Category H: Multi-use uninit lets — 3+ assigns, 1 read (28 `let`)

```typescript
let vN: string;
if (condA) {
    vN = "text A";
} else if (condB) {
    vN = "text B";
} else {
    vN = "text C";
}
this.outputText(vN);
```

Classic 3-way phi merge. 20 are in KitsuneScene.ts (gender/anatomy text
selection), 5 in Katherine.ts, 3 in Camp/Amily/Plains. All string or boolean.

**Fix**: Rewrite the if/else-if chain to a nested ternary:
`vN = condA ? "text A" : condB ? "text B" : "text C"`. This collapses the
uninit decl + multi-branch assigns + single read into a single
`const vN = ternary`. Then fold eliminates it if single-use.

The ternary rewrite pass already handles 2-branch if/else → ternary. Extending
it to handle if/else-if chains (3+ branches, all assigning to the same
variable) would capture this pattern. The result is a nested ternary.

**Complexity**: Medium — ~50 lines extending `rewrite_ternary`. Must detect
chains where each branch assigns to the same variable and the last branch is
unconditional.

#### Category I: Multi-use uninit lets — 3+ assigns, 2+ reads (7 `let`)

```typescript
let v154: string;          // KitsuneScene.ts — 7 assigns, 3 reads
let v151: string;          // KitsuneScene.ts — 3 assigns, 7 reads
let v210: string;          // KitsuneScene.ts — 3 assigns, 4 reads
let v236: string;          // KitsuneScene.ts — 4 assigns, 1 reads (reuse)
let v597: boolean;         // Rubi.ts — 4 assigns, 1 reads
let v3883: boolean;        // Camp.ts — 3 assigns, 1 read
let v2271: boolean;        // Camp.ts — 3 assigns, 1 read
```

Genuinely multi-use phi variables. These are either read multiple times (text
fragments used in several `outputText` calls) or assigned from many branches.
They are correct mutable variables.

**Fix**: These are irreducible without restructuring the control flow. A rename
pass could give them meaningful names (e.g. derive from the outputText context
or the condition). Otherwise, accept them.

**Complexity**: Rename pass is ~50 lines. Structural elimination is not
practical.

#### Category J: Init `let` ternary, single-use non-adjacent (12 `let`)

These are switch dispatch tables that ARE consumed once, but with side-effecting
statements between the ternary and the use. Same as Category C but with `let`
and ternary inits.

**Fix**: Same as Category C — requires alias analysis or source-level
reordering. Less impactful than Categories A/B.

**Complexity**: Same as Category C.

#### Category K: Init `let` non-ternary (1 `let`)

```typescript
// Rubi.ts
let v647: boolean = (this.flags[...] < 30) && this.flags[...];
```

Single instance. Multi-use reassigned boolean. Correct mutable variable.

**Fix**: Accept or rename.

#### Summary: elimination roadmap by priority

| Priority | Category | Count | Fix | Effort |
|----------|----------|-------|-----|--------|
| **1** | A: Dead ternary lets | 144 | Extend dead-decl elimination to `let` | Trivial |
| **2** | B: Dead impure consts | 10 | `const vN = expr;` → `expr;` | Easy |
| **3** | F: Forwarding stubs | 3 | Remove uninit-then-forward pattern | Easy |
| **4** | H: 3-way phi → ternary | 28 | Extend ternary rewrite to if/else-if chains | Medium |
| **5** | E: Duplicate branch edges | 31 | Deduplicate identical phi-assigns | Medium |
| **6** | G: Single-assign guard | 7 | AST dominator check or accept | Medium |
| **7** | C+D: Non-adjacent single-use | 30 | Alias/purity analysis or accept | Hard |
| **8** | J: Non-adjacent ternary | 12 | Same as C+D | Hard |
| **9** | I+K: Genuinely multi-use | 8 | Rename pass or accept | Cosmetic |
| **10** | E: Genuinely multi-use const | 18 | Rename pass or accept | Cosmetic |
| | **TOTAL** | **291** | | |

Priorities 1–3 are easy wins (157 vars, ~30 lines of code). Priority 4–5
(59 vars) are medium-effort structural improvements. Priorities 6–10 (75 vars)
require either hard analysis or are cosmetic-only.

### Architecture — Hybrid Lowering via Structured IR

The current `lower_ast.rs` (~1000 lines) interleaves three concerns: control
flow lowering, expression inlining, and side-effect ordering. This causes
cascading bugs — inlining decisions interact with ternary detection, name
coalescing, self-assignment elimination, and shape processing in ways that are
hard to reason about.

Replace with a three-phase hybrid pipeline:

- [x] **Phase 1: Shape → `LinearStmt`** — Walk the Shape tree and produce a
  flat `Vec<LinearStmt>` where every instruction is a `Def(ValueId, Op)`,
  control flow comes from shapes (`If(ValueId, Vec, Vec)`, `While`, etc.),
  and branch args become `Assign(ValueId, ValueId)`. No inlining decisions.
  Trivial ~200-line shape walk.

- [x] **Phase 2: Pure resolution on `LinearStmt`** — Single pass over the
  structured IR. Pure single-use values (`use_count == 1 && is_pure`) are
  substituted into their consumer (ValueId → expression tree). Constants
  always substituted. Scope lookups + cascading GetField marked as
  always-rebuild. Dead pure code dropped. Self-assignments detected via
  ValueId equality. Name coalescing annotated (shared ValueIds → mutable
  assignment). This handles 90% of inlining with zero side-effect concerns.

- [x] **Phase 3: `LinearStmt` → AST** — Resolve remaining ValueIds to
  variable names. Side-effecting single-use values inlined if no
  intervening side effects (the only hard case, now isolated to ~10% of
  values). Multi-use values get `const`/`let` declarations. Produces
  `Vec<Stmt>` for existing AST passes.

Existing AST passes (ternary, compound assign, const fold, decl/init merge,
self-assign elimination) continue unchanged on the output AST.

Benefits: eliminates `lazy_inlines`, `side_effecting_inlines`,
`always_inlines`, `se_flush_declared`, `skip_loop_init_assigns`, and the
flush mechanisms. Pure inlining is provably correct (no ordering concerns).
Side-effect handling is isolated. `LinearStmt` is thinner than the AST
(ValueId refs vs String names) — net memory reduction vs current `LowerCtx`.

### Low Priority (polish)

- [ ] **Redundant type casts** — Eliminate `as number` etc. when the expression
  already has the target type.
- [ ] **Inline closures** — Filter/map callbacks extracted as named function
  references instead of being inlined as arrow functions.
- [ ] **Condition inversion** — Structurizer sometimes inverts conditions.
  Not a bug but reads backward vs the original source.
