//! `BuiltinOverloadSelect` transform pass.
//!
//! After `ConstraintSolveHM` resolves operand types, replaces `xxx_any`
//! calls with the appropriately-typed variant (`xxx_f64`, `_f32`, `_i32`,
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

    let Op::Call {
        func: callee_fid,
        args,
    } = &op
    else {
        return false;
    };

    let callee = &module.functions[*callee_fid];

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

    let return_ty = module.functions[target_fid].sig.return_ty.clone();

    // Rewrite the call to the typed variant.
    if let Op::Call { func: fn_id, .. } = &mut func.insts[inst_id].op {
        *fn_id = target_fid;
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
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::inst::Op;
    use crate::ir::ty::FunctionSig;
    use crate::ir::{Module, Type, Visibility};
    use crate::pipeline::Transform;

    /// Build a module containing one function that calls `<op_name>_any`
    /// with `arg_types.len()` parameters of the given types and the given initial
    /// result type.  Returns the module and the `FuncId` of the test function.
    fn make_module_with_func(op_name: &str, arg_types: &[Type]) -> (Module, FuncId) {
        use std::collections::HashMap;

        let mut mb = ModuleBuilder::new("test");

        // Register typed scalar variants for the op being tested.
        let scalar_types = [
            Type::Float(64),
            Type::Float(32),
            Type::Int(32),
            Type::Int(64),
        ];
        let suffixes = ["f64", "f32", "i32", "i64"];
        let is_binary = arg_types.len() != 1;

        let mut specializations: HashMap<Vec<Type>, FuncId> = HashMap::new();
        for (ty, suffix) in scalar_types.iter().zip(suffixes.iter()) {
            let (params, key) = if is_binary {
                (vec![ty.clone(), ty.clone()], vec![ty.clone(), ty.clone()])
            } else {
                (vec![ty.clone()], vec![ty.clone()])
            };
            let typed_sig = FunctionSig {
                params,
                return_ty: ty.clone(),
                ..Default::default()
            };
            let typed_name = format!("{op_name}_{suffix}");
            let typed_fid = mb.module_mut().register_runtime(typed_name, typed_sig);
            specializations.insert(key, typed_fid);
        }

        // Register the `_any` stub with Unknown sig.
        let func_name = format!("{op_name}_any");
        let any_sig = FunctionSig {
            params: if is_binary {
                vec![Type::Value, Type::Value]
            } else {
                vec![Type::Value]
            },
            return_ty: Type::Value,
            ..Default::default()
        };
        let any_fid = mb.module_mut().register_runtime(&func_name, any_sig);

        // Populate specializations on the stub (no dispatch body needed for tests).
        mb.module_mut().functions[any_fid].specializations = specializations;

        let sig = FunctionSig {
            params: arg_types.to_vec(),
            return_ty: Type::Value,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Private);
        let args: Vec<_> = (0..arg_types.len()).map(|i| fb.param(i)).collect();
        let call_result = fb.call(any_fid, &args, Type::Value);
        fb.ret(Some(call_result));
        let func = fb.build();

        let test_fid = mb.add_function(func);
        (mb.build(), test_fid)
    }

    fn first_call_id(module: &Module, func_id: FuncId) -> crate::ir::InstId {
        let func = &module.functions[func_id];
        func.blocks
            .values()
            .flat_map(|b| b.insts.iter().copied())
            .find(|&id| matches!(&func.insts[id].op, Op::Call { .. }))
            .expect("no Call instruction found")
    }

    fn call_name(module: &Module, func_id: FuncId, inst_id: crate::ir::InstId) -> &str {
        let func = &module.functions[func_id];
        match &func.insts[inst_id].op {
            Op::Call { func: fid, .. } => module.func_name(*fid),
            _ => panic!("expected Call"),
        }
    }

    #[test]
    fn add_f64_both_operands_f64() {
        let (module, test_fid) = make_module_with_func("add", &[Type::Float(64), Type::Float(64)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(result.changed);

        let inst_id = first_call_id(&result.module, test_fid);
        assert_eq!(call_name(&result.module, test_fid, inst_id), "add_f64");
        let func = &result.module.functions[test_fid];
        let result_vid = func.insts[inst_id].result.unwrap();
        assert_eq!(func.value_types[result_vid], Type::Float(64));
    }

    #[test]
    fn neg_i32() {
        let (module, test_fid) = make_module_with_func("neg", &[Type::Int(32)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(result.changed);

        let inst_id = first_call_id(&result.module, test_fid);
        assert_eq!(call_name(&result.module, test_fid, inst_id), "neg_i32");
        let func = &result.module.functions[test_fid];
        let result_vid = func.insts[inst_id].result.unwrap();
        assert_eq!(func.value_types[result_vid], Type::Int(32));
    }

    #[test]
    fn mismatched_types_left_unchanged() {
        let (module, test_fid) = make_module_with_func("add", &[Type::Float(64), Type::Int(32)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(!result.changed);

        let inst_id = first_call_id(&result.module, test_fid);
        assert_eq!(call_name(&result.module, test_fid, inst_id), "add_any");
    }

    #[test]
    fn unknown_operand_left_unchanged() {
        let (module, test_fid) = make_module_with_func("mul", &[Type::Value, Type::Value]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(!result.changed);

        let inst_id = first_call_id(&result.module, test_fid);
        assert_eq!(call_name(&result.module, test_fid, inst_id), "mul_any");
    }

    #[test]
    fn sub_i64_both_i64() {
        let (module, test_fid) = make_module_with_func("sub", &[Type::Int(64), Type::Int(64)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(result.changed);

        let inst_id = first_call_id(&result.module, test_fid);
        assert_eq!(call_name(&result.module, test_fid, inst_id), "sub_i64");
        let func = &result.module.functions[test_fid];
        let result_vid = func.insts[inst_id].result.unwrap();
        assert_eq!(func.value_types[result_vid], Type::Int(64));
    }

    #[test]
    fn non_builtin_call_ignored() {
        // A call to a function not in the module's specializations should not be touched.
        // Use a stub registered in the runtime registry with no specializations.
        let mut mb = ModuleBuilder::new("test");
        // Register a dummy function in the registry.
        let dummy_fid = mb.register_runtime(
            "user.add_any",
            FunctionSig {
                params: vec![Type::Float(64), Type::Float(64)],
                return_ty: Type::Value,
                ..Default::default()
            },
        );

        let sig = FunctionSig {
            params: vec![Type::Float(64), Type::Float(64)],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Private);
        let a = fb.param(0);
        let b = fb.param(1);
        let v = fb.call(dummy_fid, &[a, b], Type::Value);
        fb.ret(Some(v));
        let func = fb.build();
        mb.add_function(func);
        let module = mb.build();

        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(!result.changed);
    }
}
