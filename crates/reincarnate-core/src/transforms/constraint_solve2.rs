//! HM-style unifier infrastructure and ConstraintSolve2 pass.
//!
//! This module provides the unifier primitives needed for a full HM type-inference
//! redesign, plus the [`ConstraintSolve2`] pass that runs [`collect_function`] and
//! processes the resulting constraints via the HM unifier to update IR value types.
//!
//! Unifier primitives:
//! - [`TypeVarArena`] — allocates fresh [`TypeVarId`] values with levels and bindings
//! - [`resolve`] — dereference a [`Type`] through the arena (walk bound vars)
//! - [`occurs`] — occurs check (prevent infinite types)
//! - [`bind_var`] — bind a type variable with occurs check + level adjustment
//! - [`unify`] — HM unification of two [`Type`] values
//! - [`UnifyError`] — error type for unification failures

use std::collections::HashMap;

use crate::entity::EntityRef;
use crate::error::CoreError;
use crate::ir::ty::{FunctionSig, Type, TypeVarId};
use crate::ir::Module;
use crate::pipeline::{Transform, TransformResult};

// ---------------------------------------------------------------------------
// TypeVarArena
// ---------------------------------------------------------------------------

/// Allocator for fresh [`TypeVarId`] values.
///
/// Each variable tracks:
/// - `level` — the generalization scope level at which it was created
/// - `binding` — `None` if unbound, `Some(ty)` if resolved
pub struct TypeVarArena {
    levels: Vec<u32>,
    bindings: Vec<Option<Type>>,
    current_level: u32,
}

impl Default for TypeVarArena {
    fn default() -> Self {
        Self::new()
    }
}

impl TypeVarArena {
    /// Create a new arena at level 0.
    pub fn new() -> Self {
        Self {
            levels: Vec::new(),
            bindings: Vec::new(),
            current_level: 0,
        }
    }

    /// Allocate a fresh type variable at the current level.
    pub fn fresh(&mut self) -> TypeVarId {
        let id = TypeVarId::new(self.levels.len() as u32);
        self.levels.push(self.current_level);
        self.bindings.push(None);
        id
    }

    /// Enter a new generalization scope (increment level).
    pub fn enter_level(&mut self) {
        self.current_level += 1;
    }

    /// Exit the current generalization scope (decrement level).
    ///
    /// # Panics
    /// Panics if already at level 0.
    pub fn exit_level(&mut self) {
        assert!(self.current_level > 0, "exit_level called at level 0");
        self.current_level -= 1;
    }

    /// Return the level at which `id` was created.
    pub fn level_of(&self, id: TypeVarId) -> u32 {
        self.levels[id.index() as usize]
    }

    /// Return the binding of `id`, if any.
    pub fn binding_of(&self, id: TypeVarId) -> Option<&Type> {
        self.bindings[id.index() as usize].as_ref()
    }

    /// Bind `id` to `ty`.
    ///
    /// # Panics
    /// Panics if `id` is already bound.
    pub fn bind(&mut self, id: TypeVarId, ty: Type) {
        let idx = id.index() as usize;
        assert!(
            self.bindings[idx].is_none(),
            "bind: type variable {:?} is already bound",
            id
        );
        self.bindings[idx] = Some(ty);
    }

    /// Lower the level of `id` to `new_level` (only if currently higher).
    fn lower_level(&mut self, id: TypeVarId, new_level: u32) {
        let idx = id.index() as usize;
        if self.levels[idx] > new_level {
            self.levels[idx] = new_level;
        }
    }
}

// ---------------------------------------------------------------------------
// resolve
// ---------------------------------------------------------------------------

