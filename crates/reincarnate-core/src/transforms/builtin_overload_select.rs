//! `BuiltinOverloadSelect` transform pass.
//!
//! After `ConstraintSolveHM` resolves operand types, replaces `xxx_any`
//! calls with the appropriately-typed variant (`builtin.xxx_f64`, `_f32`, `_i32`,
//! `_i64`) and updates the result's `value_types` entry to the concrete return
//! type.
//!
//! # Motivation
//!
//! GML bytecode uses `DataType::Variable` for most arithmetic, so the frontend
//! emits `Op::Call { func: "add_any", … }`.  The `_any` variants
//! declare no concrete return type (returns `Unknown`), so even
//! `add_any(Float64, Float64)` produces `Unknown`.  This pass replaces those
//! calls once operand types are known from HM inference.
//!
//! # Mechanism
//!
//! Each `_any` stub registered by `Module::register_arithmetic_any_builtins` carries
//! a `specializations` table mapping concrete argument-type signatures to the
//! `FuncId` of the typed variant.  This pass looks up the callee by name in
//! `module.runtime_registry`, checks whether its `specializations` is non-empty,
//! collects the live argument types, and — if a match exists — rewrites the
//! call site to the typed variant and updates the result type.
//!
//! No string manipulation, no hardcoded type maps.

use std::collections::HashSet;

use crate::error::CoreError;
use crate::ir::func::FuncId;
use crate::ir::{Function, Module, Op, Type};
use crate::pipeline::{Transform, TransformResult};

/// `BuiltinOverloadSelect` — replaces `xxx_any` calls with typed variants.
pub struct BuiltinOverloadSelect;

/// Try to select a typed overload for one instruction in `func`.
///
/// `name_to_fid` is a pre-built map from function name to `FuncId` derived from
/// `module.runtime_registry`.
///
/// Returns `true` if the instruction was rewritten.
fn try_select(func: &mut Function, inst_id: crate::ir::InstId, module: &Module) -> bool {
    // Clone the op to avoid borrowing issues while we mutate value_types.
    let op = func.insts[inst_id].op.clone();

    let Op::Call { func: fname, args } = &op else {
        return false;
    };

    // Look up the callee FuncId from the runtime registry.
    let Some(&callee_fid) = module.runtime_registry.get(fname.as_str()) else {
        return false;
    };

    let callee = &module.functions[callee_fid];

    // Only _any stubs have a non-empty specializations table.
    if callee.specializations.is_empty() {
        return false;
    }

    // Collect concrete argument types from the calling function's value_types.
    let arg_types: Vec<Type> = args.iter().map(|&v| func.value_types[v].clone()).collect();

    // Look up the specialization for these argument types.
    let Some(&target_fid) = callee.specializations.get(&arg_types) else {
        return false;
    };

    let target = &module.functions[target_fid];
    let target_name = target.name.clone();
    let return_ty = target.sig.return_ty.clone();

    // Rewrite the call to the typed variant.
    if let Op::Call { func: fn_name, .. } = &mut func.insts[inst_id].op {
        *fn_name = target_name;
    }

    // Update the result's value type.
    if let Some(result_vid) = func.insts[inst_id].result {
        func.value_types[result_vid] = return_ty;
    }

    true
}

/// Apply overload selection to all live instructions in a function.
///
/// Returns `true` if any instruction was rewritten.
fn select_function(func: &mut Function, module: &Module) -> bool {
    let live_insts: Vec<_> = func
        .blocks
        .values()
        .flat_map(|b| b.insts.iter().copied())
        .collect();

    let mut changed = false;
    for inst_id in live_insts {
        if try_select(func, inst_id, module) {
            changed = true;
        }
    }
    changed
}

