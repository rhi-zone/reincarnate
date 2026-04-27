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
use crate::ir::func::FuncId;
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
    did_bind: bool,
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
            did_bind: false,
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
        self.did_bind = true;
    }

    /// Reset and return the `did_bind` flag.
    ///
    /// Returns `true` if any [`TypeVarId`] was bound via [`bind`] since the
    /// last call to this method (or since construction).  Resets the flag to
    /// `false` atomically so the next iteration starts clean.
    pub fn take_did_bind(&mut self) -> bool {
        std::mem::replace(&mut self.did_bind, false)
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
                param_lower_bounds: sig.param_lower_bounds,
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
                param_lower_bounds: sa.param_lower_bounds,
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
    /// - Concrete ground types (including [`Type::Unknown`]) get a fresh var
    ///   that is immediately bound to that type — prior knowledge the solver
    ///   can only confirm, never weaken.
    /// - [`Type::Var`] values get fresh, unbound vars (open inference targets).
    pub value_vars: HashMap<ValueId, TypeVarId>,
    /// TypeVarId allocated for the function's return type.
    ///
    /// Linked to return values via `Return` terminator constraints.  Exposed
    /// so that interprocedural call-site linking (Step 2b in ConstraintSolve2)
    /// can unify a caller's result type var with the callee's return type var,
    /// enabling bidirectional return-type propagation even when the callee's
    /// `sig.return_ty` is not yet concrete.
    pub return_var: TypeVarId,
    /// Lower bounds for entry param TypeVars — if a param var remains free
    /// after the fixpoint, bind it to this lower bound type.
    ///
    /// Populated from `func.sig.param_lower_bounds` in [`collect_function`].
    pub param_lower_bounds: Vec<(TypeVarId, Type)>,
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
// Type-rule constraint emission helper
// ---------------------------------------------------------------------------

/// Emit type constraints for a `SystemCallTypeRule` applied to a call site.
///
/// Used by both the `Op::SystemCall` and intrinsic `Op::Call` handlers so
/// the rule dispatch logic is not duplicated.
#[allow(clippy::too_many_arguments)]
fn emit_type_rule_constraints(
    rule: &SystemCallTypeRule,
    args: &[ValueId],
    result_var: Option<Type>,
    const_strings: &HashMap<ValueId, &str>,
    global_name_vars: &HashMap<String, TypeVarId>,
    value_vars: &HashMap<ValueId, TypeVarId>,
    constraints: &mut Vec<TypeConstraint>,
) {
    // Inline var_for: map a ValueId to its TypeVar as a Type::Var.
    let var_for = |v: ValueId| -> Option<Type> { value_vars.get(&v).copied().map(Type::Var) };
    match rule {
        SystemCallTypeRule::GlobalStore {
            name_arg,
            value_arg,
        } => {
            if let Some(name) = args
                .get(*name_arg)
                .and_then(|&v| const_strings.get(&v).copied())
            {
                if let Some(&gvar) = global_name_vars.get(name) {
                    if let Some(val_var) = args.get(*value_arg).and_then(|&v| var_for(v)) {
                        constraints.push(TypeConstraint::Equal(val_var, Type::Var(gvar)));
                    }
                }
            }
        }
        SystemCallTypeRule::ResolveGlobalType => {
            // args[0] = _rt (runtime handle), args[1] = global name (const string).
            if let (Some(name), Some(rv)) = (
                args.get(1).and_then(|&v| const_strings.get(&v).copied()),
                result_var,
            ) {
                if let Some(&gvar) = global_name_vars.get(name) {
                    constraints.push(TypeConstraint::Equal(rv, Type::Var(gvar)));
                }
            }
        }
        SystemCallTypeRule::ResolveInstanceField => {
            // args[0] = _rt (runtime handle), args[1] = receiver, args[2] = field name (const string).
            if let (Some(recv_var), Some(rv)) = (args.get(1).and_then(|&v| var_for(v)), result_var)
            {
                if let Some(field) = args.get(2).and_then(|&v| const_strings.get(&v).copied()) {
                    constraints.push(TypeConstraint::HasField {
                        ty: recv_var,
                        field: field.to_string(),
                        field_ty: rv,
                    });
                }
            }
        }
        // Other rules don't emit constraints in the collector.
        SystemCallTypeRule::ResolveGlobalTypeStructOnly { .. }
        | SystemCallTypeRule::ResolveClassName
        | SystemCallTypeRule::ConstructFromFirstArgType => {}
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
    //
    // Rule: pre-bind a value to its declared type only when the type is
    // concrete AND either:
    //   a) the type is not Unknown, OR
    //   b) the value is a function parameter (entry block param).
    //
    // In other words: Unknown is pre-bound ONLY for function parameters.
    // All other Unknown values are left as free TypeVars so constraints can
    // infer their actual types.
    //
    // Why function params get special treatment:
    //   Entry block params carry a declared parameter type.  Pre-binding them
    //   prevents interprocedural constraints from changing that declaration.
    //   An Unknown parameter type means "accepts any type at call sites" —
    //   that is a declared contract, not an inference gap.
    //
    // Why all other Unknown values are freed:
    //   - Alloc results: their cell TypeVar must be free to bind to the
    //     stored type via Store constraints.
    //   - Non-entry block params (phi values from mem2reg): their type must
    //     be inferred from phi-merge Equal constraints, not fixed at Unknown.
    //   - Phi feed values (args to non-entry branch targets): must be free so
    //     the phi-merge constraint can propagate the phi param's type back.
    //   - Other Unknown instruction results (e.g. results poisoned by an
    //     earlier HM pass): must be free so a later pass can infer them from
    //     callee return types, field types, etc.
    //   Pre-binding any of these to Unknown blocks the `(Unknown, _)` arm of
    //   unify from ever binding that TypeVar to a useful type.
    // -----------------------------------------------------------------------
    let entry_param_pos: HashMap<ValueId, usize> = func.blocks[func.entry]
        .params
        .iter()
        .enumerate()
        .map(|(i, p)| (p.value, i))
        .collect();

    for (vid, ty) in func.value_types.iter() {
        let var = arena.fresh();
        let param_pos = entry_param_pos.get(&vid).copied();
        let has_lower_bound = param_pos
            .and_then(|i| func.sig.param_lower_bounds.get(i))
            .and_then(|b| b.as_ref())
            .is_some();
        let should_bind = is_concrete(ty)
            && (!matches!(ty, Type::Unknown) || (param_pos.is_some() && !has_lower_bound));
        if should_bind {
            arena.bind(var, ty.clone());
        }
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
    // Build function name → FunctionSig map for MethodCall constraints.
    // MethodCall uses a string method name so a name-keyed map is still needed.
    // -----------------------------------------------------------------------
    let func_sigs: HashMap<&str, &FunctionSig> = module
        .functions
        .keys()
        .map(|fid| (module.func_name(fid), &module.functions[fid].sig))
        .collect();

    // Intrinsic Op::Call rule lookup: call FuncId → type rule.
    // Intrinsic functions registered via `register_runtime_intrinsic` carry a
    // `type_rule` on the Function rather than in system_call_type_rules.
    let intrinsic_type_rules: HashMap<FuncId, &SystemCallTypeRule> = module
        .runtime_registry
        .values()
        .filter_map(|&fid| {
            module.functions[fid]
                .type_rule
                .as_ref()
                .map(|rule| (fid, rule))
        })
        .collect();

    // Const-string map: ValueId → string literal value.
    // Only built when there are SystemCall or intrinsic-Call rules to process.
    let const_strings: HashMap<ValueId, &str> =
        if module.system_call_type_rules.is_empty() && intrinsic_type_rules.is_empty() {
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

    // Map from alloc result ValueId → cell TypeVar (shared across all loads/stores
    // through the same alloc). Allocs are always in the entry block (hoisted by
    // hoist_allocs) so the map is populated before any loads/stores are processed.
    let mut alloc_cell_vars: HashMap<ValueId, TypeVarId> = HashMap::new();

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

                // GetIndex — emit HasIndex on the collection type.
                Op::GetIndex { collection, index } => {
                    if let (Some(coll_var), Some(idx_var), Some(rv)) = (
                        var_for(*collection, &value_vars),
                        var_for(*index, &value_vars),
                        result_var,
                    ) {
                        constraints.push(TypeConstraint::HasIndex {
                            container: coll_var,
                            index_ty: idx_var,
                            elem_ty: rv,
                        });
                    }
                }

                // SetIndex — emit HasIndex; the stored value type is the element type.
                Op::SetIndex {
                    collection,
                    index,
                    value,
                } => {
                    if let (Some(coll_var), Some(idx_var), Some(val_var)) = (
                        var_for(*collection, &value_vars),
                        var_for(*index, &value_vars),
                        var_for(*value, &value_vars),
                    ) {
                        constraints.push(TypeConstraint::HasIndex {
                            container: coll_var,
                            index_ty: idx_var,
                            elem_ty: val_var,
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

                // SystemCall — emit GlobalStore / ResolveGlobalType / ResolveInstanceField
                // constraints.  The rule is looked up directly in system_call_type_rules.
                Op::SystemCall {
                    system,
                    method,
                    args,
                } => {
                    let key = (system.clone(), method.clone());
                    if let Some(rule) = module.system_call_type_rules.get(&key) {
                        emit_type_rule_constraints(
                            rule,
                            args,
                            result_var,
                            &const_strings,
                            global_name_vars,
                            &value_vars,
                            &mut constraints,
                        );
                    }
                }

                // Call — constrain result to callee's return type and args to
                // callee's param types (when those types are concrete).
                // Also apply GlobalStore / ResolveGlobalType / ResolveInstanceField rules
                // for registered intrinsic calls (Phase 3a: formerly Op::SystemCall).
                Op::Call {
                    func: callee_fid,
                    args,
                } => {
                    if let Some(callee) = module.functions.get(*callee_fid) {
                        let sig = &callee.sig;
                        // Result ← callee return type.
                        // Unknown return types are wildcards — they carry no type
                        // information and must not constrain the result, since
                        // Equal(rv, Unknown) would poison the result to Unknown
                        // and prevent downstream inference.
                        // The return-type constraint is always emitted regardless of
                        // any per-arg conflicts below.
                        if let Some(rv) = result_var.as_ref() {
                            if is_concrete(&sig.return_ty)
                                && !matches!(sig.return_ty, Type::Unknown)
                            {
                                constraints
                                    .push(TypeConstraint::Equal(rv.clone(), sig.return_ty.clone()));
                            }
                        }
                        // Args → callee param types.
                        //
                        // Per-arg type-conflict guard: if this arg has a concrete
                        // type that conflicts with the param type, skip only that
                        // arg's constraint (not the whole call).
                        //
                        // This prevents "wrong function selected" scenarios from
                        // poisoning the inference graph.  The canonical case: GML's
                        // `DataType::Variable` maps to `add_f64`, so a String
                        // accumulator gets `add_f64(str_var, ...)` in the IR.  If we
                        // emitted `Equal(str_var, Float64)`, the solver would
                        // force-rebind `str_var` to Unknown, propagating poison
                        // through every phi and store constraint that touches it.
                        //
                        // The conflict check fires only when:
                        //   - the arg's type is concrete and not Unknown/Var (i.e.
                        //     a resolved concrete type like String or Bool), AND
                        //   - the corresponding param type is also concrete and
                        //     non-Unknown, AND
                        //   - they differ.
                        // Unknown/Var arg types are not concrete mismatches — they
                        // are open inference targets that SHOULD be constrained.
                        //
                        // When param_ty is a Var, emit Equal(arg_var, param_ty) so
                        // that intra-procedural arg type info flows into the callee
                        // param Var (mirrors the inter-procedural fix in
                        // constraint_solve_hm.rs).
                        for (i, &arg) in args.iter().enumerate() {
                            if i < sig.params.len() {
                                let param_ty = &sig.params[i];
                                if let Type::Var(_) = param_ty {
                                    // Param is an open Var — flow arg type into it.
                                    if let Some(arg_var) = var_for(arg, &value_vars) {
                                        constraints
                                            .push(TypeConstraint::Equal(arg_var, param_ty.clone()));
                                    }
                                } else if is_concrete(param_ty)
                                    && !matches!(param_ty, Type::Unknown)
                                {
                                    // Param is a concrete type — check for conflict
                                    // before emitting.
                                    let arg_ty = &func.value_types[arg];
                                    let conflict = is_concrete(arg_ty)
                                        && !matches!(arg_ty, Type::Unknown | Type::Var(_))
                                        && arg_ty != param_ty;
                                    if !conflict {
                                        if let Some(arg_var) = var_for(arg, &value_vars) {
                                            constraints.push(TypeConstraint::Equal(
                                                arg_var,
                                                param_ty.clone(),
                                            ));
                                        }
                                    }
                                }
                                // Unknown params are wildcards — they accept any type
                                // and must not constrain the argument, since
                                // Equal(arg_var, Unknown) would poison the arg to
                                // Unknown and prevent downstream inference.
                            }
                        }
                    }
                    // Intrinsic type rules: same constraint logic as Op::SystemCall.
                    if let Some(rule) = intrinsic_type_rules.get(callee_fid) {
                        emit_type_rule_constraints(
                            rule,
                            args,
                            result_var,
                            &const_strings,
                            global_name_vars,
                            &value_vars,
                            &mut constraints,
                        );
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
                        // Unknown return types are wildcards — see Op::Call comment above.
                        if let Some(rv) = result_var.as_ref() {
                            if is_concrete(&sig.return_ty)
                                && !matches!(sig.return_ty, Type::Unknown)
                            {
                                constraints
                                    .push(TypeConstraint::Equal(rv.clone(), sig.return_ty.clone()));
                            }
                        }
                        // Receiver → callee param[0] (self).
                        if !sig.params.is_empty() {
                            let recv_ty = &sig.params[0];
                            if is_concrete(recv_ty) && !matches!(recv_ty, Type::Unknown) {
                                if let Some(recv_var) = var_for(*receiver, &value_vars) {
                                    constraints
                                        .push(TypeConstraint::Equal(recv_var, recv_ty.clone()));
                                }
                            }
                        }
                        // Args → callee params[1..].
                        // When param_ty is a Var, emit Equal(arg_var, param_ty) so
                        // that intra-procedural arg type info flows into the callee
                        // param Var (mirrors the fix in Op::Call above).
                        // Unknown params are wildcards — skip to avoid poisoning.
                        for (i, &arg) in args.iter().enumerate() {
                            let param_idx = i + 1;
                            if param_idx < sig.params.len() {
                                let param_ty = &sig.params[param_idx];
                                if let Type::Var(_) = param_ty {
                                    if let Some(arg_var) = var_for(arg, &value_vars) {
                                        constraints
                                            .push(TypeConstraint::Equal(arg_var, param_ty.clone()));
                                    }
                                } else if is_concrete(param_ty)
                                    && !matches!(param_ty, Type::Unknown)
                                {
                                    if let Some(arg_var) = var_for(arg, &value_vars) {
                                        constraints
                                            .push(TypeConstraint::Equal(arg_var, param_ty.clone()));
                                    }
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

                // Alloc — record the cell TypeVar for this allocation slot.
                //
                // We reuse the arena TypeVar already allocated for the alloc
                // result in Phase 1 (rather than creating a fresh one) so that
                // writeback propagates the inferred cell type back to the
                // alloc result's value_types entry.  This is what gives
                // `let str: string` instead of `let str: unknown` in the
                // emitted output.
                //
                // The inner_ty carried on the Alloc op uses frontend-local
                // TypeVarIds that have no identity in the shared arena — they
                // must never be used directly.  Seeding the cell via an
                // Equal constraint (not direct reuse) avoids that aliasing.
                Op::Alloc(inner_ty) => {
                    if let Some(alloc_result) = inst.result {
                        let cell_var = *value_vars.get(&alloc_result).unwrap();
                        if is_concrete(inner_ty) && !matches!(inner_ty, Type::Unknown) {
                            constraints
                                .push(TypeConstraint::Equal(Type::Var(cell_var), inner_ty.clone()));
                        }
                        alloc_cell_vars.insert(alloc_result, cell_var);
                    }
                }

                // Store — unify the cell type with the stored value's type.
                Op::Store { ptr, value } => {
                    if let Some(&cell_var) = alloc_cell_vars.get(ptr) {
                        if let Some(val_var) = var_for(*value, &value_vars) {
                            constraints.push(TypeConstraint::Equal(Type::Var(cell_var), val_var));
                        }
                    }
                }

                // Load — unify the result type with the cell type.
                Op::Load(ptr) => {
                    if let (Some(rv), Some(&cell_var)) = (result_var, alloc_cell_vars.get(ptr)) {
                        constraints.push(TypeConstraint::Equal(rv, Type::Var(cell_var)));
                    }
                }

                // All other ops — no useful type constraint to emit at this stage.
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

    // Collect param lower bounds from the function signature.
    let mut param_lower_bounds: Vec<(TypeVarId, Type)> = Vec::new();
    for (i, lower_bound) in func.sig.param_lower_bounds.iter().enumerate() {
        if let Some(lb) = lower_bound {
            if let Some(param) = func.blocks[func.entry].params.get(i) {
                if let Some(&var) = value_vars.get(&param.value) {
                    param_lower_bounds.push((var, lb.clone()));
                }
            }
        }
    }

    ConstraintSet {
        constraints,
        value_vars,
        return_var,
        param_lower_bounds,
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

    fn apply(
        &self,
        module: Module,
        _dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        let mut sets: Vec<ConstraintSet> = Vec::with_capacity(module.functions.len());
        let empty_globals: HashMap<String, TypeVarId> = HashMap::new();

        // Build a set of runtime-registry FuncIds so we can skip builtin stubs.
        // Builtin stubs have empty bodies and no meaningful constraints to collect.
        let runtime_func_ids: std::collections::HashSet<FuncId> =
            module.runtime_registry.values().copied().collect();

        for (fid, func) in module.functions.iter() {
            if runtime_func_ids.contains(&fid) {
                continue;
            }
            // Each function gets its own fresh arena (ConstraintCollect is a
            // pure analysis pass; it does not cross-function-solve globals).
            let mut arena = TypeVarArena::new();
            sets.push(collect_function(func, &module, &mut arena, &empty_globals));
        }

        *self.constraint_sets.borrow_mut() = sets;

        Ok(TransformResult {
            // Pure analysis pass — no IR mutation. changed=false so fixpoint
            // mode is driven solely by constraint_solve_hm's write-backs.
            module,
            changed: false,
            changed_funcs: HashSet::new(),
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
    /// Returns the module and the FuncId of the added function.
    fn make_simple_module() -> (Module, crate::ir::func::FuncId) {
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
        let fid = mb.add_function(fb.build());
        (mb.build(), fid)
    }

    #[test]
    fn collect_simple_function_produces_constraints() {
        let (module, fid) = make_simple_module();
        let func = &module.functions[fid];
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
        let (module, _) = make_simple_module();
        let pass = ConstraintCollect::new();
        let result = pass.apply(module, None).expect("apply failed");
        let sets = pass.constraint_sets.borrow();
        assert_eq!(sets.len(), 1, "expected 1 constraint set");
        assert!(
            !result.changed,
            "pure analysis pass must not report changed"
        );
    }

    #[test]
    fn empty_module_produces_no_sets() {
        let module = Module::new("empty".into());
        let pass = ConstraintCollect::new();
        let result = pass.apply(module, None).expect("apply failed");
        let sets = pass.constraint_sets.borrow();
        assert_eq!(sets.len(), 0);
        assert!(!result.changed, "expected changed=false for empty module");
    }

    /// Type::Unknown is concrete — inference exhausted, not a free variable.
    /// Type::Var is the pre-inference placeholder and is NOT concrete.
    /// This invariant must never regress: code that treats Unknown as solvable
    /// or Var as concrete will produce wrong types throughout the pipeline.
    #[test]
    fn unknown_is_concrete_var_is_not() {
        use crate::ir::ty::TypeVarId;
        assert!(
            is_concrete(&Type::Unknown),
            "Type::Unknown must be concrete"
        );
        assert!(
            !is_concrete(&Type::Var(TypeVarId::new(0))),
            "Type::Var must not be concrete"
        );
        assert!(
            !is_concrete(&Type::Array(Box::new(Type::Var(TypeVarId::new(0))))),
            "Array(Var) must not be concrete"
        );
        assert!(
            is_concrete(&Type::Array(Box::new(Type::Unknown))),
            "Array(Unknown) must be concrete"
        );
    }
}