/// Walk `ty`, substituting any bound [`Type::Var`] with its binding (recursively).
///
/// Unbound variables remain as [`Type::Var`].
pub fn resolve(ty: Type, arena: &TypeVarArena) -> Type {
    match ty {
        Type::Var(id) => match arena.binding_of(id) {
            Some(bound) => resolve(bound.clone(), arena),
            None => Type::Var(id),
        },
        Type::Array(elem) => Type::Array(Box::new(resolve(*elem, arena))),
        Type::Map(k, v) => Type::Map(Box::new(resolve(*k, arena)), Box::new(resolve(*v, arena))),
        Type::Option(inner) => Type::Option(Box::new(resolve(*inner, arena))),
        Type::Tuple(elems) => Type::Tuple(elems.into_iter().map(|t| resolve(t, arena)).collect()),
        Type::Function(sig) => {
            let params = sig.params.into_iter().map(|p| resolve(p, arena)).collect();
            let return_ty = resolve(sig.return_ty, arena);
            Type::Function(Box::new(FunctionSig {
                params,
                return_ty,
                defaults: sig.defaults,
                has_rest_param: sig.has_rest_param,
            }))
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => Type::Coroutine {
            yield_ty: Box::new(resolve(*yield_ty, arena)),
            return_ty: Box::new(resolve(*return_ty, arena)),
        },
        Type::Union(variants) => {
            Type::Union(variants.into_iter().map(|t| resolve(t, arena)).collect())
        }
        // Leaf types — no nested type variables.
        t @ (Type::Void
        | Type::Bool
        | Type::Int(_)
        | Type::UInt(_)
        | Type::Float(_)
        | Type::String
        | Type::Struct(_)
        | Type::Enum(_)
        | Type::ClassRef(_)
        | Type::Dynamic
        | Type::Unknown) => t,
    }
}

// ---------------------------------------------------------------------------
// occurs
// ---------------------------------------------------------------------------

/// Return `true` if `id` appears free in `ty` (after resolving through `arena`).
///
/// Used to prevent the creation of infinite (recursive) types.
pub fn occurs(id: TypeVarId, ty: &Type, arena: &TypeVarArena) -> bool {
    // Resolve the type first so we follow any existing bindings.
    let resolved = resolve(ty.clone(), arena);
    occurs_resolved(id, &resolved, arena)
}

/// Occurs check on an already-resolved type (no further resolution needed at the
/// top level, but nested vars are re-resolved as we descend).
fn occurs_resolved(id: TypeVarId, ty: &Type, arena: &TypeVarArena) -> bool {
    match ty {
        Type::Var(other) => *other == id,
        Type::Array(elem) => occurs(id, elem, arena),
        Type::Map(k, v) => occurs(id, k, arena) || occurs(id, v, arena),
        Type::Option(inner) => occurs(id, inner, arena),
        Type::Tuple(elems) => elems.iter().any(|t| occurs(id, t, arena)),
        Type::Function(sig) => {
            sig.params.iter().any(|p| occurs(id, p, arena)) || occurs(id, &sig.return_ty, arena)
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => occurs(id, yield_ty, arena) || occurs(id, return_ty, arena),
        Type::Union(variants) => variants.iter().any(|t| occurs(id, t, arena)),
        Type::Void
        | Type::Bool
        | Type::Int(_)
        | Type::UInt(_)
        | Type::Float(_)
        | Type::String
        | Type::Struct(_)
        | Type::Enum(_)
        | Type::ClassRef(_)
        | Type::Dynamic
        | Type::Unknown => false,
    }
}

// ---------------------------------------------------------------------------
// Collect free type variables
// ---------------------------------------------------------------------------

/// Collect all free (unbound) type variables in `ty` into `out`.
fn collect_free_vars(ty: &Type, arena: &TypeVarArena, out: &mut Vec<TypeVarId>) {
    match ty {
        Type::Var(id) => match arena.binding_of(*id) {
            Some(bound) => collect_free_vars(&bound.clone(), arena, out),
            None => {
                if !out.contains(id) {
                    out.push(*id);
                }
            }
        },
        Type::Array(elem) => collect_free_vars(elem, arena, out),
        Type::Map(k, v) => {
            collect_free_vars(k, arena, out);
            collect_free_vars(v, arena, out);
        }
        Type::Option(inner) => collect_free_vars(inner, arena, out),
        Type::Tuple(elems) => {
            for t in elems {
                collect_free_vars(t, arena, out);
            }
        }
        Type::Function(sig) => {
            for p in &sig.params {
                collect_free_vars(p, arena, out);
            }
            collect_free_vars(&sig.return_ty, arena, out);
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => {
            collect_free_vars(yield_ty, arena, out);
            collect_free_vars(return_ty, arena, out);
        }
        Type::Union(variants) => {
            for t in variants {
                collect_free_vars(t, arena, out);
            }
        }
        Type::Void
        | Type::Bool
        | Type::Int(_)
        | Type::UInt(_)
        | Type::Float(_)
        | Type::String
        | Type::Struct(_)
        | Type::Enum(_)
        | Type::ClassRef(_)
        | Type::Dynamic
        | Type::Unknown => {}
    }
}

// ---------------------------------------------------------------------------
// UnifyError
// ---------------------------------------------------------------------------

/// Error returned by [`bind_var`] or [`unify`] on failure.
#[derive(Debug)]
pub enum UnifyError {
    /// Binding `var` to `ty` would create an infinite type.
    OccursCheck { var: TypeVarId, ty: Type },
}

// ---------------------------------------------------------------------------
// bind_var
// ---------------------------------------------------------------------------

/// Bind type variable `id` to `ty`, with occurs check and level adjustment.
///
/// - Same-variable self-binding is a no-op.
/// - Recursive types (occurs check failure) are bound to [`Type::Dynamic`] and
///   return `Ok(())` — recursive structure = genuine opacity.
/// - Free variables in `ty` whose level exceeds `id`'s level are lowered to
///   prevent escaping a generalization scope.
/// - On success, `arena.bind(id, ty)` is called.
pub fn bind_var(id: TypeVarId, ty: Type, arena: &mut TypeVarArena) -> Result<(), UnifyError> {
    // Resolve `ty` through the arena before checking.
    let ty = resolve(ty, arena);

    // Self-binding is a no-op.
    if let Type::Var(other) = ty {
        if other == id {
            return Ok(());
        }
    }

    // Occurs check: would binding create an infinite type?
    if occurs(id, &ty, arena) {
        // Treat as recursive / opaque → bind to Dynamic instead of erroring.
        arena.bind(id, Type::Dynamic);
        return Ok(());
    }

    // Level adjustment: lower any free var in `ty` that would escape our scope.
    let target_level = arena.level_of(id);
    let mut free_vars: Vec<TypeVarId> = Vec::new();
    collect_free_vars(&ty, arena, &mut free_vars);
    for fv in free_vars {
        arena.lower_level(fv, target_level);
    }

    arena.bind(id, ty);
    Ok(())
}

// ---------------------------------------------------------------------------
// unify
// ---------------------------------------------------------------------------

/// HM unification of two [`Type`] values.
///
/// Returns the unified type on success. On a concrete-type mismatch (neither is
/// a type variable or absorbing type) returns `Type::Union(vec![a, b])` rather
/// than an error — the conflict is surfaced as a union for the caller to
/// decide how to handle.
pub fn unify(a: Type, b: Type, arena: &mut TypeVarArena) -> Result<Type, UnifyError> {
    let a = resolve(a, arena);
    let b = resolve(b, arena);

    match (a, b) {
        // Identical types unify trivially.
        (a, b) if a == b => Ok(a),

        // Dynamic absorbs everything.
        (Type::Dynamic, _) | (_, Type::Dynamic) => Ok(Type::Dynamic),

        // Unknown is the top type — the concrete side wins.
        (Type::Unknown, b) => Ok(b),
        (a, Type::Unknown) => Ok(a),

        // Union — phase 2; absorb into Dynamic for now.
        (Type::Union(_), _) | (_, Type::Union(_)) => Ok(Type::Dynamic),

        // Type variable on either side.
        (Type::Var(id), b) => {
            bind_var(id, b.clone(), arena)?;
            // Return whatever `id` is now bound to (or Dynamic if recursive).
            Ok(arena.binding_of(id).cloned().unwrap_or(b))
        }
        (a, Type::Var(id)) => {
            bind_var(id, a.clone(), arena)?;
            Ok(arena.binding_of(id).cloned().unwrap_or(a))
        }

        // Structured types — unify recursively.
        (Type::Array(ea), Type::Array(eb)) => {
            let elem = unify(*ea, *eb, arena)?;
            Ok(Type::Array(Box::new(elem)))
        }
        (Type::Map(ka, va), Type::Map(kb, vb)) => {
            let k = unify(*ka, *kb, arena)?;
            let v = unify(*va, *vb, arena)?;
            Ok(Type::Map(Box::new(k), Box::new(v)))
        }
        (Type::Option(ia), Type::Option(ib)) => {
            let inner = unify(*ia, *ib, arena)?;
            Ok(Type::Option(Box::new(inner)))
        }
        (Type::Tuple(ta), Type::Tuple(tb)) => {
            if ta.len() != tb.len() {
                return Ok(Type::Union(vec![Type::Tuple(ta), Type::Tuple(tb)]));
            }
            let mut result = Vec::with_capacity(ta.len());
            for (a, b) in ta.into_iter().zip(tb) {
                result.push(unify(a, b, arena)?);
            }
            Ok(Type::Tuple(result))
        }
        (Type::Function(sa), Type::Function(sb)) => {
            if sa.params.len() != sb.params.len() {
                return Ok(Type::Union(vec![Type::Function(sa), Type::Function(sb)]));
            }
            let mut params = Vec::with_capacity(sa.params.len());
            for (pa, pb) in sa.params.into_iter().zip(sb.params) {
                params.push(unify(pa, pb, arena)?);
            }
            let return_ty = unify(sa.return_ty, sb.return_ty, arena)?;
            // Preserve metadata from `sa` (defaults, has_rest_param) when the
            // param counts are the same — merging defaults across overloads is
            // out of scope for the unifier.
            Ok(Type::Function(Box::new(FunctionSig {
                params,
                return_ty,
                defaults: sa.defaults,
                has_rest_param: sa.has_rest_param,
            })))
        }
        (
            Type::Coroutine {
                yield_ty: ya,
                return_ty: ra,
            },
            Type::Coroutine {
                yield_ty: yb,
                return_ty: rb,
            },
        ) => {
            let yield_ty = unify(*ya, *yb, arena)?;
            let return_ty = unify(*ra, *rb, arena)?;
            Ok(Type::Coroutine {
                yield_ty: Box::new(yield_ty),
                return_ty: Box::new(return_ty),
            })
        }

        // Concrete-type mismatch — fall back to Dynamic during coexistence phase.
        // Union type inference is phase 2: we don't yet have the constraint
        // vocabulary (C_UNIFY vs C_SUB) to decide when a union is appropriate vs
        // when it is spurious over-inference. Dynamic is conservative and correct.
        (_a, _b) => Ok(Type::Dynamic),
    }
}

// ---------------------------------------------------------------------------
// ConstraintSolve2 pass
// ---------------------------------------------------------------------------

/// Build a map from struct name → field name → field type from a module.
///
/// Used by the constraint solver to resolve [`TypeConstraint::HasField`]
/// constraints when the object type is a known [`Type::Struct`].
fn build_struct_fields(module: &Module) -> HashMap<String, HashMap<String, Type>> {
    let mut map: HashMap<String, HashMap<String, Type>> = HashMap::new();
    for s in &module.structs {
        let fields: HashMap<String, Type> = s
            .fields
            .iter()
            .map(|f| (f.name.clone(), f.ty.clone()))
            .collect();
        map.insert(s.name.clone(), fields);
    }
    map
}

/// HM-unifier–based constraint solver pass.
///
/// For each function in the module:
/// 1. Calls [`collect_function`] to obtain a [`ConstraintSet`].
/// 2. Processes each constraint through the HM unifier.
/// 3. Resolves the arena and back-propagates inferred types into
///    `func.value_types`, but only for values that were inference targets
///    (`Dynamic`, `Unknown`, or `Var(_)`).
pub struct ConstraintSolve2;

impl Transform for ConstraintSolve2 {
    fn name(&self) -> &str {
        "constraint-solve2"
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        use crate::ir::ty::TypeConstraint;
        use crate::transforms::constraint_collect::collect_function;

        let struct_fields = build_struct_fields(&module);

        // We must collect results before mutating module.functions to satisfy
        // the borrow checker (collect_function borrows func and module).
        struct FuncUpdate {
            // Parallel to module.functions order.
            /// (ValueId index, resolved Type) pairs to apply.
            updates: Vec<(usize, Type)>,
        }

        let func_updates: Vec<FuncUpdate> = module
            .functions
            .values()
            .map(|func| {
                let set = collect_function(func, &module);
                let crate::transforms::constraint_collect::ConstraintSet {
                    constraints,
                    mut var_arena,
                    value_vars,
                } = set;

                // --- Step 1: process constraints --------------------------------
                // We may generate additional Equal constraints from HasField and
                // Callable.  Process the original list first, then any deferred.
                let mut deferred: Vec<TypeConstraint> = Vec::new();

                let process = |c: TypeConstraint,
                               arena: &mut TypeVarArena,
                               struct_fields: &HashMap<String, HashMap<String, Type>>,
                               deferred: &mut Vec<TypeConstraint>| {
                    match c {
                        TypeConstraint::Equal(a, b) => {
                            // Ignore unification errors: on conflict we get a Union,
                            // which is fine — it won't overwrite a concrete type.
                            let _ = unify(a, b, arena);
                        }
                        TypeConstraint::Subtype { sub, sup } => {
                            // Phase 1: treat as equality.
                            let _ = unify(sub, sup, arena);
                        }
                        TypeConstraint::HasField {
                            ty,
                            field,
                            field_ty,
                        } => {
                            let resolved_ty = resolve(ty, arena);
                            match &resolved_ty {
                                Type::Struct(name) => {
                                    if let Some(fields) = struct_fields.get(name) {
                                        if let Some(ft) = fields.get(&field) {
                                            deferred
                                                .push(TypeConstraint::Equal(field_ty, ft.clone()));
                                        }
                                    }
                                    // Unknown field — skip; don't invent a type.
                                }
                                Type::Var(_) => {
                                    // Object type not yet resolved — skip for now.
                                }
                                _ => {
                                    // Dynamic, Unknown, or other — no useful info.
                                }
                            }
                        }
                        TypeConstraint::Callable { ty, args, ret } => {
                            let resolved_ty = resolve(ty, arena);
                            if let Type::Function(sig) = resolved_ty {
                                // Unify each argument type with the corresponding param.
                                for (arg_ty, param_ty) in
                                    args.into_iter().zip(sig.params.iter().cloned())
                                {
                                    deferred.push(TypeConstraint::Equal(arg_ty, param_ty));
                                }
                                // Unify return type.
                                deferred.push(TypeConstraint::Equal(ret, sig.return_ty.clone()));
                            }
                            // Var(_) or other — defer or skip.
                        }
                    }
                };

                // Process the initial constraint list.
                for c in constraints {
                    process(c, &mut var_arena, &struct_fields, &mut deferred);
                }
                // Process deferred constraints (one level deep is sufficient for phase 1).
                for c in deferred {
                    let mut noop: Vec<TypeConstraint> = Vec::new();
                    process(c, &mut var_arena, &struct_fields, &mut noop);
                }

                // --- Step 2: resolve and collect updates ----------------------
                let mut updates: Vec<(usize, Type)> = Vec::new();

                for (vid, var_id) in &value_vars {
                    let old_ty = &func.value_types[*vid];
                    // Only update values that were inference targets.
                    let is_target = matches!(old_ty, Type::Dynamic | Type::Unknown | Type::Var(_));
                    if !is_target {
                        continue;
                    }

                    let resolved = resolve(Type::Var(*var_id), &var_arena);

                    let should_update = match &resolved {
                        // Unresolved var — learned nothing.
                        Type::Var(_) => false,
                        // Unknown → no information gain.
                        Type::Unknown => false,
                        // Dynamic → update only if old was Unknown (Dynamic > Unknown).
                        Type::Dynamic => matches!(old_ty, Type::Unknown),
                        // Any other concrete type — always update.
                        _ => true,
                    };

                    if should_update {
                        updates.push((vid.index() as usize, resolved));
                    }
                }

                FuncUpdate { updates }
            })
            .collect();

        // --- Step 3: apply updates back to the module -------------------------
        let mut changed = false;
        for (func, update) in module.functions.values_mut().zip(func_updates.iter()) {
            for &(idx, ref new_ty) in &update.updates {
                use crate::entity::EntityRef;
                use crate::ir::ValueId;
                let vid = ValueId::new(idx as u32);
                if &func.value_types[vid] != new_ty {
                    func.value_types[vid] = new_ty.clone();
                    changed = true;
                }
            }
        }

        Ok(TransformResult { module, changed })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::ty::Type;

    fn fresh_arena() -> TypeVarArena {
        TypeVarArena::new()
    }

    // --- TypeVarArena ---

    #[test]
    fn arena_fresh_increments_id() {
        let mut arena = fresh_arena();
        let a = arena.fresh();
        let b = arena.fresh();
        assert_ne!(a, b);
        assert_eq!(a.index(), 0);
        assert_eq!(b.index(), 1);
    }

    #[test]
    fn arena_level_tracking() {
        let mut arena = fresh_arena();
        let a = arena.fresh();
        arena.enter_level();
        let b = arena.fresh();
        assert_eq!(arena.level_of(a), 0);
        assert_eq!(arena.level_of(b), 1);
        arena.exit_level();
        let c = arena.fresh();
        assert_eq!(arena.level_of(c), 0);
    }

    #[test]
    fn arena_bind_and_query() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        assert!(arena.binding_of(v).is_none());
        arena.bind(v, Type::Bool);
        assert_eq!(arena.binding_of(v), Some(&Type::Bool));
    }

    #[test]
    #[should_panic(expected = "already bound")]
    fn arena_double_bind_panics() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        arena.bind(v, Type::Bool);
        arena.bind(v, Type::Int(32)); // should panic
    }

    #[test]
    #[should_panic(expected = "level 0")]
    fn arena_exit_below_zero_panics() {
        let mut arena = fresh_arena();
        arena.exit_level();
    }

    // --- resolve ---

    #[test]
    fn resolve_unbound_var_is_identity() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        let ty = Type::Var(v);
        assert_eq!(resolve(ty.clone(), &arena), ty);
    }

    #[test]
    fn resolve_bound_var_follows_binding() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        arena.bind(v, Type::Int(32));
        assert_eq!(resolve(Type::Var(v), &arena), Type::Int(32));
    }

    #[test]
    fn resolve_chain() {
        let mut arena = fresh_arena();
        let a = arena.fresh();
        let b = arena.fresh();
        arena.bind(a, Type::Var(b));
        arena.bind(b, Type::String);
        assert_eq!(resolve(Type::Var(a), &arena), Type::String);
    }

    #[test]
    fn resolve_nested() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        arena.bind(v, Type::Bool);
        let ty = Type::Array(Box::new(Type::Var(v)));
        assert_eq!(resolve(ty, &arena), Type::Array(Box::new(Type::Bool)));
    }

    // --- occurs ---

    #[test]
    fn occurs_direct() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        assert!(occurs(v, &Type::Var(v), &arena));
    }

    #[test]
    fn occurs_in_array() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        let ty = Type::Array(Box::new(Type::Var(v)));
        assert!(occurs(v, &ty, &arena));
    }

    #[test]
    fn occurs_not() {
        let mut arena = fresh_arena();
        let a = arena.fresh();
        let b = arena.fresh();
        assert!(!occurs(a, &Type::Var(b), &arena));
    }

    // --- bind_var ---

    #[test]
    fn bind_var_self_noop() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        bind_var(v, Type::Var(v), &mut arena).unwrap();
        // Should remain unbound (self-binding is a no-op).
        assert!(arena.binding_of(v).is_none());
    }

    #[test]
    fn bind_var_recursive_becomes_dynamic() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        // Try to bind v to Array(v) — occurs check should catch this.
        let recursive = Type::Array(Box::new(Type::Var(v)));
        bind_var(v, recursive, &mut arena).unwrap();
        assert_eq!(arena.binding_of(v), Some(&Type::Dynamic));
    }

    #[test]
    fn bind_var_level_lowering() {
        let mut arena = fresh_arena();
        // v0 at level 0, v1 at level 1
        let v0 = arena.fresh(); // level 0
        arena.enter_level();
        let v1 = arena.fresh(); // level 1
        arena.exit_level();
        // Binding v0 to Var(v1) should lower v1's level to 0
        bind_var(v0, Type::Var(v1), &mut arena).unwrap();
        assert_eq!(arena.level_of(v1), 0);
    }

    // --- unify ---

    #[test]
    fn unify_same_concrete() {
        let mut arena = fresh_arena();
        assert_eq!(
            unify(Type::Bool, Type::Bool, &mut arena).unwrap(),
            Type::Bool
        );
    }

    #[test]
    fn unify_dynamic_absorbs() {
        let mut arena = fresh_arena();
        assert_eq!(
            unify(Type::Dynamic, Type::Int(32), &mut arena).unwrap(),
            Type::Dynamic
        );
        let mut arena = fresh_arena();
        assert_eq!(
            unify(Type::Bool, Type::Dynamic, &mut arena).unwrap(),
            Type::Dynamic
        );
    }

    #[test]
    fn unify_unknown_yields_concrete() {
        let mut arena = fresh_arena();
        assert_eq!(
            unify(Type::Unknown, Type::String, &mut arena).unwrap(),
            Type::String
        );
        let mut arena = fresh_arena();
        assert_eq!(
            unify(Type::Int(64), Type::Unknown, &mut arena).unwrap(),
            Type::Int(64)
        );
    }

    #[test]
    fn unify_var_with_concrete() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        let result = unify(Type::Var(v), Type::Float(64), &mut arena).unwrap();
        assert_eq!(result, Type::Float(64));
        assert_eq!(arena.binding_of(v), Some(&Type::Float(64)));
    }

    #[test]
    fn unify_array_element_types() {
        let mut arena = fresh_arena();
        let v = arena.fresh();
        let result = unify(
            Type::Array(Box::new(Type::Var(v))),
            Type::Array(Box::new(Type::Bool)),
            &mut arena,
        )
        .unwrap();
        assert_eq!(result, Type::Array(Box::new(Type::Bool)));
    }

    #[test]
    fn unify_tuple_pairwise() {
        let mut arena = fresh_arena();
        let result = unify(
            Type::Tuple(vec![Type::Bool, Type::Int(32)]),
            Type::Tuple(vec![Type::Bool, Type::Int(32)]),
            &mut arena,
        )
        .unwrap();
        assert_eq!(result, Type::Tuple(vec![Type::Bool, Type::Int(32)]));
    }

    #[test]
    fn unify_tuple_length_mismatch_is_union() {
        let mut arena = fresh_arena();
        let a = Type::Tuple(vec![Type::Bool]);
        let b = Type::Tuple(vec![Type::Bool, Type::Int(32)]);
        let result = unify(a.clone(), b.clone(), &mut arena).unwrap();
        assert!(matches!(result, Type::Union(_)));
    }

    #[test]
    fn unify_concrete_mismatch_is_union() {
        let mut arena = fresh_arena();
        let result = unify(Type::Bool, Type::Int(32), &mut arena).unwrap();
        assert!(
            matches!(result, Type::Union(ref v) if v.contains(&Type::Bool) && v.contains(&Type::Int(32)))
        );
    }

    #[test]
    fn unify_union_absorbs_to_dynamic() {
        let mut arena = fresh_arena();
        let result = unify(
            Type::Union(vec![Type::Bool, Type::Int(32)]),
            Type::String,
            &mut arena,
        )
        .unwrap();
        assert_eq!(result, Type::Dynamic);
    }
}
