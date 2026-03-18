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
use crate::ir::ty::{Type, TypeConstraint, TypeVarId};
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
    /// - [`Type::Dynamic`] and [`Type::Unknown`] and [`Type::Var`] get fresh,
    ///   unbound vars (inference targets).
    pub value_vars: HashMap<ValueId, TypeVarId>,
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
        Type::Dynamic | Type::Unknown | Type::Var(_) => false,
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
        // Dynamic / Unknown / Var(_) → leave unbound (inference target).
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
                // Copy — result has the same type as the source.
                Op::Copy(value) => {
                    if let (Some(rv), Some(sv)) = (result_var, var_for(*value, &value_vars)) {
                        constraints.push(TypeConstraint::Equal(rv, sv));
                    }
                }

                // Arithmetic — numeric grounding disabled pending full C_ARITH
                // constraint kind. Emitting Equal(operand, Float(64)) causes false
                // positives when a value is also used as a collection key or bool
                // (documented in TODO.md "Numeric grounding limitation").
                Op::Add(_, _) | Op::Sub(_, _) | Op::Mul(_, _) | Op::Div(_, _) | Op::Rem(_, _) => {}

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

                // Return — the returned value must match the function's return type.
                Op::Return(Some(value)) => {
                    if let Some(val_var) = var_for(*value, &value_vars) {
                        constraints.push(TypeConstraint::Equal(val_var, Type::Var(return_var)));
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

                // All other ops — no useful type constraint to emit at this
                // stage (branches, allocs, stores, loads, etc.).
                _ => {}
            }
        }
    }

    ConstraintSet {
        constraints,
        value_vars,
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
    for (_, pred_block) in func.blocks.iter() {
        for &inst_id in &pred_block.insts {
            let inst = &func.insts[inst_id];
            let edge_args: Option<&Vec<ValueId>> = match &inst.op {
                Op::Br { target, args } if *target == target_block => Some(args),
                Op::BrIf {
                    then_target,
                    then_args,
                    ..
                } if *then_target == target_block => Some(then_args),
                Op::BrIf {
                    else_target,
                    else_args,
                    ..
                } if *else_target == target_block => Some(else_args),
                Op::Switch { cases, default, .. } => {
                    // Switch can have multiple arms — handle all that match.
                    let mut found: Option<&Vec<ValueId>> = None;
                    for (_, case_target, case_args) in cases {
                        if *case_target == target_block {
                            found = Some(case_args);
                            break;
                        }
                    }
                    if found.is_none() && default.0 == target_block {
                        found = Some(&default.1);
                    }
                    found
                }
                _ => None,
            };

            if let Some(args) = edge_args {
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
        // Arithmetic ops (Add) do NOT emit grounding constraints — see
        // "Numeric grounding limitation" in TODO.md.
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
