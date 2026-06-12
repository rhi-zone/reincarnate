//! HM-style constraint solver pass (`ConstraintSolveHM`).
//!
//! This pass replaces the old `TypeInference`, `CallSiteTypeFlow`,
//! `CallSiteTypeWiden`, `ConstraintSolve`, and `ConstraintSolve2` passes with
//! a single engine-agnostic Hindley-Milner constraint solver.
//!
//! # Architecture
//!
//! 1. **Global allocation**: allocate one [`TypeVarId`] per global variable
//!    name in a single shared [`TypeVarArena`], binding concrete globals
//!    immediately.
//! 2. **Constraint collection**: call [`collect_function`] for every function,
//!    passing the shared arena and global-name → TypeVar map.  Each function's
//!    value vars are allocated into the same arena, enabling cross-function
//!    constraints.
//! 3. **Interprocedural linking**: emit [`TypeConstraint::Equal`] constraints
//!    between caller argument vars and callee parameter vars, and between
//!    caller result vars and callee return vars.
//! 4. **Fixpoint solving**: process constraints in a worklist loop, deferring
//!    `HasField` and `Callable` constraints until the object/callee type is
//!    resolved.
//! 5. **Write-back**: resolve every value's TypeVar and write the result back
//!    into `func.value_types`. Unresolved vars are left unchanged in the IR;
//!    only the emit step converts residual `Type::InferVar` to `unknown`.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};

use crate::entity::EntityRef;
use crate::error::CoreError;
use crate::ir::inst::Op;
use crate::ir::module::{SystemCallTypeRule, TypeDecl};
use crate::ir::ty::{TypeConstraint, TypeId, TypeVarId};
use crate::ir::{Constant, FuncId, Module, Type, ValueId};
use crate::pipeline::{Transform, TransformResult};
use crate::transforms::constraint_collect::{
    collect_function, is_concrete, resolve, unify, TypeVarArena,
};

/// Build the own-fields map (struct name → field name → field type) from a module.
///
/// "Own fields" means only the fields declared directly on each struct, not
/// inherited from parent types.  Used for struct narrowing discriminants.
/// Build own-fields from `module.types` (the live graph).
///
/// "Own fields" means only the fields declared directly on each struct, not
/// inherited from parent types.  Used for struct narrowing discriminants.
///
/// Used to build `all_fields` for HasField **resolution**: when the struct type is already
/// known (e.g. `HasField(Instance(Gun), "bulletDamageMultiplier", X)`), use the full
/// TypeDecl field set so that pass-inferred fields (from ConstructorStructInfer) are visible.
fn build_own_fields(module: &Module) -> HashMap<String, HashMap<String, Type>> {
    let mut map: HashMap<String, HashMap<String, Type>> = HashMap::new();
    for (_id, decl) in module.types.iter() {
        if let TypeDecl::Object {
            name: Some(name),
            fields,
            ..
        } = decl
        {
            if fields.is_empty() {
                continue;
            }
            map.insert(
                name.clone(),
                fields
                    .iter()
                    .map(|f| (f.name.clone(), f.ty.clone()))
                    .collect(),
            );
        }
    }
    map
}

