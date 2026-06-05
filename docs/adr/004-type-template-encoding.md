# ADR 004: `Type::Template` Encoding and Class-Scope Deferral

**Date:** 2026-06-05
**Status:** Accepted (variant deferred — no producer yet; encoding locked)

## Context

Parametric builtin signatures (`add_any: (T, T) -> T`, `array_get: (Array<T>, Int) -> T`)
require a way to express type parameters in the IR's `Type` enum.  The constraint collector
must instantiate fresh unification variables per call site for each parameter, allowing HM
to propagate element types through polymorphic operations.

Two candidate encodings were considered:

1. **Full de Bruijn indices** — `Template(depth, index)` where `depth` is the number of
   binders between the reference and the binding site.
2. **Bare single-binder index** — `Template(u32)` where the u32 is an index into the
   *innermost* function binder's type-parameter telescope; `depth` is always implicitly 0.

## Decision

Use **bare `Type::Template(u32)`** — an index into a single binder's type-parameter
telescope, with the binder always being the immediately enclosing function.

Full de Bruijn is rejected for two concrete reasons:

1. **No planned frontend produces nested-binder depth.** All 25 planned frontends (GMS1/GMS2,
   Flash/AS3, Twine, Ren'Py, RPG Maker, Inform, Ink, Wolf RPG, HyperCard, Director, VB6,
   Java Applets (partial), Silverlight (partial), QSP, RAGS, SRPG Studio, PuzzleScript,
   TyranoBuilder, KiriKiri/KiriKiriZ, NScripter/NScripter2, Suika2, Artemis, Narrat,
   Construct 2/3) either have no generics or have at most one level of polymorphism (function
   level).  The added complexity buys nothing.

2. **Escape checking becomes fragile.** The `ValidateNoEscapedTypeVars` pass (and its future
   `Template` arm) must confirm that no `Template` survives into persisted IR.  With de Bruijn,
   indices shift under context extension — a check that is correct in one context can silently
   become wrong after a binder is pushed or popped.  The bare encoding eliminates the problem:
   an index is always relative to the same single binder regardless of context depth.

### Binder Arity Must Be Declared

When `Type::Template` lands, `FunctionSig` gains a companion field:

```rust
pub type_param_count: u32,  // default 0; #[serde(skip_serializing_if = "is_zero")]
```

Without a declared arity the index `i` in `Template(i)` is meaningless and the escape
invariant is uncheckable.  The constraint collector allocates exactly `type_param_count`
fresh `TypeVarId`s per call site; `Template(i)` with `i >= type_param_count` is malformed IR;
any `Template` reachable from a `count == 0` signature is a bug.

### Interpretation Rule (to be written into the variant doc-comment when added)

> `Template(i)` addresses the innermost FUNCTION binder's type-parameter telescope;
> class-level type parameters, if introduced, get a distinct encoding.

This single sentence makes class-scope a migration-free additive extension: existing indices
never need reinterpreting when class generics are added.

### Escape Invariant: Never Persisted

`Type::Template` carries the same "never persisted" invariant as `Type::InferVar`: it must
not appear in `func.value_types`, `func.sig.params`, `func.sig.return_ty`, or any
`TypeDecl::Object` field after the constraint collector has finished instantiation.  The
`ValidateNoEscapedTypeVars` pass extends to `Template` by adding exactly one match arm
(the pass was designed arm-ready for this extension).

## Class-Scope Deferral

Class-level type parameters (e.g. `class Box<T>` where `T` appears in field types) are
genuinely needed by only 2 of 25 planned frontends: Java Applets and Silverlight, both of
which use reified-generics bytecode or IL.  Defer.

When class generics are built, add a sibling encoding (`ClassTemplate(u32)` or a scope tag
introduced at that point); existing `Template` indices are unaffected.  The real cost of
class generics is not the `ClassTemplate` encoding — it is reshaping `Type::Instance(TypeId)`
to carry instantiation arguments so that `Box<Int>` records `Int`.  That is a separate,
larger design, not part of the `Template` encoding decision.

## Consequences

- `Type::Template(u32)` is the locked encoding for function-level type parameters.
- The variant is not yet added — it has no producer until the parametric-builtins feature is
  implemented (see TODO.md: "Parametric FunctionSig `(T, T) → T` for `_any` builtins").
- `FunctionSig.type_param_count: u32` lands alongside the variant, not before.
- `ValidateNoEscapedTypeVars` is arm-ready: adding `Template` support is a one-arm extension.
- Class generics remain deferred and do not require migrating any `Template` indices.
