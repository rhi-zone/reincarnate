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
//!    only the emit step converts residual `Type::Var` to `unknown`.

use std::collections::{HashMap, HashSet};

use crate::entity::EntityRef;
use crate::error::CoreError;
use crate::ir::inst::Op;
use crate::ir::module::{SystemCallTypeRule, TypeDecl};
use crate::ir::ty::{TypeConstraint, TypeId};
use crate::ir::{Constant, FuncId, Module, Type, ValueId};
use crate::pipeline::{Transform, TransformResult};
use crate::transforms::constraint_collect::{
    collect_function, is_concrete, resolve, unify, TypeVarArena,
};

/// Build the own-fields map (struct name → field name → field type) from a module.
///
/// "Own fields" means only the fields declared directly on each struct, not
/// inherited from parent types.  Used for struct narrowing discriminants.
fn build_own_fields(module: &Module) -> HashMap<String, HashMap<String, Type>> {
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

    for s in &module.structs {
        // Walk the TypeDecl parent chain and merge ancestor own-fields.
        // Own fields (already in the map) take priority — use entry().or_insert.
        let entry = all_fields.entry(s.name.clone()).or_default();
        let mut current_name: Option<String> = Some(s.name.clone());
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
    deferred: &mut Vec<TypeConstraint>,
) {
    match c {
        TypeConstraint::Equal(a, b) => {
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
                        // Use all_fields: resolves fields inherited from parent types.
                        if let Some(fields) = all_fields.get(name) {
                            if let Some(ft) = fields.get(&field) {
                                deferred.push(TypeConstraint::Equal(field_ty, ft.clone()));
                            }
                        }
                    }
                    // Unknown field — skip; don't invent a type.
                }
                Type::Var(_) => {
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
                    if candidates.len() == 1 {
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

/// Check whether a callee parameter value is used as a collection (array or map)
/// in the callee's body.  Used to suppress interprocedural narrowing that would
/// incorrectly convert array/map params to scalar types.
fn param_used_as_collection(
    func: &crate::ir::Function,
    param_val: ValueId,
    array_like_fns: &std::collections::HashSet<String>,
    func_names: &HashMap<FuncId, String>,
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
                    let callee_name = func_names.get(callee_fid).map(|s| s.as_str()).unwrap_or("");
                    if array_like_fns.contains(callee_name) {
                        return true;
                    }
                }
                Op::MethodCall {
                    receiver,
                    method,
                    args,
                } if (*receiver == param_val || args.contains(&param_val)) => {
                    if array_like_fns.contains(method.as_str()) {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

/// Check whether a callee parameter value is used with field access in the callee's body.
fn param_used_with_field_access(
    func: &crate::ir::Function,
    param_val: ValueId,
    array_like_fns: &std::collections::HashSet<String>,
    func_names: &HashMap<FuncId, String>,
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
    param_used_as_collection(func, param_val, array_like_fns, func_names)
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

        // Pre-scan for undeclared global names used in intrinsic Op::Call ops
        // (Phase 3a: GML syscalls registered via register_runtime_intrinsic carry
        // their type rule on Function::type_rule rather than system_call_type_rules).
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
            let name_to_idx: HashMap<&str, (usize, FuncId)> = module
                .functions
                .keys()
                .enumerate()
                .map(|(idx, fid)| (module.func_name(fid), (idx, fid)))
                .collect();
            // func_names: FuncId → name, for collection-check helpers.
            let func_names: HashMap<FuncId, String> = module
                .runtime_registry
                .iter()
                .map(|(name, &fid)| (fid, name.clone()))
                .collect();

            for (caller_idx, (caller_fid, func)) in module.functions.iter().enumerate() {
                let caller_name = module.func_name(caller_fid);
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
                                        // Skip Unknown args — abstentions should not
                                        // pull the callee param toward Unknown.
                                        let arg_ty = &func.value_types[arg];
                                        if matches!(arg_ty, Type::Unknown) {
                                            continue;
                                        }
                                        let param_val = entry_params[i].value;
                                        // Skip already-concrete callee params.
                                        let param_ty = &callee_func.value_types[param_val];
                                        if is_concrete(param_ty) {
                                            continue;
                                        }
                                        let is_struct_arg =
                                            matches!(arg_ty, Type::Instance(_) | Type::ClassRef(_));
                                        let is_self_param = i == 0;
                                        if is_self_param {
                                            if !is_struct_arg
                                                && param_used_as_collection(
                                                    callee_func,
                                                    param_val,
                                                    &module.array_like_fns,
                                                    &func_names,
                                                )
                                            {
                                                continue;
                                            }
                                        } else if !is_struct_arg
                                            && param_used_with_field_access(
                                                callee_func,
                                                param_val,
                                                &module.array_like_fns,
                                                &func_names,
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

                                    // Link caller result ← callee return_var.
                                    // Skip Void callees — propagating Void to the
                                    // caller result would produce spurious type errors.
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
                                            && !param_used_as_collection(
                                                callee_func,
                                                param_val,
                                                &module.array_like_fns,
                                                &func_names,
                                            )
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
                                        let param_idx = i + 1;
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
                                        let is_struct_arg =
                                            matches!(arg_ty, Type::Instance(_) | Type::ClassRef(_));
                                        if !is_struct_arg
                                            && param_used_with_field_access(
                                                callee_func,
                                                param_val,
                                                &module.array_like_fns,
                                                &func_names,
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

                                    // Link caller result ← callee return_var.
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

        // -----------------------------------------------------------------------
        // Step 4: solve all constraints jointly.
        //
        // `HasField { ty: Var(_) }` and `Callable { ty: Var(_) }` constraints
        // cannot be resolved until the object/callee type variable is bound by
        // a later `Equal` constraint. We run a fixpoint loop: each pass
        // processes the pending list, and any constraint that still cannot be
        // resolved is re-deferred. We stop when either:
        //   (a) the deferred list is empty (all resolved), or
        //   (b) a full pass made no progress (deferred list no shorter than before).
        // -----------------------------------------------------------------------
        let mut pending: Vec<TypeConstraint> = all_constraints;
        let stalled_deferred: Vec<TypeConstraint>;
        loop {
            let pending_count = pending.len();
            let mut deferred: Vec<TypeConstraint> = Vec::new();
            for c in pending {
                process_constraint(
                    c,
                    &mut arena,
                    &own_fields,
                    &all_fields,
                    &type_id_to_name,
                    &name_to_type_id,
                    &non_leaf_type_names,
                    &mut deferred,
                );
            }
            if deferred.is_empty() || deferred.len() >= pending_count {
                stalled_deferred = deferred;
                break;
            }
            pending = deferred;
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
            let mut var_fields: HashMap<TypeVarId, HashSet<String>> = HashMap::new();
            for c in &stalled_deferred {
                if let TypeConstraint::HasField {
                    ty: Type::Var(var_id),
                    field,
                    ..
                } = c
                {
                    var_fields.entry(*var_id).or_default().insert(field.clone());
                }
            }

            for (var_id, fields) in &var_fields {
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

                if candidates.len() == 1 {
                    let type_id = candidates[0];
                    let _ = unify(Type::Var(*var_id), Type::Instance(type_id), &mut arena);
                    // Emit field-type constraints for all HasField constraints on this var.
                    // Use all_fields so inherited fields resolve correctly.
                    if let Some(name) = type_id_to_name.get(&type_id) {
                        if let Some(sf) = all_fields.get(name) {
                            for c in &stalled_deferred {
                                if let TypeConstraint::HasField {
                                    ty: Type::Var(vid),
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
            loop {
                let pending_count = pending2.len();
                let mut deferred: Vec<TypeConstraint> = Vec::new();
                for c in pending2 {
                    process_constraint(
                        c,
                        &mut arena,
                        &own_fields,
                        &all_fields,
                        &type_id_to_name,
                        &name_to_type_id,
                        &non_leaf_type_names,
                        &mut deferred,
                    );
                }
                if deferred.is_empty() || deferred.len() >= pending_count {
                    break;
                }
                pending2 = deferred;
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
                    if !matches!(old_ty, Type::Unknown | Type::Var(_)) {
                        continue;
                    }
                    let resolved = resolve(Type::Var(*var_id), &arena);
                    // If still unresolved, leave the existing type in place.
                    // Do not write Type::Unknown — that would conflate
                    // "unconstrained" with "genuinely unknown" and block
                    // re-inference on subsequent HM passes.
                    if matches!(&resolved, Type::Var(_)) {
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
                if !matches!(vty, Type::Var(_)) {
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
            let resolved_ret = resolve(Type::Var(data.return_var), &arena);
            if !matches!(resolved_ret, Type::Var(_)) && func.sig.return_ty != resolved_ret {
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
                let resolved = resolve(Type::Var(var_id), &arena);
                if !matches!(resolved, Type::Var(_)) && g.ty != resolved {
                    g.ty = resolved;
                    globals_changed = true;
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
                    if !matches!(func.value_types[*vid], Type::Unknown) {
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
}