/// Build the all-fields map (struct name → field name → field type), merging
/// own fields with all inherited ancestor fields.
///
/// Used for `HasField` resolution — a field access on `Player` resolves via
/// `GMLObject` when `Player` declares `super_class = "GMLObject"`.
/// Own fields take priority over inherited fields (shadowing).
fn build_all_fields(
    module: &Module,
    own_fields: &HashMap<String, HashMap<String, Type>>,
    type_id_to_name: &HashMap<TypeId, String>,
) -> HashMap<String, HashMap<String, Type>> {
    let mut all_fields = own_fields.clone();

    // Walk the TypeDecl parent chain for every named Object type and merge
    // ancestor own-fields into the all_fields entry.
    let type_names: Vec<String> = module
        .types
        .values()
        .filter_map(|td| {
            if let TypeDecl::Object {
                name: Some(name), ..
            } = td
            {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();

    for type_name in type_names {
        // Walk the TypeDecl parent chain and merge ancestor own-fields.
        // Own fields (already in the map) take priority — use entry().or_insert.
        let entry = all_fields.entry(type_name.clone()).or_default();
        let mut current_name: Option<String> = Some(type_name.clone());
        loop {
            // Find the TypeId for the current name.
            let Some(name) = current_name else { break };
            let Some(&type_id) = module.find_type(&name).as_ref() else {
                break;
            };
            // Look up its parent TypeId.
            let parent_id = match module.types.get(type_id) {
                Some(TypeDecl::Object { parent, .. }) => *parent,
                _ => None,
            };
            let Some(pid) = parent_id else { break };
            // Resolve parent name.
            let Some(parent_name) = type_id_to_name.get(&pid) else {
                break;
            };
            // Merge parent's own fields (do not overwrite child's own fields).
            if let Some(parent_own) = own_fields.get(parent_name) {
                for (fname, fty) in parent_own {
                    entry.entry(fname.clone()).or_insert_with(|| fty.clone());
                }
            }
            current_name = Some(parent_name.clone());
        }
    }

    all_fields
}

/// The ancestor chain of an instance type, from the type itself up to its root.
///
/// Walks `TypeDecl::Object.parent` over `module.types` — `TypeId` + `parent`
/// only, no engine names. The returned vec begins with `id` and ends at the
/// root ancestor (the first type with no parent). A bound on iteration guards
/// against malformed cyclic parent chains.
fn ancestor_chain(module: &Module, id: TypeId) -> Vec<TypeId> {
    let mut chain = Vec::new();
    let mut current = Some(id);
    let mut guard = 0usize;
    while let Some(cur) = current {
        if chain.contains(&cur) {
            break; // cycle guard
        }
        chain.push(cur);
        guard += 1;
        if guard > module.types.len() + 1 {
            break;
        }
        current = match module.types.get(cur) {
            Some(TypeDecl::Object { parent, .. }) => *parent,
            _ => None,
        };
    }
    chain
}

/// Least-upper-bound (join) of two source-language types.
///
/// This computes a real supertype outside the arena (the arena stays pure
/// equality). It is engine-neutral: the only structural knowledge it uses is
/// the `TypeDecl::Object.parent` chain over `module.types`.
///
/// - Identical operands → that type.
/// - Two `Instance(_)` → their least-common-ancestor by walking the parent
///   chains. If they share no ancestor, fall through to the lower-bound fallback.
/// - Anything else (mixed instance/non-instance, incompatible primitives) →
///   the param's declared lower bound if present, else `Type::Value`.
///
/// Never returns `Type::Union` — a union immediately collapses to `Value` in
/// the unifier, which is exactly the behavior this join replaces.
fn join(module: &Module, lower_bound: Option<&Type>, a: &Type, b: &Type) -> Type {
    if a == b {
        return a.clone();
    }
    if let (Type::Instance(ia), Type::Instance(ib)) = (a, b) {
        let chain_a = ancestor_chain(module, *ia);
        let chain_b: BTreeSet<TypeId> = ancestor_chain(module, *ib).into_iter().collect();
        // First ancestor of `a` (walking up from `a`) that is also an ancestor
        // of `b` is their least-common-ancestor.
        if let Some(&lca) = chain_a.iter().find(|id| chain_b.contains(id)) {
            return Type::Instance(lca);
        }
    }
    // No common ancestor, or non-instance operands: fall back to the declared
    // lower bound (a true supertype set by the frontend), else `Value`.
    lower_bound.cloned().unwrap_or(Type::Value)
}

/// Left-fold [`join`] over a non-empty list of types. The caller guarantees the
/// list is non-empty (the single-element case is handled before reaching here).
fn join_all(module: &Module, lower_bound: Option<&Type>, types: &[Type]) -> Type {
    let mut iter = types.iter();
    let first = iter.next().expect("join_all called on empty list").clone();
    iter.fold(first, |acc, t| join(module, lower_bound, &acc, t))
}

/// HM-unifier–based constraint solver pass.
///
/// Replaces `TypeInference`, `CallSiteTypeFlow`, `CallSiteTypeWiden`,
/// `ConstraintSolve`, and `ConstraintSolve2` with a single engine-agnostic
/// pass.  See module-level documentation for the full algorithm.
pub struct ConstraintSolveHM;

/// Process a single [`TypeConstraint`], potentially emitting deferred
/// secondary constraints (from `HasField` / `Callable` resolution).
///
/// `own_fields`: fields declared directly on each struct — used for struct
/// narrowing discriminants (inherited fields are not discriminants).
///
/// `all_fields`: own + all ancestor fields — used for `HasField` resolution
/// so that `Player.x` resolves via the `GMLObject` parent.
///
/// `non_leaf_type_names`: types that appear as a parent of some other type.
/// Excluded from narrowing targets — narrowing to a non-leaf base type erases
/// specificity (seeing `obj.x` means `obj: some subtype of GMLObject`, not
/// `obj: GMLObject`).
#[allow(clippy::too_many_arguments)]
fn process_constraint(
    c: TypeConstraint,
    arena: &mut TypeVarArena,
    own_fields: &HashMap<String, HashMap<String, Type>>,
    all_fields: &HashMap<String, HashMap<String, Type>>,
    type_id_to_name: &HashMap<TypeId, String>,
    name_to_type_id: &HashMap<String, TypeId>,
    non_leaf_type_names: &HashSet<String>,
    join_param_vars: &HashSet<TypeVarId>,
    deferred: &mut Vec<TypeConstraint>,
) {
    match c {
        TypeConstraint::Equal(a, b) => {
            let _ = unify(a.clone(), b.clone(), arena);
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
                        // Use all_fields: resolves fields inherited from parent types.
                        if let Some(fields) = all_fields.get(name) {
                            if let Some(ft) = fields.get(&field) {
                                deferred.push(TypeConstraint::Equal(field_ty, ft.clone()));
                            }
                        }
                    }
                    // Unknown field — skip; don't invent a type.
                }
                Type::InferVar(var_id) => {
                    // Join precedence: if this var is a param with COMPLETE call-site
                    // evidence, its type is decided by the SOUND post-fixpoint join of
                    // its actual callers — not by the HasField single-owner heuristic.
                    // The single-owner guess (binding a free receiver to the unique leaf
                    // type that declares the accessed field) is a fallback for values
                    // WITHOUT caller evidence; firing it here would pre-bind the param to
                    // one caller's leaf type, which is unsound when the caller set spans
                    // siblings. Re-defer so the join binds the var first; once bound, the
                    // HasField re-resolves against the joined instance type.
                    if join_param_vars.contains(&var_id) {
                        deferred.push(TypeConstraint::HasField {
                            ty: resolved_ty,
                            field,
                            field_ty,
                        });
                        return;
                    }
                    // Part 1: if exactly one struct has this field in its own fields,
                    // unify immediately.  Use own_fields — inherited fields are not
                    // discriminants; every child would match and produce multiple candidates.
                    // Only consider leaf types: non-leaf types (those that have subtypes) are
                    // never valid narrowing targets — seeing obj.x means obj is *some subtype*
                    // of GMLObject, not GMLObject itself.
                    let candidates: Vec<TypeId> = name_to_type_id
                        .iter()
                        .filter(|(name, _)| !non_leaf_type_names.contains(*name))
                        .filter(|(name, _)| {
                            own_fields
                                .get(*name)
                                .is_some_and(|f| f.contains_key(&field))
                        })
                        .map(|(_, &id)| id)
                        .collect();
                    let field_in_non_leaf = non_leaf_type_names
                        .iter()
                        .any(|name| own_fields.get(name).is_some_and(|f| f.contains_key(&field)));
                    if candidates.len() == 1 && !field_in_non_leaf {
                        let type_id = candidates[0];
                        let _ = unify(resolved_ty, Type::Instance(type_id), arena);
                        if let Some(name) = type_id_to_name.get(&type_id) {
                            // Use all_fields for the field-type constraint once type is known.
                            if let Some(ft) = all_fields.get(name).and_then(|f| f.get(&field)) {
                                deferred.push(TypeConstraint::Equal(field_ty, ft.clone()));
                            }
                        }
                    } else {
                        // Object type not yet resolved — re-defer.
                        deferred.push(TypeConstraint::HasField {
                            ty: resolved_ty,
                            field,
                            field_ty,
                        });
                    }
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
            } else if matches!(resolved_ty, Type::InferVar(_)) {
                // Callee type not yet resolved — re-defer.
                deferred.push(TypeConstraint::Callable {
                    ty: resolved_ty,
                    args,
                    ret,
                });
            }
            // Other — no useful info.
        }
        TypeConstraint::HasIndex {
            container,
            index_ty,
            elem_ty,
        } => {
            let resolved = resolve(container, arena);
            match resolved {
                Type::Array(elem_type_box) => {
                    deferred.push(TypeConstraint::Equal(elem_ty, *elem_type_box));
                    // index_ty is left unconstrained here; the actual index value's
                    // type is already known from its own instruction (no engine-specific
                    // Int width should be assumed in core).
                    let _ = index_ty;
                }
                Type::InferVar(_) => {
                    // Container type not yet resolved — re-defer.
                    deferred.push(TypeConstraint::HasIndex {
                        container: resolved,
                        index_ty,
                        elem_ty,
                    });
                }
                _ => {
                    // Unknown or other concrete type — no useful info.
                }
            }
        }
    }
}

/// Check whether a callee parameter value is used as a collection (array or map)
/// in the callee's body.  Used to suppress interprocedural narrowing that would
/// incorrectly convert array/map params to scalar types.
fn param_used_as_collection(
    func: &crate::ir::Function,
    param_val: ValueId,
    array_like_fids: &std::collections::HashSet<FuncId>,
) -> bool {
    for block in func.blocks.values() {
        for &inst_id in &block.insts {
            let inst = &func.insts[inst_id];
            match &inst.op {
                Op::GetIndex { collection, .. } if *collection == param_val => {
                    return true;
                }
                Op::SetIndex { collection, .. } if *collection == param_val => {
                    return true;
                }
                Op::GetField { object, field } if *object == param_val && field == "length" => {
                    return true;
                }
                Op::Call {
                    func: callee_fid,
                    args,
                } if args.contains(&param_val) => {
                    if array_like_fids.contains(callee_fid) {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

/// Returns true only when `ty` is a known primitive scalar that can never be a struct instance.
/// Returns false for `Type::InferVar`, `Type::Value`, and any compound or reference type so that
/// unresolved type variables are not incorrectly excluded from inter-procedural constraints.
fn is_definitely_scalar(ty: &crate::ir::Type) -> bool {
    matches!(
        ty,
        crate::ir::Type::Int(_)
            | crate::ir::Type::UInt(_)
            | crate::ir::Type::Float(_)
            | crate::ir::Type::Bool
            | crate::ir::Type::String
            | crate::ir::Type::Void
    )
}

/// Check whether a callee parameter value is used with field access in the callee's body.
fn param_used_with_field_access(
    func: &crate::ir::Function,
    param_val: ValueId,
    array_like_fids: &std::collections::HashSet<FuncId>,
) -> bool {
    for block in func.blocks.values() {
        for &inst_id in &block.insts {
            let inst = &func.insts[inst_id];
            match &inst.op {
                Op::GetField { object, .. } if *object == param_val => {
                    return true;
                }
                Op::SetField { object, .. } if *object == param_val => {
                    return true;
                }
                Op::GetIndex { collection, .. } if *collection == param_val => {
                    return true;
                }
                Op::SetIndex { collection, .. } if *collection == param_val => {
                    return true;
                }
                _ => {}
            }
        }
    }
    // Also check collection usage (field access implies collection usage is fine to suppress).
    param_used_as_collection(func, param_val, array_like_fids)
}

/// Per-parameter inference evidence accumulated across call sites.
///
/// `call_site` holds concrete argument types observed at call sites; `default`
/// holds the type of the param's default argument (if any). `incomplete` is set
/// only when a caller is genuinely *un-enumerable* — a dynamic value or an opaque
/// dispatch — so the remaining evidence can never become a complete enumeration.
/// The drain may join a param from its evidence ONLY when `incomplete == false`.
///
/// Completeness distinguishes two kinds of non-concrete caller:
///  - a genuinely-dynamic (`Type::Value`) argument, a body-usage abstention,
///    or an opaque/address-taken-indirect callee → truly un-enumerable, sets
///    `incomplete` (see `seed_param_from_arg` and `mark_func_params_incomplete`);
///  - a **linked, not-yet-resolved param-var caller** — the arg is a free
///    `InferVar` because the caller is itself a param whose own type is not yet
///    resolved at seeding (intra-run ordering). This caller IS enumerable: it
///    will resolve, via its own evidence, to a concrete type. It is recorded as
///    a directional lower bound (`lower_bound_vars`) — NOT unified, NOT
///    marked incomplete. Post-fixpoint the join resolves each such var through
///    the arena and folds it into the param's lower-bound set.
///
/// The join over the COMPLETE lower-bound set (concrete `call_site` types PLUS
/// resolved `lower_bound_vars`) is a supertype of every caller, so every caller
/// value remains assignable to the param — narrowing to the join is sound.
#[derive(Default)]
struct ParamEvidence {
    call_site: Vec<Type>,
    default: Option<Type>,
    incomplete: bool,
    /// Linked, not-yet-resolved param-var callers (free `InferVar` args). Each
    /// is resolved through the arena post-fixpoint and folded into the join's
    /// lower-bound set. A directional lower-bound record (a `Subtype`-style edge
    /// from the caller's var to this param), never unified into equality.
    lower_bound_vars: Vec<TypeVarId>,
}

/// Seed a callee param's inference evidence from one caller argument.
///
/// Shared body of the Call / MethodCall / MakeClosure arg-seeding loops:
///  - already-concrete params need no narrowing (return),
///  - a `Value` argument marks the evidence incomplete (the caller is dropped),
///  - a body-usage abstention (`usage_suppressed`) marks the evidence incomplete
///    (the caller declines to contribute, so it is dropped from `call_site`),
///  - a concrete argument is recorded as call-site evidence;
///  - a non-concrete argument WITH a caller var (`arg_var`) is a linked,
///    not-yet-resolved param-var caller: it is recorded as a directional lower
///    bound (`lower_bound_vars`) — enumerable, NOT incomplete, NOT unified;
///  - a non-concrete argument WITHOUT a caller var cannot be tracked at all, so
///    it is genuinely un-enumerable and marks the evidence incomplete.
///
/// A caller is dropped — marking the evidence incomplete — only when it is
/// genuinely un-enumerable (`Value`, usage abstention, no caller var). A linked
/// param-var caller is enumerable and is folded into the post-fixpoint join.
#[allow(clippy::too_many_arguments)]
fn seed_param_from_arg(
    param_concrete_types: &mut BTreeMap<TypeVarId, ParamEvidence>,
    arg_ty: &Type,
    param_ty: &Type,
    arg_var: Option<TypeVarId>,
    param_var: Option<TypeVarId>,
    usage_suppressed: bool,
) {
    // Already-concrete callee params are resolved; no narrowing needed.
    if is_concrete(param_ty) {
        return;
    }
    let Some(param_var) = param_var else {
        return;
    };
    // A genuinely-dynamic (`Value`) argument means this caller cannot be recorded
    // as a concrete type: it is dropped from `call_site`, so the evidence no longer
    // enumerates every caller. Mark it incomplete so the drain leaves the param free.
    if matches!(arg_ty, Type::Value) {
        param_concrete_types
            .entry(param_var)
            .or_default()
            .incomplete = true;
        return;
    }
    // A body-usage abstention drops this caller from `call_site` without recording
    // it. The remaining evidence is no longer a complete caller enumeration.
    if usage_suppressed {
        param_concrete_types
            .entry(param_var)
            .or_default()
            .incomplete = true;
        return;
    }
    if is_concrete(arg_ty) {
        param_concrete_types
            .entry(param_var)
            .or_default()
            .call_site
            .push(arg_ty.clone());
    } else if let Some(arg_var) = arg_var {
        // A non-concrete argument with a caller var is a linked, not-yet-resolved
        // param-var caller: it WILL resolve via its own evidence. Record it as a
        // directional lower bound (resolved through the arena post-fixpoint and
        // folded into the join). It is enumerable, so it does NOT mark the
        // evidence incomplete, and it is NOT unified into equality — the join is a
        // supertype relation, not an equality.
        param_concrete_types
            .entry(param_var)
            .or_default()
            .lower_bound_vars
            .push(arg_var);
    } else {
        // A non-concrete argument with no caller var cannot be tracked or resolved
        // — it is genuinely un-enumerable. Mark the evidence incomplete so
        // `call_site` alone never narrows the param.
        param_concrete_types
            .entry(param_var)
            .or_default()
            .incomplete = true;
    }
}

/// Result of address-taken / indirect-escape analysis over the whole module.
///
/// Soundness of call-site param narrowing requires a *complete* enumeration of
/// every caller of a function. Direct calls (`Op::Call`/`Op::MethodCall`) are
/// enumerated by the seeding loop. Indirect dispatch (`Op::CallIndirect`) is not
/// expressed as a static caller→callee edge, so we must bound which functions it
/// can reach.
///
/// A FuncId can only be the target of an indirect call if it first becomes a
/// *value* somewhere. In this IR a function reference is a value only via a
/// name-carrying op — `Constant` has no function variant and there is no
/// first-class `FuncId` value. The complete set of FuncId-escape routes is:
///  - `Op::MakeClosure { func }` — closure function reference,
///  - `Op::CoroutineCreate { func }` — coroutine from a function reference,
///  - `Op::GlobalRef(name)` where `name` resolves to a module function
///    (the GMS2.3 `pushref`→SCPT route, lowered to `GlobalRef(script_name)`).
///
/// Storing a FuncId to a field/global/var, passing it as an argument, or
/// returning it all first turn it into one of the above as a value, then flow
/// that value — so the three routes are exhaustive for this IR.
///
/// A function NOT in `address_taken` can only be reached by direct calls; its
/// caller set is fully enumerated and it narrows soundly with no change. A
/// function in `address_taken` may be the target of `opaque_indirect` if any
/// `Op::CallIndirect` has a callee value that cannot be traced to a concrete
/// function reference (a genuinely-dynamic dispatch such as `live_call`, or a
/// callee flowing from an opaque source). When `opaque_indirect` is set, every
/// address-taken function has an un-enumerable caller and must be marked
/// incomplete so neither single-caller nor LCA narrowing fires on partial
/// evidence.
struct AddressTakenAnalysis {
    /// FuncIds whose reference escapes as a value (see routes above).
    address_taken: HashSet<FuncId>,
    /// True if any `Op::CallIndirect` callee cannot be resolved to a concrete
    /// function reference — such a call could target any address-taken function.
    opaque_indirect: bool,
}

/// Trace a `CallIndirect` callee `ValueId` back to a concrete function name, if
/// it is a constant function reference produced by `Op::GlobalRef` in the same
/// function. Returns `None` for any callee that does not resolve to a single
/// statically-known function reference (the opaque / genuinely-dynamic case).
fn resolve_indirect_callee_name<'a>(
    callee: ValueId,
    result_to_op: &HashMap<ValueId, &'a Op>,
) -> Option<&'a str> {
    match result_to_op.get(&callee)? {
        Op::GlobalRef(name) => Some(name.as_str()),
        _ => None,
    }
}

/// Compute the address-taken / indirect-escape analysis for the whole module.
///
/// `name_to_fid` resolves a function name to its `FuncId`; only names that are
/// real module functions count as FuncId escapes (a `GlobalRef` to a sprite or
/// room is not a function reference).
fn compute_address_taken(
    module: &Module,
    name_to_fid: &HashMap<&str, FuncId>,
) -> AddressTakenAnalysis {
    let mut address_taken: HashSet<FuncId> = HashSet::new();
    let mut opaque_indirect = false;

    for (_, func) in module.functions.iter() {
        // result ValueId → defining Op, for callee tracing within this function.
        let result_to_op: HashMap<ValueId, &Op> = func
            .insts
            .values()
            .filter_map(|inst| inst.result.map(|r| (r, &inst.op)))
            .collect();

        for block in func.blocks.values() {
            for &inst_id in &block.insts {
                match &func.insts[inst_id].op {
                    Op::MakeClosure { func: name, .. } | Op::CoroutineCreate { func: name, .. } => {
                        if let Some(&fid) = name_to_fid.get(name.as_str()) {
                            address_taken.insert(fid);
                        }
                    }
                    Op::GlobalRef(name) => {
                        if let Some(&fid) = name_to_fid.get(name.as_str()) {
                            address_taken.insert(fid);
                        }
                    }
                    Op::CallIndirect { callee, .. } => {
                        if resolve_indirect_callee_name(*callee, &result_to_op).is_none() {
                            // Callee is not a statically-known function reference:
                            // this dispatch could target any address-taken function.
                            opaque_indirect = true;
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    AddressTakenAnalysis {
        address_taken,
        opaque_indirect,
    }
}

impl Transform for ConstraintSolveHM {
    fn name(&self) -> &str {
        "constraint-solve-hm"
    }

    fn apply(
        &self,
        mut module: Module,
        dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        // own_fields: enriched from module.types (live graph).
        // - For narrowing discriminants: constructor-only scanning means non-leaf types
        //   don't gain spurious fields that would over-trigger field_in_non_leaf.
        // - For all_fields resolution: includes constructor-inferred fields on leaf structs
        //   (e.g. Gun.bulletDamageMultiplier) that are not in module.structs.
        let own_fields = build_own_fields(&module);
        let type_id_to_name: HashMap<TypeId, String> = module
            .types
            .iter()
            .filter_map(|(id, td)| td.name().map(|n| (id, n.to_string())))
            .collect();
        let name_to_type_id: HashMap<String, TypeId> = type_id_to_name
            .iter()
            .map(|(&id, name)| (name.clone(), id))
            .collect();
        // all_fields: own fields + fields inherited from parent types via TypeDecl parent chain.
        // Used for HasField resolution — a field access on Player resolves via GMLObject.
        let all_fields = build_all_fields(&module, &own_fields, &type_id_to_name);

        // Non-leaf types: types that appear as a `parent` of any other type.
        // These should not be used as narrowing targets — seeing obj.x doesn't mean
        // obj IS GMLObject, only that it is some GMLObject subtype.
        let non_leaf_type_names: HashSet<String> = module
            .types
            .values()
            .filter_map(|decl| {
                if let TypeDecl::Object { parent, .. } = decl {
                    *parent
                } else {
                    None
                }
            })
            .filter_map(|parent_id| type_id_to_name.get(&parent_id).cloned())
            .collect();

        // -----------------------------------------------------------------------
        // Step 1: allocate one TypeVarId per global name in a shared arena.
        // -----------------------------------------------------------------------
        let mut arena = TypeVarArena::new();
        let mut global_name_vars: HashMap<String, crate::ir::ty::TypeVarId> = HashMap::new();

        // Pre-allocate TypeVarIds for all declared globals, binding concrete ones.
        for g in &module.globals {
            let v = arena.fresh();
            if is_concrete(&g.ty) {
                arena.bind(v, g.ty.clone());
            }
            global_name_vars.insert(g.name.clone(), v);
        }

        // Pre-scan functions for undeclared global names used in SystemCall ops.
        if !module.system_call_type_rules.is_empty() {
            for func in module.functions.values() {
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

        // Pre-scan for undeclared global names used in Op::Call ops whose
        // Function::type_rule references globals (GML syscalls carry their
        // type rule on Function::type_rule rather than system_call_type_rules).
        let intrinsic_has_globals = module.runtime_registry.values().any(|&fid| {
            matches!(
                module.functions[fid].type_rule,
                Some(
                    SystemCallTypeRule::GlobalStore { .. }
                        | SystemCallTypeRule::ResolveGlobalType
                        | SystemCallTypeRule::ResolveGlobalTypeStructOnly { .. }
                )
            )
        });
        if intrinsic_has_globals {
            // Build FuncId → name_arg index map for intrinsics with global rules.
            let intrinsic_global_rules: HashMap<FuncId, usize> = module
                .runtime_registry
                .values()
                .filter_map(|&fid| {
                    let name_arg = match module.functions[fid].type_rule {
                        Some(SystemCallTypeRule::GlobalStore { name_arg, .. }) => name_arg,
                        Some(
                            SystemCallTypeRule::ResolveGlobalType
                            | SystemCallTypeRule::ResolveGlobalTypeStructOnly { .. },
                        ) => 0,
                        _ => return None,
                    };
                    Some((fid, name_arg))
                })
                .collect();

            for func in module.functions.values() {
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
                    if let Op::Call {
                        func: callee_fid,
                        args,
                    } = &inst.op
                    {
                        let Some(&name_arg) = intrinsic_global_rules.get(callee_fid) else {
                            continue;
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
        struct FuncData {
            value_vars: HashMap<ValueId, crate::ir::ty::TypeVarId>,
            return_var: crate::ir::ty::TypeVarId,
            constraint_set_param_lower_bounds: Vec<(TypeVarId, Type)>,
        }

        let mut all_constraints: Vec<TypeConstraint> = Vec::new();
        let mut func_data: Vec<FuncData> = Vec::new();

        for (_, func) in module.functions.iter() {
            let set = collect_function(func, &module, &mut arena, &global_name_vars);
            all_constraints.extend(set.constraints);
            func_data.push(FuncData {
                value_vars: set.value_vars,
                return_var: set.return_var,
                constraint_set_param_lower_bounds: set.param_lower_bounds,
            });
        }

        // -----------------------------------------------------------------------
        // Accumulates concrete types flowing into each param var from multiple call sites.
        // Drained after the interprocedural loop to emit union constraints.
        let mut param_concrete_types: BTreeMap<TypeVarId, ParamEvidence> = BTreeMap::new();

        // Step 3: emit interprocedural call-site constraints.
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
            let fid_to_idx: HashMap<FuncId, usize> = module
                .functions
                .keys()
                .enumerate()
                .map(|(idx, fid)| (fid, idx))
                .collect();
            // name → (idx, fid). Built by inserting in FuncId order; a name shared
            // by multiple functions (collision) keeps the last insertion. Collisions
            // make name-based dispatch (MethodCall / MakeClosure / GlobalRef) ambiguous:
            // a caller naming a colliding function is attributed to only one of them, so
            // every same-named function has an under-enumerated caller set. Such
            // functions are marked incomplete below so they never narrow on partial
            // evidence.
            let mut name_to_idx: HashMap<&str, (usize, FuncId)> = HashMap::new();
            let mut name_seen: HashMap<&str, FuncId> = HashMap::new();
            let mut name_collisions: HashSet<&str> = HashSet::new();
            for (idx, fid) in module.functions.keys().enumerate() {
                let name = module.func_name(fid);
                if name_seen.insert(name, fid).is_some() {
                    name_collisions.insert(name);
                }
                name_to_idx.insert(name, (idx, fid));
            }
            // name → fid for FuncId-escape resolution (any module function name).
            let name_to_fid: HashMap<&str, FuncId> =
                name_seen.iter().map(|(&n, &f)| (n, f)).collect();
            let addr_taken = compute_address_taken(&module, &name_to_fid);
            for (caller_idx, (caller_fid, func)) in module.functions.iter().enumerate() {
                let _caller_name = module.func_name(caller_fid);
                let caller_data = &func_data[caller_idx];

                for block in func.blocks.values() {
                    for &inst_id in &block.insts {
                        let inst = &func.insts[inst_id];

                        match &inst.op {
                            Op::Call {
                                func: callee_fid,
                                args,
                            } => {
                                if *callee_fid == caller_fid {
                                    continue;
                                }
                                if let Some(&callee_idx) = fid_to_idx.get(callee_fid) {
                                    let callee_fid = *callee_fid;
                                    let callee_func = &module.functions[callee_fid];
                                    let callee_data = &func_data[callee_idx];
                                    let entry = callee_func.entry;
                                    let entry_params = &callee_func.blocks[entry].params;

                                    for (i, &arg) in args.iter().enumerate() {
                                        if i >= entry_params.len() {
                                            break;
                                        }
                                        let arg_ty = &func.value_types[arg];
                                        let param_val = entry_params[i].value;
                                        let param_ty = &callee_func.value_types[param_val];
                                        let is_struct_arg =
                                            matches!(arg_ty, Type::Instance(_) | Type::ClassRef(_));
                                        let usage_suppressed = if i == 0 {
                                            !is_struct_arg
                                                && param_used_as_collection(
                                                    callee_func,
                                                    param_val,
                                                    &module.array_like_fids,
                                                )
                                        } else {
                                            is_definitely_scalar(arg_ty)
                                                && param_used_with_field_access(
                                                    callee_func,
                                                    param_val,
                                                    &module.array_like_fids,
                                                )
                                        };
                                        seed_param_from_arg(
                                            &mut param_concrete_types,
                                            arg_ty,
                                            param_ty,
                                            caller_data.value_vars.get(&arg).copied(),
                                            callee_data.value_vars.get(&param_val).copied(),
                                            usage_suppressed,
                                        );
                                    }

                                    // Link caller result ← callee return_var.
                                    // Skip Void callees — propagating Void to the
                                    // caller result would produce spurious type errors.
                                    if let Some(result) = inst.result {
                                        if !matches!(callee_func.sig.return_ty, Type::Void) {
                                            if let Some(&result_var) =
                                                caller_data.value_vars.get(&result)
                                            {
                                                all_constraints.push(TypeConstraint::Equal(
                                                    Type::InferVar(result_var),
                                                    Type::InferVar(callee_data.return_var),
                                                ));
                                                // If the callee's return type is already
                                                // concrete (e.g. a runtime builtin), emit a
                                                // direct constraint so the caller result var
                                                // is resolved even if return_var never gets
                                                // bound through transitive constraints.
                                                if is_concrete(&callee_func.sig.return_ty) {
                                                    all_constraints.push(TypeConstraint::Equal(
                                                        Type::InferVar(result_var),
                                                        callee_func.sig.return_ty.clone(),
                                                    ));
                                                }
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
                                if let Some(&(callee_idx, callee_fid)) =
                                    name_to_idx.get(method.as_str())
                                {
                                    if callee_fid == caller_fid {
                                        continue;
                                    }
                                    let callee_func = &module.functions[callee_fid];
                                    let callee_data = &func_data[callee_idx];
                                    let entry = callee_func.entry;
                                    let entry_params = &callee_func.blocks[entry].params;

                                    // Link receiver to param[0] (self).
                                    if !entry_params.is_empty() {
                                        let recv_ty = &func.value_types[*receiver];
                                        let param_val = entry_params[0].value;
                                        let param_ty = &callee_func.value_types[param_val];
                                        let usage_suppressed = param_used_as_collection(
                                            callee_func,
                                            param_val,
                                            &module.array_like_fids,
                                        );
                                        seed_param_from_arg(
                                            &mut param_concrete_types,
                                            recv_ty,
                                            param_ty,
                                            caller_data.value_vars.get(receiver).copied(),
                                            callee_data.value_vars.get(&param_val).copied(),
                                            usage_suppressed,
                                        );
                                    }

                                    // Link args to params[1..] (skip self).
                                    for (i, &arg) in args.iter().enumerate() {
                                        let param_idx = i + 1;
                                        if param_idx >= entry_params.len() {
                                            break;
                                        }
                                        let arg_ty = &func.value_types[arg];
                                        let param_val = entry_params[param_idx].value;
                                        let param_ty = &callee_func.value_types[param_val];
                                        let usage_suppressed = is_definitely_scalar(arg_ty)
                                            && param_used_with_field_access(
                                                callee_func,
                                                param_val,
                                                &module.array_like_fids,
                                            );
                                        seed_param_from_arg(
                                            &mut param_concrete_types,
                                            arg_ty,
                                            param_ty,
                                            caller_data.value_vars.get(&arg).copied(),
                                            callee_data.value_vars.get(&param_val).copied(),
                                            usage_suppressed,
                                        );
                                    }

                                    // Link caller result ← callee return_var.
                                    if let Some(result) = inst.result {
                                        if !matches!(callee_func.sig.return_ty, Type::Void) {
                                            if let Some(&result_var) =
                                                caller_data.value_vars.get(&result)
                                            {
                                                all_constraints.push(TypeConstraint::Equal(
                                                    Type::InferVar(result_var),
                                                    Type::InferVar(callee_data.return_var),
                                                ));
                                                // If the callee's return type is already
                                                // concrete (e.g. a runtime builtin), emit a
                                                // direct constraint so the caller result var
                                                // is resolved even if return_var never gets
                                                // bound through transitive constraints.
                                                if is_concrete(&callee_func.sig.return_ty) {
                                                    all_constraints.push(TypeConstraint::Equal(
                                                        Type::InferVar(result_var),
                                                        callee_func.sig.return_ty.clone(),
                                                    ));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                            Op::MakeClosure {
                                func: callee_name,
                                captures,
                            } => {
                                if let Some(&(callee_idx, callee_fid)) =
                                    name_to_idx.get(callee_name.as_str())
                                {
                                    let callee_func = &module.functions[callee_fid];
                                    let callee_data = &func_data[callee_idx];
                                    let entry = callee_func.entry;
                                    let entry_params = &callee_func.blocks[entry].params;

                                    let capture_param_offset = callee_func.sig.params.len();
                                    for (i, &capture) in captures.iter().enumerate() {
                                        let param_idx = capture_param_offset + i;
                                        if param_idx >= entry_params.len() {
                                            break;
                                        }
                                        let capture_ty = &func.value_types[capture];
                                        let param_val = entry_params[param_idx].value;
                                        let param_ty = &callee_func.value_types[param_val];
                                        let is_struct_arg = matches!(
                                            capture_ty,
                                            Type::Instance(_) | Type::ClassRef(_)
                                        );
                                        let usage_suppressed = !is_struct_arg
                                            && param_used_with_field_access(
                                                callee_func,
                                                param_val,
                                                &module.array_like_fids,
                                            );
                                        seed_param_from_arg(
                                            &mut param_concrete_types,
                                            capture_ty,
                                            param_ty,
                                            caller_data.value_vars.get(&capture).copied(),
                                            callee_data.value_vars.get(&param_val).copied(),
                                            usage_suppressed,
                                        );
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
            }

            // -------------------------------------------------------------------
            // Completeness marking over all call shapes.
            //
            // The seeding loop above enumerates only direct callers. Two classes
            // of caller are NOT enumerable from that traversal and therefore make
            // the callee's caller set incomplete:
            //
            //  1. Indirect dispatch (`Op::CallIndirect`) with an opaque callee.
            //     An address-taken function could be the target of any such call,
            //     so its caller set is under-enumerated. Non-address-taken
            //     functions are unreachable by indirect dispatch and stay sound.
            //
            //  2. Name-collision dispatch. A name shared by multiple functions is
            //     resolved to a single one by `name_to_idx`; a caller naming a
            //     colliding function is attributed to only one of them, so every
            //     same-named function has an under-enumerated caller set.
            //
            // In both cases we mark every narrowable param var of the affected
            // function `incomplete`, so the drain leaves the param free (it still
            // resolves via link-driven unification or emits as the honest
            // inference-failure `unknown`). This is the soundness gate over all
            // call shapes; without it, single-caller or LCA narrowing of a
            // function with an un-enumerated indirect/colliding caller is unsound.
            let mark_func_params_incomplete =
                |param_concrete_types: &mut BTreeMap<TypeVarId, ParamEvidence>,
                 callee_idx: usize,
                 callee_func: &crate::ir::Function| {
                    let callee_data = &func_data[callee_idx];
                    let entry = callee_func.entry;
                    for p in &callee_func.blocks[entry].params {
                        let param_ty = &callee_func.value_types[p.value];
                        if is_concrete(param_ty) {
                            continue;
                        }
                        if let Some(&param_var) = callee_data.value_vars.get(&p.value) {
                            param_concrete_types
                                .entry(param_var)
                                .or_default()
                                .incomplete = true;
                        }
                    }
                };

            for (callee_idx, (callee_fid, callee_func)) in module.functions.iter().enumerate() {
                let name = module.func_name(callee_fid);
                let unenumerable = name_collisions.contains(name)
                    || (addr_taken.opaque_indirect
                        && addr_taken.address_taken.contains(&callee_fid));
                if unenumerable {
                    mark_func_params_incomplete(&mut param_concrete_types, callee_idx, callee_func);
                }
            }
        }

        // Seed param TypeVars from sig.defaults so default-argument types contribute
        // alongside call-site types. Skips already-concrete params (same guard as
        // call-site seeding above).
        for (callee_idx, (_, func)) in module.functions.iter().enumerate() {
            let callee_data = &func_data[callee_idx];
            let entry = func.entry;
            let entry_params = &func.blocks[entry].params;
            for (i, default) in func.sig.defaults.iter().enumerate() {
                let Some(constant) = default else { continue };
                if i >= entry_params.len() {
                    break;
                }
                let param_val = entry_params[i].value;
                let param_ty = &func.value_types[param_val];
                if is_concrete(param_ty) {
                    continue;
                }
                let Some(&param_var) = callee_data.value_vars.get(&param_val) else {
                    continue;
                };
                let default_ty = constant.ty();
                param_concrete_types.entry(param_var).or_default().default = Some(default_ty);
            }
        }

        // Per-param declared lower bound, used as the join fallback when caller
        // types share no common ancestor (or are not all instances). Set by the
        // frontend (e.g. GMLObject for ownerless GML script `self`); never
        // hardcoded in the join.
        let param_lower_bounds: HashMap<TypeVarId, Type> = func_data
            .iter()
            .flat_map(|fd| fd.constraint_set_param_lower_bounds.iter().cloned())
            .collect();

        // Params whose call-site evidence is a COMPLETE caller enumeration
        // (`!incomplete`, non-empty `call_site`). Their type is decided by the
        // SOUND post-fixpoint join of their actual callers, so the HasField
        // single-owner heuristic must NOT pre-bind them (join precedence — Part 3):
        // the heuristic is a fallback for evidence-less values, and binding such a
        // param to one caller's leaf type is unsound when the caller set spans
        // siblings. These vars are gated out of HasField narrowing during the
        // fixpoint and resolved by the post-fixpoint join below.
        let join_param_vars: HashSet<TypeVarId> = param_concrete_types
            .iter()
            .filter(|(_, ev)| !ev.incomplete && !ev.call_site.is_empty())
            .map(|(var, _)| *var)
            .collect();

        // -----------------------------------------------------------------------
        // Step 4: solve the equality constraints jointly.
        //
        // `HasField { ty: Var(_) }` and `Callable { ty: Var(_) }` constraints
        // cannot be resolved until the object/callee type variable is bound by
        // a later `Equal` constraint. We run a fixpoint loop: each pass
        // processes the pending list, and any constraint that still cannot be
        // resolved is re-deferred. We stop when either:
        //   (a) the deferred list is empty (all resolved), or
        //   (b) a full pass made no progress (deferred list no shorter than before).
        //
        // Call-site param JOINS are NOT applied here — they are computed
        // post-fixpoint (Step 4.4) over the COMPLETE lower-bound set, which is
        // only available once linked param-var callers have themselves resolved
        // via the equality fixpoint.
        // -----------------------------------------------------------------------
        let run_fixpoint =
            |arena: &mut TypeVarArena, mut pending: Vec<TypeConstraint>| -> Vec<TypeConstraint> {
                loop {
                    let pending_count = pending.len();
                    let mut deferred: Vec<TypeConstraint> = Vec::new();
                    for c in pending {
                        process_constraint(
                            c,
                            arena,
                            &own_fields,
                            &all_fields,
                            &type_id_to_name,
                            &name_to_type_id,
                            &non_leaf_type_names,
                            &join_param_vars,
                            &mut deferred,
                        );
                    }
                    let did_bind = arena.take_did_bind();
                    if deferred.is_empty() || (!did_bind && deferred.len() >= pending_count) {
                        return deferred;
                    }
                    pending = deferred;
                }
            };
        let mut stalled_deferred: Vec<TypeConstraint> = run_fixpoint(&mut arena, all_constraints);

        // -----------------------------------------------------------------------
        // Step 4.4: resolve call-site param JOINS (post-fixpoint).
        //
        // For each param with COMPLETE call-site evidence, its lower-bound set is
        // the concrete `call_site` types PLUS each linked param-var caller
        // resolved through the arena (now that the equality fixpoint has run). The
        // JOIN (least-upper-bound; LCA for instances) of that COMPLETE set is a
        // supertype of every caller, so every caller value remains assignable —
        // narrowing the param to the join is sound. The join is computed OUTSIDE
        // the arena and bound once.
        //
        // One param's resolved join can feed another's lower bound (a join result
        // used as an argument), so we iterate to a fixpoint, re-running the
        // equality fixpoint after each round of joins to propagate. A small cap
        // bounds the iteration; convergence is asserted.
        // -----------------------------------------------------------------------
        {
            const MAX_JOIN_PASSES: usize = 3;
            let mut pass = 0;
            loop {
                let mut bound_any = false;
                for (param_var, evidence) in &param_concrete_types {
                    if !join_param_vars.contains(param_var) {
                        continue;
                    }
                    // Skip params already resolved to a concrete type (bound by an
                    // equality link during the fixpoint, or by a prior join pass).
                    let already = resolve(Type::InferVar(*param_var), &arena);
                    if !matches!(already, Type::InferVar(_)) {
                        continue;
                    }
                    // Lower-bound set: concrete call-site types + default +
                    // resolved linked param-var callers. A linked caller still
                    // free (its own evidence not yet resolved) is skipped this
                    // pass; a later pass picks it up once it resolves.
                    let mut deduped: Vec<Type> = Vec::new();
                    for ty in evidence
                        .call_site
                        .iter()
                        .cloned()
                        .chain(evidence.default.clone())
                    {
                        if !deduped.contains(&ty) {
                            deduped.push(ty);
                        }
                    }
                    for lb_var in &evidence.lower_bound_vars {
                        let r = resolve(Type::InferVar(*lb_var), &arena);
                        if !matches!(r, Type::InferVar(_)) && !deduped.contains(&r) {
                            deduped.push(r);
                        }
                    }
                    if deduped.is_empty() {
                        continue;
                    }
                    let join_ty = if deduped.len() == 1 {
                        deduped.into_iter().next().unwrap()
                    } else {
                        join_all(&module, param_lower_bounds.get(param_var), &deduped)
                    };
                    // Bind the terminal free var. The HasField gate kept this var
                    // free, so the join wins over the single-owner heuristic.
                    if let Type::InferVar(free_id) = resolve(Type::InferVar(*param_var), &arena) {
                        if arena.binding_of(free_id).is_none() {
                            arena.bind(free_id, join_ty);
                            bound_any = true;
                        }
                    }
                }
                if !bound_any {
                    break;
                }
                // Propagate the new join bindings (a joined param may unlock a
                // HasField/Callable, and its result may feed another param's join
                // lower bound in the next pass).
                stalled_deferred = run_fixpoint(&mut arena, stalled_deferred);
                pass += 1;
                assert!(
                    pass < MAX_JOIN_PASSES,
                    "call-site param join did not converge within {} passes",
                    MAX_JOIN_PASSES
                );
            }
        }

        // -----------------------------------------------------------------------
        // Step 4.5: multi-field struct narrowing.
        //
        // After the fixpoint loop stalls, group remaining HasField{ty:Var}
        // constraints by TypeVarId and intersect candidate struct sets across
        // all fields for the same Var. If the intersection is a singleton,
        // unify the Var to that struct type and emit field-type constraints.
        // Then run one more fixpoint pass to propagate the unlocked constraints.
        // -----------------------------------------------------------------------
        let mut new_from_narrowing: Vec<TypeConstraint> = Vec::new();
        {
            use crate::ir::ty::TypeVarId;

            // Group: TypeVarId → set of field names still unresolved on that var.
            let mut var_fields: BTreeMap<TypeVarId, BTreeSet<String>> = BTreeMap::new();
            for c in &stalled_deferred {
                if let TypeConstraint::HasField {
                    ty: Type::InferVar(var_id),
                    field,
                    ..
                } = c
                {
                    var_fields.entry(*var_id).or_default().insert(field.clone());
                }
            }

            for (var_id, fields) in &var_fields {
                // Join precedence (Part 3): a param with COMPLETE call-site
                // evidence is resolved by the post-fixpoint join, never by the
                // multi-field single-owner heuristic. If it is still free here the
                // join declined to bind it (e.g. no concrete lower bounds), in
                // which case Step 4.6 applies its declared lower bound — narrowing
                // it to a sibling leaf type by field-set would be unsound.
                if join_param_vars.contains(var_id) {
                    continue;
                }
                // Intersect: structs that have ALL the observed fields in their own
                // (non-inherited) fields.  Using own_fields as discriminant prevents
                // every GMLObject child from matching on inherited fields like `x` or `y`.
                // Only consider leaf types: non-leaf types are never valid narrowing targets.
                let candidates: Vec<TypeId> = name_to_type_id
                    .iter()
                    .filter(|(name, _)| !non_leaf_type_names.contains(*name))
                    .filter(|(name, _)| {
                        own_fields
                            .get(*name)
                            .is_some_and(|sf| fields.iter().all(|f| sf.contains_key(f)))
                    })
                    .map(|(_, &id)| id)
                    .collect();

                let any_field_in_non_leaf = fields.iter().any(|f| {
                    non_leaf_type_names
                        .iter()
                        .any(|name| own_fields.get(name).is_some_and(|sf| sf.contains_key(f)))
                });
                if candidates.len() == 1 && !any_field_in_non_leaf {
                    let type_id = candidates[0];
                    let _ = unify(Type::InferVar(*var_id), Type::Instance(type_id), &mut arena);
                    // Emit field-type constraints for all HasField constraints on this var.
                    // Use all_fields so inherited fields resolve correctly.
                    if let Some(name) = type_id_to_name.get(&type_id) {
                        if let Some(sf) = all_fields.get(name) {
                            for c in &stalled_deferred {
                                if let TypeConstraint::HasField {
                                    ty: Type::InferVar(vid),
                                    field,
                                    field_ty,
                                } = c
                                {
                                    if vid == var_id {
                                        if let Some(ft) = sf.get(field) {
                                            new_from_narrowing.push(TypeConstraint::Equal(
                                                field_ty.clone(),
                                                ft.clone(),
                                            ));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // If any vars were narrowed, run one more fixpoint pass to propagate.
        if !new_from_narrowing.is_empty() {
            let mut pending2 = new_from_narrowing;
            // Re-include stalled_deferred so newly-narrowed HasField constraints can resolve.
            pending2.extend(stalled_deferred);
            let _ = run_fixpoint(&mut arena, pending2);
        }

        // -----------------------------------------------------------------------
        // Step 4.6: apply param lower bounds.
        //
        // If a param TypeVar is still free after the fixpoint (no call-site
        // narrowed it), bind it to the lower bound declared in the signature.
        // This ensures ownerless GML script `self` params default to GMLObject
        // rather than remaining unresolved (which emits `unknown`).
        // -----------------------------------------------------------------------
        {
            let all_lower_bounds: Vec<(TypeVarId, Type)> = func_data
                .iter()
                .flat_map(|fd| fd.constraint_set_param_lower_bounds.iter().cloned())
                .collect();

            for (var, lb) in &all_lower_bounds {
                let resolved = resolve(Type::InferVar(*var), &arena);
                let is_free = matches!(resolved, Type::InferVar(_));
                if is_free {
                    // resolved is Type::InferVar(free_id) — bind the terminal free var,
                    // not the original var (which may already be bound to free_id).
                    if let Type::InferVar(free_id) = resolved {
                        if arena.binding_of(free_id).is_none() {
                            arena.bind(free_id, lb.clone());
                        }
                    }
                }
            }
        }

        // -----------------------------------------------------------------------
        // Step 5: collect per-function value-type updates.
        //
        // For every value that was an inference target (Unknown or Var), resolve
        // its TypeVar and collect the result. If still unresolved (Var), leave
        // the existing type unchanged — the emit step converts residual Var to
        // `unknown`. Only write back concrete resolved types.
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
                    if !matches!(old_ty, Type::Value | Type::InferVar(_)) {
                        continue;
                    }
                    let resolved = resolve(Type::InferVar(*var_id), &arena);
                    // If still unresolved, leave the existing `InferVar` in place
                    // — do NOT lower it to `Type::Value`. This is load-bearing for
                    // the loud posture: an unresolved inference variable must
                    // surface as a hard escaped-typevar error (RC0006) via
                    // `ValidateNoEscapedTypeVars`, not silently become a dynamic
                    // `Value`/`unknown`. Leaving it in place also preserves
                    // re-inference across subsequent HM passes.
                    if matches!(&resolved, Type::InferVar(_)) {
                        continue;
                    }
                    if resolved != *old_ty {
                        updates.push((vid.index() as usize, resolved));
                    }
                }
                FuncUpdate { updates }
            })
            .collect();

        // -----------------------------------------------------------------------
        // Step 6: apply per-function updates (value_types, sig.params, block
        // param types, sig.return_ty).
        //
        // When `dirty` is Some, only write back to functions in the dirty set.
        // Constraint collection above still read all functions, so the global
        // type environment is always fully built. The `changed_funcs` set
        // tracks which functions' value_types or sig actually changed.
        // -----------------------------------------------------------------------
        use crate::pipeline::checker::{Diagnostic, DiagnosticCode, RcDiagnostic, Severity};
        let mut changed_funcs_set: HashSet<FuncId> = HashSet::new();
        let mut new_diagnostics: Vec<Diagnostic> = Vec::new();
        let func_ids: Vec<FuncId> = module.functions.keys().collect();
        let func_names: Vec<String> = func_ids
            .iter()
            .map(|&fid| module.func_name(fid).to_string())
            .collect();
        for (((func_id, _fname), update), data) in func_ids
            .iter()
            .copied()
            .zip(func_names.iter())
            .zip(func_updates.iter())
            .zip(func_data.iter())
        {
            // In dirty-aware mode, only write back to functions in the dirty set.
            if dirty.is_some_and(|d| !d.contains(&func_id)) {
                continue;
            }
            let func = &mut module.functions[func_id];
            for &(idx, ref new_ty) in &update.updates {
                let vid = ValueId::new(idx as u32);
                if &func.value_types[vid] != new_ty {
                    func.value_types[vid] = new_ty.clone();
                    changed_funcs_set.insert(func_id);
                }
            }

            // Sync entry block param.ty and sig.params from value_types.
            // Write unconditionally — the solver is authoritative. Skip only if
            // the resolved type is still Var (solver never touched this value).
            let entry = func.entry;
            let entry_param_count = func.blocks[entry].params.len();
            for i in 0..entry_param_count {
                let p_value = func.blocks[entry].params[i].value;
                let vty = func.value_types[p_value].clone();
                if !matches!(vty, Type::InferVar(_)) {
                    if func.blocks[entry].params[i].ty != vty {
                        func.blocks[entry].params[i].ty = vty.clone();
                        changed_funcs_set.insert(func_id);
                    }
                    if i < func.sig.params.len() && func.sig.params[i] != vty {
                        func.sig.params[i] = vty;
                        changed_funcs_set.insert(func_id);
                    }
                }
            }

            // Sync sig.return_ty ← resolved return_var.
            // Skip only if still Var (solver never constrained the return).
            let resolved_ret = resolve(Type::InferVar(data.return_var), &arena);
            if !matches!(resolved_ret, Type::InferVar(_)) && func.sig.return_ty != resolved_ret {
                func.sig.return_ty = resolved_ret;
                changed_funcs_set.insert(func_id);
            }
        }
        module.diagnostics.append(&mut new_diagnostics);

        // -----------------------------------------------------------------------
        // Step 7: write back improved global types to module.globals.
        //
        // Only update declared globals that were Unknown and now have a more
        // concrete resolved type.
        // -----------------------------------------------------------------------
        let mut globals_changed = false;
        for g in &mut module.globals {
            if let Some(&var_id) = global_name_vars.get(&g.name) {
                let resolved = resolve(Type::InferVar(var_id), &arena);
                if !matches!(resolved, Type::InferVar(_)) && g.ty != resolved {
                    g.ty = resolved;
                    globals_changed = true;
                }
            }
        }

        // -----------------------------------------------------------------------
        // Step 6.5: resolve stale TypeVars in struct field types.
        //
        // ConstructorStructInfer commits field types before HM runs, so any
        // field whose type was Type::InferVar(v) at CSI time still holds that InferVar
        // after HM write-back.  Walk module.types here and resolve every field
        // type.  Unions have Unknown stripped when a concrete alternative exists
        // — matching CSI's merge_field_type rule.
        // -----------------------------------------------------------------------
        {
            fn resolve_field_ty(ty: Type, arena: &TypeVarArena) -> Type {
                let resolved = resolve(ty, arena);
                match resolved {
                    Type::Union(variants) => {
                        let resolved_variants: Vec<Type> =
                            variants.into_iter().map(|v| resolve(v, arena)).collect();
                        let has_concrete = resolved_variants
                            .iter()
                            .any(|v| !matches!(v, Type::Value | Type::InferVar(_)));
                        let filtered: Vec<Type> = resolved_variants
                            .into_iter()
                            .filter(|v| {
                                if has_concrete {
                                    // Drop unsettled variants (genuinely-dynamic
                                    // Value and un-inferred InferVar alike) when a
                                    // concrete alternative exists; an InferVar left
                                    // in a persisted field type would violate the
                                    // never-persist invariant.
                                    !matches!(v, Type::Value | Type::InferVar(_))
                                } else {
                                    true
                                }
                            })
                            .collect();
                        let deduped: Vec<Type> = {
                            let mut d: Vec<Type> = Vec::new();
                            for t in filtered {
                                if !d.contains(&t) {
                                    d.push(t);
                                }
                            }
                            d
                        };
                        if deduped.len() == 1 {
                            deduped.into_iter().next().unwrap()
                        } else {
                            Type::Union(deduped)
                        }
                    }
                    other => other,
                }
            }

            for decl in module.types.values_mut() {
                if let TypeDecl::Object { ref mut fields, .. } = decl {
                    for field in fields.iter_mut() {
                        if matches!(field.ty, Type::InferVar(_) | Type::Union(_)) {
                            let new_ty = resolve_field_ty(field.ty.clone(), &arena);
                            if new_ty != field.ty {
                                field.ty = new_ty;
                            }
                        }
                    }
                }
            }
        }

        // -----------------------------------------------------------------------
        // Step 8: emit inference failure diagnostics for values that remain
        // Unknown after solving.
        // -----------------------------------------------------------------------
        {
            let mut step8_diagnostics: Vec<Diagnostic> = Vec::new();

            for ((fid, func), data) in module.functions.iter().zip(func_data.iter()) {
                let mut value_op: HashMap<ValueId, &'static str> = HashMap::new();
                for inst in func.insts.values() {
                    if let Some(result) = inst.result {
                        value_op.insert(result, inst.op.variant_name());
                    }
                }

                let func_name = module.func_name(fid).to_string();

                for (vid, &var_id) in &data.value_vars {
                    if !matches!(func.value_types[*vid], Type::Value) {
                        continue;
                    }

                    let op_name = match value_op.get(vid) {
                        Some(&name) => name,
                        None => continue,
                    };
                    if op_name == "Alloc" || op_name == "Load" {
                        continue;
                    }

                    let binding = arena.binding_of(var_id);
                    let (code, message) = if binding.is_none() {
                        (
                            DiagnosticCode::Rc(RcDiagnostic::InferenceNoConstraints),
                            format!(
                                "value {:?} remains Unknown: no constraints (produced by Op::{})",
                                vid, op_name
                            ),
                        )
                    } else {
                        (
                            DiagnosticCode::Rc(RcDiagnostic::InferenceInheritedUnknown),
                            format!(
                                "value {:?} remains Unknown: all constraints resolved to Unknown (produced by Op::{})",
                                vid, op_name
                            ),
                        )
                    };

                    step8_diagnostics.push(Diagnostic {
                        file: func_name.clone(),
                        line: 0,
                        col: 0,
                        code,
                        severity: Severity::Warning,
                        message,
                    });
                }
            }

            module.diagnostics.append(&mut step8_diagnostics);
        }

        let changed = !changed_funcs_set.is_empty() || globals_changed;
        Ok(TransformResult {
            module,
            changed,
            changed_funcs: changed_funcs_set,
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
    use crate::pipeline::Transform;

    fn make_simple_module() -> Module {
        let sig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("simple", sig, Visibility::Public);
        let x = fb.param(0);
        let one = fb.const_float(1.0);
        let _sum = fb.add(x, one);
        fb.ret(Some(x));
        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        mb.build()
    }

    #[test]
    fn hm_solver_runs_on_simple_module() {
        let module = make_simple_module();
        let pass = ConstraintSolveHM;
        let result = pass.apply(module, None).expect("apply failed");
        // Should complete without panic and mark changed.
        // The function has Float(64) params so nothing to infer.
        let _ = result;
    }

    #[test]
    fn hm_solver_empty_module() {
        let module = Module::new("empty".into());
        let pass = ConstraintSolveHM;
        let result = pass.apply(module, None).expect("apply failed");
        assert!(!result.changed);
    }

    /// Build a module with: `target` (referenced via MakeClosure), `unref` (never
    /// referenced as a value), and `caller` (takes the closure ref and either
    /// makes the closure or performs an opaque indirect call).
    fn module_with_escapes(opaque_indirect: bool) -> Module {
        let mut mb = ModuleBuilder::new("test");

        let tsig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut tb = FunctionBuilder::new("target", tsig, Visibility::Public);
        let _ = tb.param(0);
        tb.ret(None);
        mb.add_function(tb.build());

        let usig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut ub = FunctionBuilder::new("unref", usig, Visibility::Public);
        let _ = ub.param(0);
        ub.ret(None);
        mb.add_function(ub.build());

        let csig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut cb = FunctionBuilder::new("caller", csig, Visibility::Public);
        // MakeClosure("target") makes `target` address-taken.
        let _clo = cb.make_closure("target", &[], Type::Value);
        if opaque_indirect {
            // An indirect call whose callee is an opaque value (a param load, here
            // a fresh const) — not a GlobalRef to a known function.
            let opaque = cb.const_float(0.0);
            let _ = cb.call_indirect(opaque, &[], Type::Value);
        }
        cb.ret(None);
        mb.add_function(cb.build());

        mb.build()
    }

    #[test]
    fn address_taken_includes_closure_target_only() {
        let module = module_with_escapes(false);
        let name_to_fid: HashMap<&str, FuncId> = module
            .functions
            .keys()
            .map(|fid| (module.func_name(fid), fid))
            .collect();
        let analysis = compute_address_taken(&module, &name_to_fid);
        let target = name_to_fid["target"];
        let unref = name_to_fid["unref"];
        assert!(
            analysis.address_taken.contains(&target),
            "MakeClosure target must be address-taken"
        );
        assert!(
            !analysis.address_taken.contains(&unref),
            "a function never referenced as a value must not be address-taken"
        );
        assert!(
            !analysis.opaque_indirect,
            "no CallIndirect ⇒ no opaque indirect dispatch"
        );
    }

    #[test]
    fn opaque_call_indirect_sets_opaque_flag() {
        let module = module_with_escapes(true);
        let name_to_fid: HashMap<&str, FuncId> = module
            .functions
            .keys()
            .map(|fid| (module.func_name(fid), fid))
            .collect();
        let analysis = compute_address_taken(&module, &name_to_fid);
        assert!(
            analysis.opaque_indirect,
            "an unresolvable CallIndirect callee must set opaque_indirect"
        );
    }

    #[test]
    fn global_ref_to_function_is_address_taken_but_resolvable_indirect_is_not_opaque() {
        // caller2: globalref("target") then call_indirect(that ref) — a resolvable
        // indirect call. `target` is address-taken (its name escaped via GlobalRef),
        // but the call is NOT opaque because the callee traces to a known function.
        let mut mb = ModuleBuilder::new("test");
        let tsig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut tb = FunctionBuilder::new("target", tsig, Visibility::Public);
        let _ = tb.param(0);
        tb.ret(None);
        mb.add_function(tb.build());

        let csig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut cb = FunctionBuilder::new("caller2", csig, Visibility::Public);
        let r = cb.global_ref("target", Type::Value);
        let _ = cb.call_indirect(r, &[], Type::Value);
        cb.ret(None);
        mb.add_function(cb.build());

        let module = mb.build();
        let name_to_fid: HashMap<&str, FuncId> = module
            .functions
            .keys()
            .map(|fid| (module.func_name(fid), fid))
            .collect();
        let analysis = compute_address_taken(&module, &name_to_fid);
        assert!(analysis.address_taken.contains(&name_to_fid["target"]));
        assert!(
            !analysis.opaque_indirect,
            "a CallIndirect whose callee is a GlobalRef to a known function is resolvable, not opaque"
        );
    }
}