impl Transform for BuiltinOverloadSelect {
    fn name(&self) -> &str {
        "builtin-overload-select"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn requires(&self) -> &[&str] {
        &["constraint-solve-hm"]
    }

    fn apply(
        &self,
        mut module: Module,
        dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        let mut changed_funcs: HashSet<FuncId> = HashSet::new();
        for func_id in module.functions.keys().collect::<Vec<_>>() {
            if dirty.is_some_and(|d| !d.contains(&func_id)) {
                continue;
            }
            // Split the borrow: we need a mutable reference to module.functions[func_id]
            // while reading module.runtime_registry and other entries in module.functions
            // (the callee stubs).  Extract the function, run selection against the rest
            // of the module, then put it back.
            let mut func = module.functions[func_id].clone();
            if select_function(&mut func, &module) {
                module.functions[func_id] = func;
                changed_funcs.insert(func_id);
            } else {
                // func was cloned; no write needed.
            }
        }
        let changed = !changed_funcs.is_empty();
        Ok(TransformResult {
            module,
            changed,
            changed_funcs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityRef;
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::inst::Op;
    use crate::ir::ty::FunctionSig;
    use crate::ir::{Module, Type, Visibility};
    use crate::pipeline::Transform;

    /// Build a module containing one function that calls `<op_name>_any`
    /// with `arg_types.len()` parameters of the given types and the given initial
    /// result type.  Returns the module and the `FuncId` of the first user function.
    fn make_module_with_func(op_name: &str, arg_types: &[Type]) -> Module {
        let sig = FunctionSig {
            params: arg_types.to_vec(),
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Private);

        let args: Vec<_> = (0..arg_types.len()).map(|i| fb.param(i)).collect();
        let func_name = format!("{}_any", op_name);
        let call_result = fb.call(func_name, &args, Type::Unknown);
        fb.ret(Some(call_result));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let mut module = mb.build();
        // Register the GML-specific _any stubs so BuiltinOverloadSelect can
        // find and rewrite the call emitted above.
        module.register_arithmetic_any_builtins();
        module
    }

    fn first_call_id(module: &Module) -> crate::ir::InstId {
        let func_id = FuncId::new(Module::NUM_CORE_BUILTINS);
        let func = &module.functions[func_id];
        func.blocks
            .values()
            .flat_map(|b| b.insts.iter().copied())
            .find(|&id| matches!(&func.insts[id].op, Op::Call { .. }))
            .expect("no Call instruction found")
    }

    #[test]
    fn add_f64_both_operands_f64() {
        let module = make_module_with_func("add", &[Type::Float(64), Type::Float(64)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(result.changed);

        let func_id = FuncId::new(Module::NUM_CORE_BUILTINS);
        let inst_id = first_call_id(&result.module);
        let func = &result.module.functions[func_id];

        let Op::Call { func: fname, .. } = &func.insts[inst_id].op else {
            panic!("expected Call");
        };
        assert_eq!(fname, "builtin.add_f64");
        let result_vid = func.insts[inst_id].result.unwrap();
        assert_eq!(func.value_types[result_vid], Type::Float(64));
    }

    #[test]
    fn neg_i32() {
        let module = make_module_with_func("neg", &[Type::Int(32)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(result.changed);

        let func_id = FuncId::new(Module::NUM_CORE_BUILTINS);
        let inst_id = first_call_id(&result.module);
        let func = &result.module.functions[func_id];

        let Op::Call { func: fname, .. } = &func.insts[inst_id].op else {
            panic!()
        };
        assert_eq!(fname, "builtin.neg_i32");
        let result_vid = func.insts[inst_id].result.unwrap();
        assert_eq!(func.value_types[result_vid], Type::Int(32));
    }

    #[test]
    fn mismatched_types_left_unchanged() {
        let module = make_module_with_func("add", &[Type::Float(64), Type::Int(32)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(!result.changed);

        let func_id = FuncId::new(Module::NUM_CORE_BUILTINS);
        let inst_id = first_call_id(&result.module);
        let func = &result.module.functions[func_id];

        let Op::Call { func: fname, .. } = &func.insts[inst_id].op else {
            panic!()
        };
        assert_eq!(fname, "add_any");
    }

    #[test]
    fn unknown_operand_left_unchanged() {
        let module = make_module_with_func("mul", &[Type::Unknown, Type::Unknown]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(!result.changed);

        let func_id = FuncId::new(Module::NUM_CORE_BUILTINS);
        let inst_id = first_call_id(&result.module);
        let func = &result.module.functions[func_id];

        let Op::Call { func: fname, .. } = &func.insts[inst_id].op else {
            panic!()
        };
        assert_eq!(fname, "mul_any");
    }

    #[test]
    fn sub_i64_both_i64() {
        let module = make_module_with_func("sub", &[Type::Int(64), Type::Int(64)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(result.changed);

        let func_id = FuncId::new(Module::NUM_CORE_BUILTINS);
        let inst_id = first_call_id(&result.module);
        let func = &result.module.functions[func_id];

        let Op::Call { func: fname, .. } = &func.insts[inst_id].op else {
            panic!()
        };
        assert_eq!(fname, "builtin.sub_i64");
        let result_vid = func.insts[inst_id].result.unwrap();
        assert_eq!(func.value_types[result_vid], Type::Int(64));
    }

    #[test]
    fn non_builtin_call_ignored() {
        // A call that doesn't start with "builtin." should not be touched.
        let sig = FunctionSig {
            params: vec![Type::Float(64), Type::Float(64)],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Private);
        let a = fb.param(0);
        let b = fb.param(1);
        let v = fb.call("user.add_any", &[a, b], Type::Unknown);
        fb.ret(Some(v));
        let func = fb.build();
        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(!result.changed);
    }
}
