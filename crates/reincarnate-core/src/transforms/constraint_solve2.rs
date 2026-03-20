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
use crate::ir::ty::{FunctionSig, Type, TypeConstraint, TypeId, TypeVarId};
use crate::ir::{Constant, FuncId, Module, ValueId};
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
    /// concrete type is unified against it (e.g., rebind `String` → `Unknown`
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
        | Type::Instance(_)
        | Type::Struct(_)
        | Type::Enum(_)
        | Type::ClassRef(_)
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
        | Type::Instance(_)
        | Type::Struct(_)
        | Type::Enum(_)
        | Type::ClassRef(_)
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
        | Type::Instance(_)
        | Type::Struct(_)
        | Type::Enum(_)
        | Type::ClassRef(_)
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
/// - Recursive types (occurs check failure) are bound to [`Type::Unknown`] and
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
        // Treat as recursive / opaque → bind to Unknown instead of erroring.
        arena.bind(id, Type::Unknown);
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
/// a type variable or absorbing type) returns [`Type::Unknown`] — the conflict
/// is the conservative fallback during the coexistence phase.
///
/// TypeVar poisoning: when `a` or `b` is a bound TypeVar that resolves to a
/// conflicting concrete type, that TypeVar is force-rebound to `Unknown` so
/// that later reads of the same TypeVar see `Unknown` rather than the stale
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

        // Unknown absorbs everything — conservative fallback for inference gaps.
        (Type::Unknown, _) | (_, Type::Unknown) => Ok(Type::Unknown),

        // Union — phase 2; absorb into Unknown for now.
        (Type::Union(_), _) | (_, Type::Union(_)) => Ok(Type::Unknown),

        // Type variable on either side.
        (Type::Var(id), b) => {
            bind_var(id, b.clone(), arena)?;
            // Return whatever `id` is now bound to (or Unknown if recursive).
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

        // Concrete-type mismatch — fall back to Unknown during coexistence phase.
        // Union type inference is phase 2: we don't yet have the constraint
        // vocabulary (C_UNIFY vs C_SUB) to decide when a union is appropriate vs
        // when it is spurious over-inference. Unknown is conservative and correct.
        //
        // Poison: if either input was a bound TypeVar that resolved to a concrete
        // type, force-rebind it to Unknown.  Without this, a global TypeVar bound
        // to `String` by the first write would remain `String` even after a
        // conflicting `Bool` write is processed — the TypeVar would keep returning
        // the wrong first-write type instead of `Unknown`.
        (_a, _b) => {
            if let Some(id) = a_var {
                arena.force_rebind(id, Type::Unknown);
            }
            if let Some(id) = b_var {
                arena.force_rebind(id, Type::Unknown);
            }
            Ok(Type::Unknown)
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
///    only) and into `module.globals` (declared globals that were Unknown or
///    Unknown and now have a more concrete type).
pub struct ConstraintSolve2;

/// Process a single [`TypeConstraint`], potentially emitting deferred
/// secondary constraints (from `HasField` / `Callable` resolution).
fn process_constraint(
    c: TypeConstraint,
    arena: &mut TypeVarArena,
    struct_fields: &HashMap<String, HashMap<String, Type>>,
    type_id_to_name: &HashMap<TypeId, String>,
    deferred: &mut Vec<TypeConstraint>,
) {
    match c {
        TypeConstraint::Equal(a, b) => {
            // Ignore unification errors: on conflict we get Unknown (coexistence
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
            match resolved_ty {
                Type::Instance(id) => {
                    if let Some(name) = type_id_to_name.get(&id) {
                        if let Some(fields) = struct_fields.get(name) {
                            if let Some(ft) = fields.get(&field) {
                                deferred.push(TypeConstraint::Equal(field_ty, ft.clone()));
                            }
                        }
                    }
                    // Unknown field — skip; don't invent a type.
                }
                Type::Var(_) => {
                    // Object type not yet resolved — re-defer so the fixpoint
                    // loop retries once the Var is bound by an Equal constraint.
                    deferred.push(TypeConstraint::HasField {
                        ty: resolved_ty,
                        field,
                        field_ty,
                    });
                }
                _ => {
                    // Unknown or other — no useful info.
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
            } else if matches!(resolved_ty, Type::Var(_)) {
                // Callee type not yet resolved — re-defer.
                deferred.push(TypeConstraint::Callable {
                    ty: resolved_ty,
                    args,
                    ret,
                });
            }
            // Other — no useful info.
        }
    }
}

impl Transform for ConstraintSolve2 {
    fn name(&self) -> &str {
        "constraint-solve2"
    }

    fn requires(&self) -> &[&str] {
        &["type-inference"]
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        use crate::transforms::constraint_collect::{collect_function, is_concrete};

        let struct_fields = build_struct_fields(&module);
        let type_id_to_name: HashMap<TypeId, String> = module
            .types
            .iter()
            .map(|(id, nt)| (id, nt.name.clone()))
            .collect();

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
            return_var: TypeVarId,
        }

        let mut all_constraints: Vec<TypeConstraint> = Vec::new();
        let mut func_data: Vec<FuncData> = Vec::new();

        for (_, func) in module.functions.iter() {
            let set = collect_function(func, &module, &mut arena, &global_name_vars);
            all_constraints.extend(set.constraints);
            func_data.push(FuncData {
                value_vars: set.value_vars,
                return_var: set.return_var,
            });
        }

        // -----------------------------------------------------------------------
        // Step 2b: emit interprocedural call-site constraints.
        {
            //
            // For every Op::Call and Op::MethodCall, link the caller's argument
            // type vars to the callee's entry block param type vars in the shared
            // arena.  This allows the HM unifier to flow concrete types from
            // callers into callees (and vice versa) across function boundaries.
            //
            // Self-calls (recursive) are skipped to avoid circular reasoning.
            // Params used as collections (GetIndex, .length) are skipped to avoid
            // over-narrowing arrays/maps to numeric types.
            // -----------------------------------------------------------------------
            {
                use crate::transforms::call_site_flow::{
                    param_used_as_collection, param_used_with_field_access,
                };
                // Build name → (func_data_index, FuncId) map.
                let name_to_idx: HashMap<&str, (usize, FuncId)> = module
                    .functions
                    .keys()
                    .enumerate()
                    .map(|(idx, fid)| (module.func_name(fid), (idx, fid)))
                    .collect();

                for (caller_idx, (fid, func)) in module.functions.iter().enumerate() {
                    let caller_name = module.func_name(fid);
                    let caller_data = &func_data[caller_idx];

                    for block in func.blocks.values() {
                        for &inst_id in &block.insts {
                            let inst = &func.insts[inst_id];
                            match &inst.op {
                                Op::Call {
                                    func: callee_name,
                                    args,
                                } => {
                                    // Skip self-calls.
                                    if callee_name == caller_name {
                                        continue;
                                    }
                                    if let Some(&(callee_idx, callee_fid)) =
                                        name_to_idx.get(callee_name.as_str())
                                    {
                                        let callee_func = &module.functions[callee_fid];
                                        let callee_data = &func_data[callee_idx];
                                        let entry = callee_func.entry;
                                        let entry_params = &callee_func.blocks[entry].params;

                                        for (i, &arg) in args.iter().enumerate() {
                                            if i >= entry_params.len() {
                                                break;
                                            }
                                            // Only emit constraint when the caller arg
                                            // has a concrete type. Unknown args are
                                            // abstentions — they should not pull the
                                            // callee param toward Unknown or poison
                                            // shared type variables in the arena.
                                            let arg_ty = &func.value_types[arg];
                                            if matches!(arg_ty, Type::Unknown) {
                                                continue;
                                            }
                                            let param_val = entry_params[i].value;
                                            // Skip when the callee param is already
                                            // concrete — earlier passes' evidence is
                                            // more reliable.
                                            let param_ty = &callee_func.value_types[param_val];
                                            if is_concrete(param_ty) {
                                                continue;
                                            }
                                            // Guard against narrowing params the body
                                            // uses as collections/objects — but only when
                                            // the arg type is not already a struct. Struct
                                            // args must propagate through so that HasField
                                            // constraints on the param can resolve.
                                            let is_struct_arg = matches!(
                                                arg_ty,
                                                Type::Instance(_) | Type::ClassRef(_)
                                            );
                                            let is_self_param = i == 0;
                                            if is_self_param {
                                                if !is_struct_arg
                                                    && param_used_as_collection(
                                                        callee_func,
                                                        param_val,
                                                    )
                                                {
                                                    continue;
                                                }
                                            } else if !is_struct_arg
                                                && param_used_with_field_access(
                                                    callee_func,
                                                    param_val,
                                                )
                                            {
                                                continue;
                                            }
                                            if let (Some(&arg_var), Some(&param_var)) = (
                                                caller_data.value_vars.get(&arg),
                                                callee_data.value_vars.get(&param_val),
                                            ) {
                                                all_constraints.push(TypeConstraint::Equal(
                                                    Type::Var(arg_var),
                                                    Type::Var(param_var),
                                                ));
                                            }
                                        }

                                        // Link caller result ← callee return_var (unconditional,
                                        // not gated on sig.return_ty). The HM joint arena
                                        // propagates concrete return types inferred from callee
                                        // bodies even when sig.return_ty is still Unknown.
                                        // Skip when sig.return_ty is already Void — void functions
                                        // have no meaningful return value; propagating Void to the
                                        // caller result would produce spurious type errors in GML
                                        // where void calls in value position are valid.
                                        if let Some(result) = inst.result {
                                            if !matches!(callee_func.sig.return_ty, Type::Void) {
                                                if let Some(&result_var) =
                                                    caller_data.value_vars.get(&result)
                                                {
                                                    all_constraints.push(TypeConstraint::Equal(
                                                        Type::Var(result_var),
                                                        Type::Var(callee_data.return_var),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                                Op::MethodCall {
                                    method,
                                    args,
                                    receiver,
                                } => {
                                    // Skip self-calls.
                                    if method == caller_name {
                                        continue;
                                    }
                                    if let Some(&(callee_idx, callee_fid)) =
                                        name_to_idx.get(method.as_str())
                                    {
                                        let callee_func = &module.functions[callee_fid];
                                        let callee_data = &func_data[callee_idx];
                                        let entry = callee_func.entry;
                                        let entry_params = &callee_func.blocks[entry].params;

                                        // Link receiver to param[0] (self).
                                        if !entry_params.is_empty() {
                                            let recv_ty = &func.value_types[*receiver];
                                            let param_val = entry_params[0].value;
                                            let param_ty = &callee_func.value_types[param_val];
                                            if !matches!(recv_ty, Type::Unknown)
                                                && !is_concrete(param_ty)
                                                && !param_used_as_collection(callee_func, param_val)
                                            {
                                                if let (Some(&recv_var), Some(&param_var)) = (
                                                    caller_data.value_vars.get(receiver),
                                                    callee_data.value_vars.get(&param_val),
                                                ) {
                                                    all_constraints.push(TypeConstraint::Equal(
                                                        Type::Var(recv_var),
                                                        Type::Var(param_var),
                                                    ));
                                                }
                                            }
                                        }

                                        // Link args to params[1..] (skip self).
                                        for (i, &arg) in args.iter().enumerate() {
                                            let param_idx = i + 1; // skip self
                                            if param_idx >= entry_params.len() {
                                                break;
                                            }
                                            let arg_ty = &func.value_types[arg];
                                            if matches!(arg_ty, Type::Unknown) {
                                                continue;
                                            }
                                            let param_val = entry_params[param_idx].value;
                                            let param_ty = &callee_func.value_types[param_val];
                                            if is_concrete(param_ty) {
                                                continue;
                                            }
                                            let is_struct_arg = matches!(
                                                arg_ty,
                                                Type::Instance(_) | Type::ClassRef(_)
                                            );
                                            if !is_struct_arg
                                                && param_used_with_field_access(
                                                    callee_func,
                                                    param_val,
                                                )
                                            {
                                                continue;
                                            }
                                            if let (Some(&arg_var), Some(&param_var)) = (
                                                caller_data.value_vars.get(&arg),
                                                callee_data.value_vars.get(&param_val),
                                            ) {
                                                all_constraints.push(TypeConstraint::Equal(
                                                    Type::Var(arg_var),
                                                    Type::Var(param_var),
                                                ));
                                            }
                                        }

                                        // Link caller result ← callee return_var (unconditional,
                                        // not gated on sig.return_ty). The HM joint arena
                                        // propagates concrete return types inferred from callee
                                        // bodies even when sig.return_ty is still Unknown.
                                        // Skip when sig.return_ty is already Void — void functions
                                        // have no meaningful return value; propagating Void to the
                                        // caller result would produce spurious type errors in GML
                                        // where void calls in value position are valid.
                                        if let Some(result) = inst.result {
                                            if !matches!(callee_func.sig.return_ty, Type::Void) {
                                                if let Some(&result_var) =
                                                    caller_data.value_vars.get(&result)
                                                {
                                                    all_constraints.push(TypeConstraint::Equal(
                                                        Type::Var(result_var),
                                                        Type::Var(callee_data.return_var),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                                _ => {}
                            }
                        }
                    }
                }
            }
        } // end step 2b

        // -----------------------------------------------------------------------
        // Step 3: solve all constraints jointly.
        //
        // `HasField { ty: Var(_) }` and `Callable { ty: Var(_) }` constraints
        // cannot be resolved until the object/callee type variable is bound by
        // a later `Equal` constraint. We run a fixpoint loop: each pass
        // processes the pending list, and any constraint that still cannot be
        // resolved is re-deferred. We stop when either:
        //   (a) the deferred list is empty (all resolved), or
        //   (b) a full pass made no progress (deferred list is no shorter than
        //       before — the remaining constraints are genuinely unresolvable).
        // -----------------------------------------------------------------------
        let mut pending: Vec<TypeConstraint> = all_constraints;
        loop {
            let pending_count = pending.len();
            let mut deferred: Vec<TypeConstraint> = Vec::new();
            for c in pending {
                process_constraint(
                    c,
                    &mut arena,
                    &struct_fields,
                    &type_id_to_name,
                    &mut deferred,
                );
            }
            if deferred.is_empty() {
                break; // all resolved
            }
            if deferred.len() >= pending_count {
                // No progress this pass — remaining constraints are unresolvable.
                break;
            }
            pending = deferred;
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
                    if !matches!(old_ty, Type::Unknown | Type::Var(_)) {
                        continue;
                    }
                    let resolved = resolve(Type::Var(*var_id), &arena);
                    if !matches!(&resolved, Type::Var(_) | Type::Unknown) {
                        updates.push((vid.index() as usize, resolved));
                    }
                }
                FuncUpdate { updates }
            })
            .collect();

        // -----------------------------------------------------------------------
        // Step 5: apply per-function updates (value_types, sig.params, block
        // param types, sig.return_ty).
        // -----------------------------------------------------------------------
        use crate::pipeline::checker::{Diagnostic, DiagnosticCode, RcDiagnostic, Severity};
        let mut changed = false;
        let mut new_diagnostics: Vec<Diagnostic> = Vec::new();
        for ((func, update), data) in module
            .functions
            .values_mut()
            .zip(func_updates.iter())
            .zip(func_data.iter())
        {
            for &(idx, ref new_ty) in &update.updates {
                let vid = ValueId::new(idx as u32);
                if &func.value_types[vid] != new_ty {
                    func.value_types[vid] = new_ty.clone();
                    changed = true;
                }
            }

            // Sync entry block param.ty and sig.params for params that CS2
            // narrowed from Unknown to concrete. Do NOT overwrite
            // already-concrete values — earlier passes have more reliable
            // evidence.
            let entry = func.entry;
            let entry_param_count = func.blocks[entry].params.len();
            for i in 0..entry_param_count {
                let p_value = func.blocks[entry].params[i].value;
                let p_ty = func.blocks[entry].params[i].ty.clone();
                let vty = func.value_types[p_value].clone();
                // Sync block param.ty ← value_types (only Unknown→concrete).
                if matches!(p_ty, Type::Unknown) && !matches!(vty, Type::Unknown | Type::Var(_)) {
                    func.blocks[entry].params[i].ty = vty.clone();
                    changed = true;
                }
                // Sync sig.params ← value_types (only Unknown→concrete).
                if i < func.sig.params.len()
                    && matches!(func.sig.params[i], Type::Unknown)
                    && !matches!(vty, Type::Unknown | Type::Var(_))
                {
                    func.sig.params[i] = vty.clone();
                    changed = true;
                }
            }

            // Sync sig.return_ty ← resolved return_var (only Unknown→concrete).
            // If sig.return_ty is already concrete but conflicts with the solver's
            // inference, emit RC1002 and keep the existing sig (TypeInference
            // evidence is more reliable).
            let resolved_ret = resolve(Type::Var(data.return_var), &arena);
            if is_concrete(&resolved_ret) {
                if matches!(func.sig.return_ty, Type::Unknown) {
                    func.sig.return_ty = resolved_ret;
                    changed = true;
                } else if func.sig.return_ty != resolved_ret {
                    // Conflict: TypeInference set a concrete return type that
                    // differs from what the constraint solver inferred.  Keep
                    // sig.return_ty unchanged and report the disagreement.
                    new_diagnostics.push(Diagnostic {
                        file: func.name.clone(),
                        line: 0,
                        col: 0,
                        code: DiagnosticCode::Rc(RcDiagnostic::InferenceConflict),
                        severity: Severity::Warning,
                        message: format!(
                            "return type inferred as {:?} by constraint solver conflicts with {:?} from TypeInference (keeping {:?})",
                            resolved_ret,
                            func.sig.return_ty,
                            func.sig.return_ty,
                        ),
                    });
                }
            }
        }
        module.diagnostics.append(&mut new_diagnostics);

        // -----------------------------------------------------------------------
        // Step 6: write back improved global types to module.globals.
        //
        // Only update declared globals that were Unknown and now
        // have a more concrete resolved type.  Undeclared story variables are
        // not added here — TypeInference handles their discovery.
        // -----------------------------------------------------------------------
        for g in &mut module.globals {
            if let Some(&var_id) = global_name_vars.get(&g.name) {
                if matches!(g.ty, Type::Unknown) {
                    let resolved = resolve(Type::Var(var_id), &arena);
                    if is_concrete(&resolved) {
                        g.ty = resolved;
                        changed = true;
                    }
                }
            }
        }

        // -----------------------------------------------------------------------
        // Step 7: emit inference failure diagnostics for values that remain
        // Unknown after solving.
        // -----------------------------------------------------------------------
        // RC1001 — value had no constraints touching it (arena binding is None).
        // RC1005 — value was constrained but all constraints resolved to Unknown.
        // -----------------------------------------------------------------------
        {
            use crate::pipeline::checker::{Diagnostic, DiagnosticCode, RcDiagnostic, Severity};
            let mut step7_diagnostics: Vec<Diagnostic> = Vec::new();

            for ((fid, func), data) in module.functions.iter().zip(func_data.iter()) {
                // Build ValueId → Op::variant_name map.
                let mut value_op: HashMap<ValueId, &'static str> = HashMap::new();
                for inst in func.insts.values() {
                    if let Some(result) = inst.result {
                        value_op.insert(result, inst.op.variant_name());
                    }
                }

                let func_name = module.func_name(fid).to_string();

                for (vid, &var_id) in &data.value_vars {
                    // Only report values that are still Unknown after Step 5.
                    if !matches!(func.value_types[*vid], Type::Unknown) {
                        continue;
                    }

                    // Skip Alloc and Load results — they are not inference targets.
                    let op_name = match value_op.get(vid) {
                        Some(&name) => name,
                        None => continue, // block param or other non-inst value
                    };
                    if op_name == "Alloc" || op_name == "Load" {
                        continue;
                    }

                    // Determine whether the var was ever bound in the arena.
                    let binding = arena.binding_of(var_id);
                    let (code, message) = if binding.is_none() {
                        // Var was never unified with anything — no constraints touched it.
                        (
                            DiagnosticCode::Rc(RcDiagnostic::InferenceNoConstraints),
                            format!(
                                "value {:?} remains Unknown: no constraints (produced by Op::{})",
                                vid, op_name
                            ),
                        )
                    } else {
                        // Var was unified but all constraints resolved to Unknown.
                        (
                            DiagnosticCode::Rc(RcDiagnostic::InferenceInheritedUnknown),
                            format!(
                                "value {:?} remains Unknown: all constraints resolved to Unknown (produced by Op::{})",
                                vid, op_name
                            ),
                        )
                    };

                    step7_diagnostics.push(Diagnostic {
                        file: func_name.clone(),
                        line: 0,
                        col: 0,
                        code,
                        severity: Severity::Warning,
                        message,
                    });
                }
            }

            module.diagnostics.append(&mut step7_diagnostics);
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
        assert_eq!(arena.binding_of(v), Some(&Type::Unknown));
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
    fn unify_unknown_absorbs() {
        let mut arena = fresh_arena();
        assert_eq!(
            unify(Type::Unknown, Type::Int(32), &mut arena).unwrap(),
            Type::Unknown
        );
        let mut arena = fresh_arena();
        assert_eq!(
            unify(Type::Bool, Type::Unknown, &mut arena).unwrap(),
            Type::Unknown
        );
        let mut arena = fresh_arena();
        assert_eq!(
            unify(Type::Unknown, Type::String, &mut arena).unwrap(),
            Type::Unknown
        );
        let mut arena = fresh_arena();
        assert_eq!(
            unify(Type::Int(64), Type::Unknown, &mut arena).unwrap(),
            Type::Unknown
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
        // Unknown rather than Union — see the "Numeric grounding limitation" note
        // in constraint_solve2.rs and TODO.md.
        let mut arena = fresh_arena();
        let result = unify(Type::Bool, Type::Int(32), &mut arena).unwrap();
        assert_eq!(result, Type::Unknown);
    }

    #[test]
    fn unify_bound_var_conflict_poisons_var() {
        // A TypeVar already bound to String, unified with Bool:
        // result = Unknown AND the var must be rebound to Unknown.
        // Without the poisoning fix, the var would stay bound to String and
        // future resolve() calls would return String (first-write-wins bug).
        let mut arena = fresh_arena();
        let v = arena.fresh();
        arena.bind(v, Type::String);

        let result = unify(Type::Var(v), Type::Bool, &mut arena).unwrap();
        assert_eq!(result, Type::Unknown);
        assert_eq!(arena.binding_of(v), Some(&Type::Unknown));
    }

    #[test]
    fn unify_bound_var_conflict_symmetric() {
        // Symmetric: Bool on the left, Var(v) bound to String on the right.
        let mut arena = fresh_arena();
        let v = arena.fresh();
        arena.bind(v, Type::String);

        let result = unify(Type::Bool, Type::Var(v), &mut arena).unwrap();
        assert_eq!(result, Type::Unknown);
        assert_eq!(arena.binding_of(v), Some(&Type::Unknown));
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
        assert_eq!(result, Type::Unknown);
    }
}
