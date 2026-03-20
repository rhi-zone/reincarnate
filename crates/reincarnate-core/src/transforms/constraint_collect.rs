//! Constraint collection pass (HM type-inference Phase 2).
//!
//! Walks every IR function and emits [`TypeConstraint`] values for every [`Op`]
//! that carries type information.  No solving happens here — collection only.
//!
//! The collected constraints are stored in [`ConstraintSet`] values, one per
//! function, and accumulated in [`ConstraintCollect::constraint_sets`].

use std::collections::{HashMap, HashSet};

use crate::error::CoreError;
use crate::ir::block::BlockId;
use crate::ir::inst::Op;
use crate::ir::module::SystemCallTypeRule;
use crate::ir::ty::{FunctionSig, Type, TypeConstraint, TypeVarId};
use crate::ir::{Constant, Function, Module, ValueId};
use crate::pipeline::{Transform, TransformResult};
use crate::transforms::constraint_solve2::TypeVarArena;

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
        Type::Unknown | Type::Var(_) => false,
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
        | Type::Struct(_)
        | Type::Enum(_)
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

    // Const-string map: ValueId → string literal value.
    // Only built when there are SystemCall rules to process.
    let const_strings: HashMap<ValueId, &str> =
        if store_rules.is_empty() && resolve_rules.is_empty() {
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

                // Op::Add is excluded — overloaded for string concatenation in GML,
                // so result type cannot be assumed to match operand types.
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
