//! Constraint collection pass (HM type-inference).
//!
//! Walks every IR function and emits [`TypeConstraint`] values for every [`Op`]
//! that carries type information.  No solving happens here — collection only.
//!
//! The collected constraints are stored in [`ConstraintSet`] values, one per
//! function, and accumulated in [`ConstraintCollect::constraint_sets`].
//!
//! This module also owns the [`TypeVarArena`] and HM unifier primitives
//! ([`resolve`], [`occurs`], [`bind_var`], [`unify`]) used by the solver pass.

use std::collections::{HashMap, HashSet};

use crate::entity::EntityRef;
use crate::error::CoreError;
use crate::ir::block::BlockId;
use crate::ir::inst::Op;
use crate::ir::module::SystemCallTypeRule;
use crate::ir::ty::{FunctionSig, Type, TypeConstraint, TypeVarId};
use crate::ir::{Constant, Function, Module, ValueId};
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
    /// concrete type is unified against it.  Unlike [`bind`], this does not
    /// assert that `id` is unbound.
    pub fn force_rebind(&mut self, id: TypeVarId, ty: Type) {
        self.bindings[id.index() as usize] = Some(ty);
    }

    /// Lower the level of `id` to `new_level` (only if currently higher).
    pub(crate) fn lower_level(&mut self, id: TypeVarId, new_level: u32) {
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
    let resolved = resolve(ty.clone(), arena);
    occurs_resolved(id, &resolved, arena)
}

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
        | Type::ClassRef(_)
        | Type::Unknown => false,
    }
}

