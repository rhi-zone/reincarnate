//! `BuiltinOverloadSelect` transform pass.
//!
//! After `ConstraintSolveHM` resolves operand types, replaces `builtin.xxx_any`
//! calls with the appropriately-typed variant (`builtin.xxx_f64`, `_f32`, `_i32`,
//! `_i64`) and updates the result's `value_types` entry to the concrete return
//! type.
//!
//! # Motivation
//!
//! GML bytecode uses `DataType::Variable` for most arithmetic, so the frontend
//! emits `Op::Call { func: "builtin.add_any", … }`.  The `_any` variants
//! declare no concrete return type (returns `Unknown`), so even
//! `add_any(Float64, Float64)` produces `Unknown`.  This pass replaces those
//! calls once operand types are known from HM inference.
//!
//! # Rules
//!
//! Binary ops (`add`, `sub`, `mul`, `div`, `rem`):
//! - Both operands must have the same concrete type.
//! - `Float(64)` → `_f64`, `Float(32)` → `_f32`, `Int(32)` → `_i32`, `Int(64)` → `_i64`.
//! - If operands disagree, are `Unknown`, or have a type not in the map → leave unchanged.
//!
//! Unary op (`neg`):
//! - Single operand, same type map.
//!
//! Result type is set to the matched concrete type.

use std::collections::HashSet;

use crate::error::CoreError;
use crate::ir::func::FuncId;
use crate::ir::{Function, Module, Op, Type};
use crate::pipeline::{Transform, TransformResult};

/// `BuiltinOverloadSelect` — replaces `builtin.xxx_any` calls with typed variants.
pub struct BuiltinOverloadSelect;

/// Suffix appended to the base op name for a given operand type.
fn type_suffix(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::Float(64) => Some("f64"),
        Type::Float(32) => Some("f32"),
        Type::Int(32) => Some("i32"),
        Type::Int(64) => Some("i64"),
        _ => None,
    }
}

/// Try to select a typed overload for one instruction in `func`.
///
/// Returns `true` if the instruction was rewritten.
fn try_select(func: &mut Function, inst_id: crate::ir::InstId) -> bool {
    // Clone the op to avoid borrowing issues while we mutate value_types.
    let op = func.insts[inst_id].op.clone();

    let (base_op, new_func_name, result_ty) = match &op {
        Op::Call { func: fname, args } => {
            // Must be a "builtin." call ending with "_any".
            let Some(rest) = fname.strip_prefix("builtin.") else {
                return false;
            };
            let Some(op_name) = rest.strip_suffix("_any") else {
                return false;
            };

            match op_name {
                "add" | "sub" | "mul" | "div" | "rem" => {
                    if args.len() != 2 {
                        return false;
                    }
                    let ty_a = func.value_types[args[0]].clone();
                    let ty_b = func.value_types[args[1]].clone();
                    if ty_a != ty_b {
                        return false;
                    }
                    let Some(suffix) = type_suffix(&ty_a) else {
                        return false;
                    };
                    let new_name = format!("builtin.{}_{}", op_name, suffix);
                    (op_name.to_string(), new_name, ty_a)
                }
                "neg" => {
                    if args.len() != 1 {
                        return false;
                    }
                    let ty_a = func.value_types[args[0]].clone();
                    let Some(suffix) = type_suffix(&ty_a) else {
                        return false;
                    };
                    let new_name = format!("builtin.neg_{}", suffix);
                    ("neg".to_string(), new_name, ty_a)
                }
                _ => return false,
            }
        }
        _ => return false,
    };

    let _ = base_op; // consumed above

    // Update the instruction's func name.
    if let Op::Call { func: fname, .. } = &mut func.insts[inst_id].op {
        *fname = new_func_name;
    }

    // Update the result's value type.
    if let Some(result_vid) = func.insts[inst_id].result {
        func.value_types[result_vid] = result_ty;
    }

    true
}

/// Apply overload selection to all live instructions in a function.
///
/// Returns `true` if any instruction was rewritten.
fn select_function(func: &mut Function) -> bool {
    let live_insts: Vec<_> = func
        .blocks
        .values()
        .flat_map(|b| b.insts.iter().copied())
        .collect();

    let mut changed = false;
    for inst_id in live_insts {
        if try_select(func, inst_id) {
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
            if select_function(&mut module.functions[func_id]) {
                changed_funcs.insert(func_id);
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

    /// Build a module containing one function that calls `builtin.<op_name>_any`
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
        let func_name = format!("builtin.{}_any", op_name);
        let call_result = fb.call(func_name, &args, Type::Unknown);
        fb.ret(Some(call_result));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        mb.build()
    }

    fn first_builtin_call_id(module: &Module) -> crate::ir::InstId {
        let func_id = FuncId::new(Module::NUM_CORE_BUILTINS);
        let func = &module.functions[func_id];
        let live: Vec<_> = func
            .blocks
            .values()
            .flat_map(|b| b.insts.iter().copied())
            .collect();
        live.into_iter()
            .find(|&id| {
                matches!(&func.insts[id].op, Op::Call { func: f, .. } if f.starts_with("builtin."))
            })
            .expect("no builtin Call instruction found")
    }

    #[test]
    fn add_f64_both_operands_f64() {
        let module = make_module_with_func("add", &[Type::Float(64), Type::Float(64)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(result.changed);

        let func_id = FuncId::new(Module::NUM_CORE_BUILTINS);
        let inst_id = first_builtin_call_id(&result.module);
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
        let inst_id = first_builtin_call_id(&result.module);
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
        let inst_id = first_builtin_call_id(&result.module);
        let func = &result.module.functions[func_id];

        let Op::Call { func: fname, .. } = &func.insts[inst_id].op else {
            panic!()
        };
        assert_eq!(fname, "builtin.add_any");
    }

    #[test]
    fn unknown_operand_left_unchanged() {
        let module = make_module_with_func("mul", &[Type::Unknown, Type::Unknown]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(!result.changed);

        let func_id = FuncId::new(Module::NUM_CORE_BUILTINS);
        let inst_id = first_builtin_call_id(&result.module);
        let func = &result.module.functions[func_id];

        let Op::Call { func: fname, .. } = &func.insts[inst_id].op else {
            panic!()
        };
        assert_eq!(fname, "builtin.mul_any");
    }

    #[test]
    fn sub_i64_both_i64() {
        let module = make_module_with_func("sub", &[Type::Int(64), Type::Int(64)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(result.changed);

        let func_id = FuncId::new(Module::NUM_CORE_BUILTINS);
        let inst_id = first_builtin_call_id(&result.module);
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
