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
use crate::ir::inst::Op;
use crate::ir::module::SystemCallTypeRule;
use crate::ir::ty::{FunctionSig, Type, TypeConstraint, TypeVarId};
use crate::ir::{Constant, Module, ValueId};
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

    /// Overwrite the binding of `id` unconditionally.
    ///
    /// Used to poison a TypeVar that was already bound when a conflicting
    /// concrete type is unified against it (e.g., rebind `String` → `Dynamic`
    /// when a second write assigns `Bool` to the same global).  Unlike
    /// [`bind`], this does not assert that `id` is unbound.
    pub fn force_rebind(&mut self, id: TypeVarId, ty: Type) {
        self.bindings[id.index() as usize] = Some(ty);
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
/// a type variable or absorbing type) returns [`Type::Dynamic`] — the conflict
/// is the conservative fallback during the coexistence phase.
///
/// TypeVar poisoning: when `a` or `b` is a bound TypeVar that resolves to a
/// conflicting concrete type, that TypeVar is force-rebound to `Dynamic` so
/// that later reads of the same TypeVar see `Dynamic` rather than the stale
/// first-bound concrete type.
pub fn unify(a: Type, b: Type, arena: &mut TypeVarArena) -> Result<Type, UnifyError> {
    // Save direct Var IDs before resolving — needed to poison them if we hit a
    // concrete-type mismatch (see the `(_a, _b)` arm below).
    let a_var = if let Type::Var(id) = &a {
        Some(*id)
    } else {
        None
    };
    let b_var = if let Type::Var(id) = &b {
        Some(*id)
    } else {
        None
    };

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
        //
        // Poison: if either input was a bound TypeVar that resolved to a concrete
        // type, force-rebind it to Dynamic.  Without this, a global TypeVar bound
        // to `String` by the first write would remain `String` even after a
        // conflicting `Bool` write is processed — the TypeVar would keep returning
        // the wrong first-write type instead of `Dynamic`.
        (_a, _b) => {
            if let Some(id) = a_var {
                arena.force_rebind(id, Type::Dynamic);
            }
            if let Some(id) = b_var {
                arena.force_rebind(id, Type::Dynamic);
            }
            Ok(Type::Dynamic)
        }
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
/// 1. Allocates one [`TypeVarId`] per global variable name (declared or
///    discovered from [`SystemCallTypeRule::GlobalStore`] /
///    [`SystemCallTypeRule::ResolveGlobalType`] ops) in a single shared
///    [`TypeVarArena`].
/// 2. Calls [`collect_function`] for every function, passing the shared arena
///    and the global-name → TypeVar map.  Each function's value vars are
///    allocated into the same arena, enabling cross-function constraints (e.g.
///    linking a `GlobalStore` write value to the global's type var, and linking
///    a `ResolveGlobalType` result to the same var).
/// 3. Solves all collected constraints jointly.
/// 4. Back-propagates inferred types into `func.value_types` (inference targets
///    only) and into `module.globals` (declared globals that were Dynamic or
///    Unknown and now have a more concrete type).
pub struct ConstraintSolve2;

/// Process a single [`TypeConstraint`], potentially emitting deferred
/// secondary constraints (from `HasField` / `Callable` resolution).
fn process_constraint(
    c: TypeConstraint,
    arena: &mut TypeVarArena,
    struct_fields: &HashMap<String, HashMap<String, Type>>,
    deferred: &mut Vec<TypeConstraint>,
) {
    match c {
        TypeConstraint::Equal(a, b) => {
            // Ignore unification errors: on conflict we get Dynamic (coexistence
            // phase — see constraint_solve2 module doc).
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
                            deferred.push(TypeConstraint::Equal(field_ty, ft.clone()));
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
                for (arg_ty, param_ty) in args.into_iter().zip(sig.params.iter().cloned()) {
                    deferred.push(TypeConstraint::Equal(arg_ty, param_ty));
                }
                deferred.push(TypeConstraint::Equal(ret, sig.return_ty.clone()));
            }
            // Var(_) or other — defer or skip.
        }
    }
}

impl Transform for ConstraintSolve2 {
    fn name(&self) -> &str {
        "constraint-solve2"
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        use crate::transforms::constraint_collect::{collect_function, is_concrete};

        let struct_fields = build_struct_fields(&module);

        // -----------------------------------------------------------------------
        // Step 1: allocate one TypeVarId per global name in a shared arena.
        //
        // We pre-scan all functions to discover undeclared story variables
        // (e.g. SugarCube `$x` written via `State.set` with no Module::globals
        // entry) so that cross-function constraints can reference them.
        // -----------------------------------------------------------------------
        let mut arena = TypeVarArena::new();
        let mut global_name_vars: HashMap<String, TypeVarId> = HashMap::new();

        // Pre-allocate TypeVarIds for all declared globals, binding concrete ones.
        for g in &module.globals {
            let v = arena.fresh();
            if is_concrete(&g.ty) {
                arena.bind(v, g.ty.clone());
            }
            global_name_vars.insert(g.name.clone(), v);
        }

        // Pre-scan functions for undeclared global names.
        if !module.system_call_type_rules.is_empty() {
            for func in module.functions.values() {
                // Build a ValueId → string map for const-string operands.
                let const_strings: HashMap<ValueId, &str> = func
                    .insts
                    .values()
                    .filter_map(|inst| {
                        if let Op::Const(Constant::String(s)) = &inst.op {
                            Some((inst.result?, s.as_str()))
                        } else {
                            None
                        }
                    })
                    .collect();

                for inst in func.insts.values() {
                    if let Op::SystemCall {
                        system,
                        method,
                        args,
                    } = &inst.op
                    {
                        let key = (system.clone(), method.clone());
                        let name_arg = match module.system_call_type_rules.get(&key) {
                            Some(SystemCallTypeRule::GlobalStore { name_arg, .. }) => *name_arg,
                            Some(
                                SystemCallTypeRule::ResolveGlobalType
                                | SystemCallTypeRule::ResolveGlobalTypeStructOnly { .. },
                            ) => 0,
                            _ => continue,
                        };
                        if name_arg < args.len() {
                            if let Some(name) = const_strings.get(&args[name_arg]) {
                                global_name_vars
                                    .entry(name.to_string())
                                    .or_insert_with(|| arena.fresh());
                            }
                        }
                    }
                }
            }
        }

        // -----------------------------------------------------------------------
        // Step 2: collect constraints from all functions into the shared arena.
        // -----------------------------------------------------------------------
        // We must collect value_vars per function before mutating the module.
        struct FuncData {
            value_vars: HashMap<ValueId, TypeVarId>,
        }

        let mut all_constraints: Vec<TypeConstraint> = Vec::new();
        let mut func_data: Vec<FuncData> = Vec::new();

        for (_, func) in module.functions.iter() {
            let set = collect_function(func, &module, &mut arena, &global_name_vars);
            all_constraints.extend(set.constraints);
            func_data.push(FuncData {
                value_vars: set.value_vars,
            });
        }

        // -----------------------------------------------------------------------
        // Step 3: solve all constraints jointly.
        // -----------------------------------------------------------------------
        let mut deferred: Vec<TypeConstraint> = Vec::new();
        for c in all_constraints {
            process_constraint(c, &mut arena, &struct_fields, &mut deferred);
        }
        // Process deferred constraints (one level deep is sufficient for phase 1).
        for c in deferred {
            process_constraint(c, &mut arena, &struct_fields, &mut Vec::new());
        }

        // -----------------------------------------------------------------------
        // Step 4: collect per-function value-type updates.
        // -----------------------------------------------------------------------
        struct FuncUpdate {
            updates: Vec<(usize, Type)>,
        }

        let func_updates: Vec<FuncUpdate> = module
            .functions
            .values()
            .zip(func_data.iter())
            .map(|(func, data)| {
                let mut updates: Vec<(usize, Type)> = Vec::new();
                for (vid, var_id) in &data.value_vars {
                    let old_ty = &func.value_types[*vid];
                    // Only update values that were inference targets.
                    if !matches!(old_ty, Type::Dynamic | Type::Unknown | Type::Var(_)) {
                        continue;
                    }
                    let resolved = resolve(Type::Var(*var_id), &arena);
                    let should_update = match &resolved {
                        Type::Var(_) | Type::Unknown => false,
                        Type::Dynamic => matches!(old_ty, Type::Unknown),
                        _ => true,
                    };
                    if should_update {
                        updates.push((vid.index() as usize, resolved));
                    }
                }
                FuncUpdate { updates }
            })
            .collect();

        // -----------------------------------------------------------------------
        // Step 5: apply per-function updates.
        // -----------------------------------------------------------------------
        let mut changed = false;
        for (func, update) in module.functions.values_mut().zip(func_updates.iter()) {
            for &(idx, ref new_ty) in &update.updates {
                let vid = ValueId::new(idx as u32);
                if &func.value_types[vid] != new_ty {
                    func.value_types[vid] = new_ty.clone();
                    changed = true;
                }
            }
        }

        // -----------------------------------------------------------------------
        // Step 6: write back improved global types to module.globals.
        //
        // Only update declared globals that were Dynamic or Unknown and now
        // have a more concrete resolved type.  Undeclared story variables are
        // not added here — TypeInference handles their discovery.
        // -----------------------------------------------------------------------
        for g in &mut module.globals {
            if let Some(&var_id) = global_name_vars.get(&g.name) {
                if matches!(g.ty, Type::Dynamic | Type::Unknown) {
                    let resolved = resolve(Type::Var(var_id), &arena);
                    if is_concrete(&resolved) {
                        g.ty = resolved;
                        changed = true;
                    }
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
    fn unify_concrete_mismatch_is_dynamic() {
        // During the coexistence phase, concrete-type mismatches fall back to
        // Dynamic rather than Union — see the "Numeric grounding limitation" note
        // in constraint_solve2.rs and TODO.md.
        let mut arena = fresh_arena();
        let result = unify(Type::Bool, Type::Int(32), &mut arena).unwrap();
        assert_eq!(result, Type::Dynamic);
    }

    #[test]
    fn unify_bound_var_conflict_poisons_var() {
        // A TypeVar already bound to String, unified with Bool:
        // result = Dynamic AND the var must be rebound to Dynamic.
        // Without the poisoning fix, the var would stay bound to String and
        // future resolve() calls would return String (first-write-wins bug).
        let mut arena = fresh_arena();
        let v = arena.fresh();
        arena.bind(v, Type::String);

        let result = unify(Type::Var(v), Type::Bool, &mut arena).unwrap();
        assert_eq!(result, Type::Dynamic);
        assert_eq!(arena.binding_of(v), Some(&Type::Dynamic));
    }

    #[test]
    fn unify_bound_var_conflict_symmetric() {
        // Symmetric: Bool on the left, Var(v) bound to String on the right.
        let mut arena = fresh_arena();
        let v = arena.fresh();
        arena.bind(v, Type::String);

        let result = unify(Type::Bool, Type::Var(v), &mut arena).unwrap();
        assert_eq!(result, Type::Dynamic);
        assert_eq!(arena.binding_of(v), Some(&Type::Dynamic));
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