// ---------------------------------------------------------------------------
// collect_free_vars (internal)
// ---------------------------------------------------------------------------

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
    let ty = resolve(ty, arena);

    if let Type::Var(other) = ty {
        if other == id {
            return Ok(());
        }
        // Bind to other var — level adjustment only (no occurs check needed for Var→Var).
        let target_level = arena.level_of(id);
        arena.lower_level(other, target_level);
        arena.bind(id, Type::Var(other));
        return Ok(());
    }

    if occurs(id, &ty, arena) {
        arena.bind(id, Type::Unknown);
        return Ok(());
    }

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
/// Returns the unified type on success.  On a concrete-type mismatch (neither
/// is a type variable or absorbing type) returns [`Type::Unknown`] — the
/// conflict is the conservative fallback.
///
/// TypeVar poisoning: when `a` or `b` is a bound TypeVar that resolves to a
/// conflicting concrete type, that TypeVar is force-rebound to `Unknown` so
/// that later reads of the same TypeVar see `Unknown` rather than the stale
/// first-bound concrete type.
pub fn unify(a: Type, b: Type, arena: &mut TypeVarArena) -> Result<Type, UnifyError> {
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
        (a, b) if a == b => Ok(a),
        (Type::Unknown, _) | (_, Type::Unknown) => Ok(Type::Unknown),
        (Type::Union(_), _) | (_, Type::Union(_)) => Ok(Type::Unknown),

        (Type::Var(id), b) => {
            bind_var(id, b.clone(), arena)?;
            Ok(arena.binding_of(id).cloned().unwrap_or(b))
        }
        (a, Type::Var(id)) => {
            bind_var(id, a.clone(), arena)?;
            Ok(arena.binding_of(id).cloned().unwrap_or(a))
        }

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

        // Concrete-type mismatch — fall back to Unknown.
        // Poison bound TypeVars so they return Unknown on future resolve().
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
// ConstraintSet
// ---------------------------------------------------------------------------

/// Constraints collected from a single function.
pub struct ConstraintSet {
    /// All type constraints emitted while walking the function.
    pub constraints: Vec<TypeConstraint>,
    /// Map from [`ValueId`] → [`TypeVarId`].
    ///
    /// Pre-populated during initialisation:
    /// - Concrete ground types get a fresh var that is immediately bound to
    ///   that type.
    /// - [`Type::Unknown`] and [`Type::Var`] get fresh,
    ///   unbound vars (inference targets).
    pub value_vars: HashMap<ValueId, TypeVarId>,
    /// TypeVarId allocated for the function's return type.
    ///
    /// Linked to return values via `Return` terminator constraints.  Exposed
    /// so that interprocedural call-site linking (Step 2b in ConstraintSolve2)
    /// can unify a caller's result type var with the callee's return type var,
    /// enabling bidirectional return-type propagation even when the callee's
    /// `sig.return_ty` is not yet concrete.
    pub return_var: TypeVarId,
}

// ---------------------------------------------------------------------------
// Helper: is a type a concrete ground type (fully known, no inference needed)?
// ---------------------------------------------------------------------------

/// Returns `true` for types that do not contain unresolved variables and are
/// not inference placeholders.
///
/// Compound types (Array, Map, …) whose inner types are all concrete are also
/// considered concrete.
pub(crate) fn is_concrete(ty: &Type) -> bool {
    match ty {
        Type::Unknown => true, // decided: inference exhausted, not a free variable
        Type::Var(_) => false,
        Type::Array(elem) => is_concrete(elem),
        Type::Map(k, v) => is_concrete(k) && is_concrete(v),
        Type::Option(inner) => is_concrete(inner),
        Type::Tuple(elems) => elems.iter().all(is_concrete),
        Type::Union(variants) => variants.iter().all(is_concrete),
        Type::Function(sig) => sig.params.iter().all(is_concrete) && is_concrete(&sig.return_ty),
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => is_concrete(yield_ty) && is_concrete(return_ty),
        // All leaf concrete types.
        Type::Void
        | Type::Bool
        | Type::Int(_)
        | Type::UInt(_)
        | Type::Float(_)
        | Type::String
        | Type::Instance(_)
        | Type::ClassRef(_) => true,
    }
}

// ---------------------------------------------------------------------------
// collect_function
// ---------------------------------------------------------------------------

/// Walk `func` and collect all type constraints into a [`ConstraintSet`].
///
/// `arena` is the shared type-variable allocator.  Callers manage the arena
/// lifetime; [`ConstraintSet`] does not own it.
///
/// `global_name_vars` maps global variable names to their [`TypeVarId`]s in
/// `arena`.  Reserved for future cross-function global constraint emission;
/// not yet used (see TODO.md — pipeline ordering for global HM inference).
pub fn collect_function(
    func: &Function,
    module: &Module,
    arena: &mut TypeVarArena,
    global_name_vars: &HashMap<String, TypeVarId>,
) -> ConstraintSet {
    let mut value_vars: HashMap<ValueId, TypeVarId> = HashMap::new();

    // -----------------------------------------------------------------------
    // Phase 1 — allocate a TypeVarId for every value in value_types.
    // -----------------------------------------------------------------------
    for (vid, ty) in func.value_types.iter() {
        let var = arena.fresh();
        if is_concrete(ty) {
            arena.bind(var, ty.clone());
        }
        // Unknown / Unknown / Var(_) → leave unbound (inference target).
        value_vars.insert(vid, var);
    }

    // Allocate a TypeVarId for the function's return type.
    let return_var: TypeVarId = arena.fresh();
    let return_ty = &func.sig.return_ty;
    if is_concrete(return_ty) {
        arena.bind(return_var, return_ty.clone());
    }

    let mut constraints: Vec<TypeConstraint> = Vec::new();

    // -----------------------------------------------------------------------
    // Helper: emit a constraint only when both values have registered vars.
    // -----------------------------------------------------------------------
    let var_for = |value: ValueId, vv: &HashMap<ValueId, TypeVarId>| -> Option<Type> {
        vv.get(&value).copied().map(Type::Var)
    };

    // -----------------------------------------------------------------------
    // Build function name → FunctionSig map for Call/MethodCall constraints.
    // -----------------------------------------------------------------------
    let func_sigs: HashMap<&str, &FunctionSig> = module
        .functions
        .keys()
        .map(|fid| (module.func_name(fid), &module.functions[fid].sig))
        .collect();

    // -----------------------------------------------------------------------
    // Pre-compute SystemCall rule tables and const-string map.
    //
    // These are used in Phase 2 to emit GlobalStore / ResolveGlobalType
    // constraints that link write/read values to global TypeVars.
    // -----------------------------------------------------------------------
    let store_rules: HashMap<(&str, &str), (usize, usize)> = module
        .system_call_type_rules
        .iter()
        .filter_map(|((sys, meth), rule)| {
            if let SystemCallTypeRule::GlobalStore {
                name_arg,
                value_arg,
            } = rule
            {
                Some(((sys.as_str(), meth.as_str()), (*name_arg, *value_arg)))
            } else {
                None
            }
        })
        .collect();

    // Only ResolveGlobalType (e.g. State.get) emits Equal constraints.
    // ResolveGlobalTypeStructOnly (e.g. Engine.resolve) is excluded: it is
    // also used for JS built-ins whose default TS overload returns `unknown`,
    // so linking those calls through a shared global TypeVar causes false
    // TS2571 regressions when unrelated uses constrain the TypeVar unexpectedly.
    let resolve_rules: HashSet<(&str, &str)> = module
        .system_call_type_rules
        .iter()
        .filter_map(|((sys, meth), rule)| {
            if matches!(rule, SystemCallTypeRule::ResolveGlobalType) {
                Some((sys.as_str(), meth.as_str()))
            } else {
                None
            }
        })
        .collect();

    // ResolveInstanceField (e.g. GameMaker.Instance.getField / getOn): emit HasField
    // constraints so the solver can propagate field types once the receiver type is known.
    let field_rules: HashSet<(&str, &str)> = module
        .system_call_type_rules
        .iter()
        .filter_map(|((sys, meth), rule)| {
            if matches!(rule, SystemCallTypeRule::ResolveInstanceField) {
                Some((sys.as_str(), meth.as_str()))
            } else {
                None
            }
        })
        .collect();

    // Const-string map: ValueId → string literal value.
    // Only built when there are SystemCall rules to process.
    let const_strings: HashMap<ValueId, &str> =
        if store_rules.is_empty() && resolve_rules.is_empty() && field_rules.is_empty() {
            HashMap::new()
        } else {
            func.insts
                .values()
                .filter_map(|inst| {
                    if let Op::Const(Constant::String(s)) = &inst.op {
                        Some((inst.result?, s.as_str()))
                    } else {
                        None
                    }
                })
                .collect()
        };

    // -----------------------------------------------------------------------
    // Phase 2 — walk blocks and emit constraints per Op.
    // -----------------------------------------------------------------------
    for (block_id, block) in func.blocks.iter() {
        // -- Block param phi-merge constraints --------------------------------
        // For every predecessor that branches to this block, emit Equal
        // constraints pairing each passed argument with the corresponding
        // block param.
        //
        // We gather the predecessor arg lists from branch ops in all other
        // blocks rather than maintaining a reverse-edge map, which keeps the
        // code simple at the cost of an extra scan.  For the sizes we operate
        // on this is fine.
        if !block.params.is_empty() {
            emit_phi_constraints(
                block_id,
                block,
                func,
                &value_vars,
                &var_for,
                &mut constraints,
            );
        }

        // -- Instruction constraints ------------------------------------------
        for &inst_id in &block.insts {
            let inst = &func.insts[inst_id];
            let result_var = inst.result.and_then(|r| var_for(r, &value_vars));

            match &inst.op {
                // Constants — ground the result to the literal's type.
                Op::Const(constant) => {
                    if let Some(rv) = result_var {
                        let ty = constant.ty();
                        if is_concrete(&ty) {
                            constraints.push(TypeConstraint::Equal(rv, ty));
                        }
                    }
                }

                // GlobalRef — link result to the global's shared type var.
                Op::GlobalRef(name) => {
                    if let Some(rv) = result_var {
                        if let Some(&gvar) = global_name_vars.get(name.as_str()) {
                            constraints.push(TypeConstraint::Equal(rv, Type::Var(gvar)));
                        }
                    }
                }

                // Op::Add is excluded — overloaded for string concatenation in GML and AS3,
                // so result type cannot be assumed to match operand types. Correct general
                // behavior is Phase 9 (arithmetic ops as typed builtin calls).
                Op::Add(_, _) => {}

                // Interim: propagate operand type to result. Phase 9 replaces with builtin signatures.
                Op::Sub(a, b) => {
                    if let (Some(a_var), Some(b_var), Some(rv)) = (
                        var_for(*a, &value_vars),
                        var_for(*b, &value_vars),
                        result_var,
                    ) {
                        constraints.push(TypeConstraint::Equal(rv.clone(), a_var));
                        constraints.push(TypeConstraint::Equal(rv, b_var));
                    }
                }
                Op::Mul(a, b) => {
                    if let (Some(a_var), Some(b_var), Some(rv)) = (
                        var_for(*a, &value_vars),
                        var_for(*b, &value_vars),
                        result_var,
                    ) {
                        constraints.push(TypeConstraint::Equal(rv.clone(), a_var));
                        constraints.push(TypeConstraint::Equal(rv, b_var));
                    }
                }
                Op::Div(a, b) => {
                    if let (Some(a_var), Some(b_var), Some(rv)) = (
                        var_for(*a, &value_vars),
                        var_for(*b, &value_vars),
                        result_var,
                    ) {
                        constraints.push(TypeConstraint::Equal(rv.clone(), a_var));
                        constraints.push(TypeConstraint::Equal(rv, b_var));
                    }
                }
                Op::Rem(a, b) => {
                    if let (Some(a_var), Some(b_var), Some(rv)) = (
                        var_for(*a, &value_vars),
                        var_for(*b, &value_vars),
                        result_var,
                    ) {
                        constraints.push(TypeConstraint::Equal(rv.clone(), a_var));
                        constraints.push(TypeConstraint::Equal(rv, b_var));
                    }
                }
                Op::BitAnd(a, b) => {
                    if let (Some(a_var), Some(b_var), Some(rv)) = (
                        var_for(*a, &value_vars),
                        var_for(*b, &value_vars),
                        result_var,
                    ) {
                        constraints.push(TypeConstraint::Equal(rv.clone(), a_var));
                        constraints.push(TypeConstraint::Equal(rv, b_var));
                    }
                }
                Op::BitOr(a, b) => {
                    if let (Some(a_var), Some(b_var), Some(rv)) = (
                        var_for(*a, &value_vars),
                        var_for(*b, &value_vars),
                        result_var,
                    ) {
                        constraints.push(TypeConstraint::Equal(rv.clone(), a_var));
                        constraints.push(TypeConstraint::Equal(rv, b_var));
                    }
                }
                Op::BitXor(a, b) => {
                    if let (Some(a_var), Some(b_var), Some(rv)) = (
                        var_for(*a, &value_vars),
                        var_for(*b, &value_vars),
                        result_var,
                    ) {
                        constraints.push(TypeConstraint::Equal(rv.clone(), a_var));
                        constraints.push(TypeConstraint::Equal(rv, b_var));
                    }
                }
                Op::Shl(a, b) => {
                    if let (Some(a_var), Some(b_var), Some(rv)) = (
                        var_for(*a, &value_vars),
                        var_for(*b, &value_vars),
                        result_var,
                    ) {
                        constraints.push(TypeConstraint::Equal(rv.clone(), a_var));
                        constraints.push(TypeConstraint::Equal(rv, b_var));
                    }
                }
                Op::Shr(a, b) => {
                    if let (Some(a_var), Some(b_var), Some(rv)) = (
                        var_for(*a, &value_vars),
                        var_for(*b, &value_vars),
                        result_var,
                    ) {
                        constraints.push(TypeConstraint::Equal(rv.clone(), a_var));
                        constraints.push(TypeConstraint::Equal(rv, b_var));
                    }
                }
                // Interim: propagate operand type to result. Phase 9 replaces with builtin signatures.
                Op::Neg(a) => {
                    if let (Some(a_var), Some(rv)) = (var_for(*a, &value_vars), result_var) {
                        constraints.push(TypeConstraint::Equal(rv, a_var));
                    }
                }
                Op::BitNot(a) => {
                    if let (Some(a_var), Some(rv)) = (var_for(*a, &value_vars), result_var) {
                        constraints.push(TypeConstraint::Equal(rv, a_var));
                    }
                }

                // Select — result is compatible with both branches.
                Op::Select {
                    on_true, on_false, ..
                } => {
                    if let Some(rv) = result_var {
                        if let Some(t_var) = var_for(*on_true, &value_vars) {
                            constraints.push(TypeConstraint::Equal(rv.clone(), t_var));
                        }
                        if let Some(f_var) = var_for(*on_false, &value_vars) {
                            constraints.push(TypeConstraint::Equal(rv, f_var));
                        }
                    }
                }

                // GetField — emit HasField on the object type.
                Op::GetField { object, field } => {
                    if let (Some(obj_var), Some(rv)) = (var_for(*object, &value_vars), result_var) {
                        constraints.push(TypeConstraint::HasField {
                            ty: obj_var,
                            field: field.clone(),
                            field_ty: rv,
                        });
                    }
                }

                // SetField — emit HasField; the stored value type is the field type.
                Op::SetField {
                    object,
                    field,
                    value,
                } => {
                    if let (Some(obj_var), Some(val_var)) =
                        (var_for(*object, &value_vars), var_for(*value, &value_vars))
                    {
                        constraints.push(TypeConstraint::HasField {
                            ty: obj_var,
                            field: field.clone(),
                            field_ty: val_var,
                        });
                    }
                }

                // CallIndirect — callee is a value with a known type var.
                Op::CallIndirect { callee, args } => {
                    if let Some(callee_var) = var_for(*callee, &value_vars) {
                        let arg_types: Vec<Type> = args
                            .iter()
                            .filter_map(|a| var_for(*a, &value_vars))
                            .collect();
                        // Only emit if we resolved all args (partial coverage
                        // would produce an incorrect arity constraint).
                        if arg_types.len() == args.len() {
                            let ret = result_var.unwrap_or(Type::Void);
                            constraints.push(TypeConstraint::Callable {
                                ty: callee_var,
                                args: arg_types,
                                ret,
                            });
                        }
                    }
                }

                // SystemCall — emit GlobalStore / ResolveGlobalType constraints.
                //
                // GlobalStore: the written value's type must equal the global's type.
                // ResolveGlobalType: the result's type must equal the global's type.
                // Both use the global's shared TypeVarId from global_name_vars.
                Op::SystemCall {
                    system,
                    method,
                    args,
                } => {
                    let key = (system.as_str(), method.as_str());
                    if let Some(&(name_arg, value_arg)) = store_rules.get(&key) {
                        if let Some(name) = args
                            .get(name_arg)
                            .and_then(|&v| const_strings.get(&v).copied())
                        {
                            if let Some(&gvar) = global_name_vars.get(name) {
                                if let Some(val_var) =
                                    args.get(value_arg).and_then(|&v| var_for(v, &value_vars))
                                {
                                    constraints
                                        .push(TypeConstraint::Equal(val_var, Type::Var(gvar)));
                                }
                            }
                        }
                    } else if resolve_rules.contains(&key) {
                        if let (Some(name), Some(rv)) = (
                            args.first().and_then(|&v| const_strings.get(&v).copied()),
                            result_var,
                        ) {
                            if let Some(&gvar) = global_name_vars.get(name) {
                                constraints.push(TypeConstraint::Equal(rv, Type::Var(gvar)));
                            }
                        }
                    } else if field_rules.contains(&key) {
                        // ResolveInstanceField: emit HasField so the solver can propagate the
                        // field type once the receiver type is known (deferred if still Var).
                        // args[0] = receiver, args[1] = field name (const string).
                        if let (Some(recv_var), Some(rv)) = (
                            args.first().and_then(|&v| var_for(v, &value_vars)),
                            result_var,
                        ) {
                            if let Some(field) =
                                args.get(1).and_then(|&v| const_strings.get(&v).copied())
                            {
                                constraints.push(TypeConstraint::HasField {
                                    ty: recv_var,
                                    field: field.to_string(),
                                    field_ty: rv,
                                });
                            }
                        }
                    }
                }

                // Call — constrain result to callee's return type and args to
                // callee's param types (when those types are concrete).
                Op::Call {
                    func: callee_name,
                    args,
                } => {
                    if let Some(sig) = func_sigs.get(callee_name.as_str()) {
                        // Result ← callee return type.
                        if let Some(rv) = result_var.as_ref() {
                            if is_concrete(&sig.return_ty) {
                                constraints
                                    .push(TypeConstraint::Equal(rv.clone(), sig.return_ty.clone()));
                            }
                        }
                        // Args → callee param types (only concrete).
                        for (i, &arg) in args.iter().enumerate() {
                            if i < sig.params.len() && is_concrete(&sig.params[i]) {
                                if let Some(arg_var) = var_for(arg, &value_vars) {
                                    constraints.push(TypeConstraint::Equal(
                                        arg_var,
                                        sig.params[i].clone(),
                                    ));
                                }
                            }
                        }
                    }
                }

                // MethodCall — same as Call but params are offset by 1 (self).
                Op::MethodCall {
                    receiver,
                    method,
                    args,
                } => {
                    if let Some(sig) = func_sigs.get(method.as_str()) {
                        // Result ← callee return type.
                        if let Some(rv) = result_var.as_ref() {
                            if is_concrete(&sig.return_ty) {
                                constraints
                                    .push(TypeConstraint::Equal(rv.clone(), sig.return_ty.clone()));
                            }
                        }
                        // Receiver → callee param[0] (self).
                        if !sig.params.is_empty() && is_concrete(&sig.params[0]) {
                            if let Some(recv_var) = var_for(*receiver, &value_vars) {
                                constraints
                                    .push(TypeConstraint::Equal(recv_var, sig.params[0].clone()));
                            }
                        }
                        // Args → callee params[1..].
                        for (i, &arg) in args.iter().enumerate() {
                            let param_idx = i + 1;
                            if param_idx < sig.params.len() && is_concrete(&sig.params[param_idx]) {
                                if let Some(arg_var) = var_for(arg, &value_vars) {
                                    constraints.push(TypeConstraint::Equal(
                                        arg_var,
                                        sig.params[param_idx].clone(),
                                    ));
                                }
                            }
                        }
                    }
                }

                // Cast — result type is the cast target.
                Op::Cast(_, target_ty, _) => {
                    if let Some(rv) = result_var {
                        if is_concrete(target_ty) {
                            constraints.push(TypeConstraint::Equal(rv, target_ty.clone()));
                        }
                    }
                }

                // Comparison — result is Bool.
                Op::Cmp(_, _, _) => {
                    if let Some(rv) = result_var {
                        constraints.push(TypeConstraint::Equal(rv, Type::Bool));
                    }
                }

                // Boolean logic — operands and result are Bool.
                Op::Not(_) | Op::BoolAnd(_, _) | Op::BoolOr(_, _) => {
                    if let Some(rv) = result_var {
                        constraints.push(TypeConstraint::Equal(rv, Type::Bool));
                    }
                }

                // TypeCheck — result is Bool.
                Op::TypeCheck(_, _) => {
                    if let Some(rv) = result_var {
                        constraints.push(TypeConstraint::Equal(rv, Type::Bool));
                    }
                }

                // StructInit — result is Instance(id).
                Op::StructInit { name, .. } => {
                    if let Some(rv) = result_var {
                        if let Some(id) = module.find_type(name) {
                            constraints.push(TypeConstraint::Equal(rv, Type::Instance(id)));
                        }
                    }
                }

                // All other ops — no useful type constraint to emit at this
                // stage (allocs, stores, loads, etc.).
                _ => {}
            }
        }
    }

    // Constrain Return terminators.
    use crate::ir::inst::Terminator;
    for (_, block) in func.blocks.iter() {
        if let Terminator::Return(Some(value)) = &block.terminator {
            if let Some(val_var) = var_for(*value, &value_vars) {
                constraints.push(TypeConstraint::Equal(val_var, Type::Var(return_var)));
            }
        }
    }

    ConstraintSet {
        constraints,
        value_vars,
        return_var,
    }
}

/// Scan every block in `func` for branch ops that target `target_block` and
/// emit `Equal` constraints between each passed argument and the corresponding
/// block param.
fn emit_phi_constraints(
    target_block: BlockId,
    block: &crate::ir::block::Block,
    func: &Function,
    value_vars: &HashMap<ValueId, TypeVarId>,
    var_for: &impl Fn(ValueId, &HashMap<ValueId, TypeVarId>) -> Option<Type>,
    constraints: &mut Vec<TypeConstraint>,
) {
    use crate::ir::inst::Terminator;
    for (_, pred_block) in func.blocks.iter() {
        let edge_args_list: Vec<&Vec<ValueId>> = match &pred_block.terminator {
            Terminator::Br { target, args } if *target == target_block => vec![args],
            Terminator::BrIf {
                then_target,
                then_args,
                else_target,
                else_args,
                ..
            } => {
                let mut v = Vec::new();
                if *then_target == target_block {
                    v.push(then_args);
                }
                if *else_target == target_block {
                    v.push(else_args);
                }
                v
            }
            Terminator::Switch { cases, default, .. } => {
                let mut v = Vec::new();
                for (_, case_target, case_args) in cases {
                    if *case_target == target_block {
                        v.push(case_args);
                        break;
                    }
                }
                if v.is_empty() && default.0 == target_block {
                    v.push(&default.1);
                }
                v
            }
            _ => vec![],
        };

        for args in edge_args_list {
            for (param, &arg_val) in block.params.iter().zip(args.iter()) {
                if let (Some(param_var), Some(arg_var)) = (
                    var_for(param.value, value_vars),
                    var_for(arg_val, value_vars),
                ) {
                    constraints.push(TypeConstraint::Equal(param_var, arg_var));
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ConstraintCollect transform
// ---------------------------------------------------------------------------

/// Constraint collection pass.
///
/// Walks every function in the module and emits [`TypeConstraint`] values for
/// each instruction that carries type information.  Results accumulate in
/// [`ConstraintCollect::constraint_sets`].
///
/// This pass is a pure analysis — it does not modify the module.
pub struct ConstraintCollect {
    /// One [`ConstraintSet`] per function, in module function order.
    pub constraint_sets: std::cell::RefCell<Vec<ConstraintSet>>,
}

impl Default for ConstraintCollect {
    fn default() -> Self {
        Self::new()
    }
}

impl ConstraintCollect {
    pub fn new() -> Self {
        Self {
            constraint_sets: std::cell::RefCell::new(Vec::new()),
        }
    }
}

impl Transform for ConstraintCollect {
    fn name(&self) -> &str {
        "constraint-collect"
    }

    fn apply(&self, module: Module) -> Result<TransformResult, CoreError> {
        let mut sets: Vec<ConstraintSet> = Vec::with_capacity(module.functions.len());
        let any_functions = !module.functions.is_empty();
        let empty_globals: HashMap<String, TypeVarId> = HashMap::new();

        for (_, func) in module.functions.iter() {
            // Each function gets its own fresh arena (ConstraintCollect is a
            // pure analysis pass; it does not cross-function-solve globals).
            let mut arena = TypeVarArena::new();
            sets.push(collect_function(func, &module, &mut arena, &empty_globals));
        }

        *self.constraint_sets.borrow_mut() = sets;

        Ok(TransformResult {
            module,
            changed: any_functions,
        })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::func::Visibility;
    use crate::ir::ty::{FunctionSig, Type};

    /// Build a minimal function: `fn simple(x: f64) -> f64 { return x + 1.0; }`
    fn make_simple_module() -> Module {
        let sig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("simple", sig, Visibility::Public);

        let x = fb.param(0);
        let one = fb.const_float(1.0);
        let sum = fb.add(x, one);
        fb.ret(Some(sum));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        mb.build()
    }

    #[test]
    fn collect_simple_function_produces_constraints() {
        let module = make_simple_module();
        let func = module.functions.values().next().expect("no functions");
        let mut arena = TypeVarArena::new();
        let empty_globals = HashMap::new();
        let set = collect_function(func, &module, &mut arena, &empty_globals);

        // Every value in value_types should have a var.
        for (vid, _) in func.value_types.iter() {
            assert!(
                set.value_vars.contains_key(&vid),
                "missing var for value {:?}",
                vid
            );
        }

        // The Return op emits one Equal constraint: return_value_var == return_type_var.
        // Op::Add does NOT emit Equal constraints (string-concat overload); other
        // arithmetic ops do, but that is a separate concern tested elsewhere.
        assert!(
            !set.constraints.is_empty(),
            "expected at least one constraint from Return op"
        );
    }

    #[test]
    fn transform_stores_constraint_sets() {
        let module = make_simple_module();
        let pass = ConstraintCollect::new();
        let result = pass.apply(module).expect("apply failed");
        let sets = pass.constraint_sets.borrow();
        assert_eq!(sets.len(), 1, "expected 1 constraint set");
        assert!(result.changed, "expected changed=true");
    }

    #[test]
    fn empty_module_produces_no_sets() {
        let module = Module::new("empty".into());
        let pass = ConstraintCollect::new();
        let result = pass.apply(module).expect("apply failed");
        let sets = pass.constraint_sets.borrow();
        assert_eq!(sets.len(), 0);
        assert!(!result.changed, "expected changed=false for empty module");
    }
}
