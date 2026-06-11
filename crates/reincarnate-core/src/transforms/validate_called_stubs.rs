//! `ValidateCalledStubs` validation pass.
//!
//! Emits a diagnostic for every `Op::Call` that targets an unresolved `_any`
//! stub — i.e. a function whose entry block has zero instructions and a
//! `Return(None)` terminator, and whose `specializations` table is non-empty.
//!
//! This pass runs after all optimization transforms (especially
//! `BuiltinOverloadSelect`) and flags calls that could not be resolved to a
//! typed variant because argument types remained `Unknown`.

use std::collections::HashSet;

use crate::error::CoreError;
use crate::ir::func::FuncId;
use crate::ir::inst::Terminator;
use crate::ir::{Module, Op};
use crate::pipeline::checker::{Diagnostic, DiagnosticCode, RcDiagnostic, Severity};
use crate::pipeline::{Transform, TransformResult};

/// `ValidateCalledStubs` — warns on surviving calls to unresolved `_any` stubs.
pub struct ValidateCalledStubs;

impl Transform for ValidateCalledStubs {
    fn name(&self) -> &str {
        "validate-called-stubs"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn requires(&self) -> &[&str] {
        &["builtin-overload-select", "dead-code-elimination"]
    }

    fn apply(
        &self,
        mut module: Module,
        dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        // 1. Build set of stub FuncIds (functions with specializations but empty body).
        let stub_fids: HashSet<FuncId> = module
            .functions
            .keys()
            .filter(|&fid| {
                let f = &module.functions[fid];
                if f.specializations.is_empty() {
                    return false;
                }
                // Entry block = first block (BlockId(0)).
                let Some(entry) = f.blocks.values().next() else {
                    return false;
                };
                entry.insts.is_empty() && matches!(entry.terminator, Terminator::Return(None))
            })
            .collect();

        // 2. Scan all functions for calls to stubs.
        for func_id in module.functions.keys().collect::<Vec<_>>() {
            if dirty.is_some_and(|d| !d.contains(&func_id)) {
                continue;
            }
            let func = &module.functions[func_id];
            let caller_name = func.name.clone();

            let live_insts: Vec<_> = func
                .blocks
                .values()
                .flat_map(|b| b.insts.iter().copied())
                .collect();

            for inst_id in live_insts {
                if let Op::Call {
                    func: callee_fid, ..
                } = &func.insts[inst_id].op
                {
                    if stub_fids.contains(callee_fid) {
                        let callee_name = module.func_name(*callee_fid).to_string();
                        module.diagnostics.push(Diagnostic {
                            file: caller_name.clone(),
                            line: 0,
                            col: 0,
                            code: DiagnosticCode::Rc(RcDiagnostic::CalledStub),
                            severity: Severity::Error,
                            message: format!(
                                "call to unresolved stub `{}` \u{2014} argument types could not be inferred",
                                callee_name
                            ),
                        });
                    }
                }
            }
        }

        Ok(TransformResult {
            module,
            changed: false,
            changed_funcs: HashSet::new(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::func::FuncId;
    use crate::ir::ty::FunctionSig;
    use crate::ir::{Type, Visibility};
    use crate::pipeline::Transform;
    use crate::transforms::BuiltinOverloadSelect;

    /// Build a module containing one function that calls `<op_name>_any`
    /// with `arg_types.len()` parameters of the given types.
    fn make_module_with_func(op_name: &str, arg_types: &[Type]) -> Module {
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

        // Build a minimal non-empty body for the `_any` stub so that
        // `ValidateCalledStubs` does not treat it as an unresolved stub.
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
        let mut any_fb = FunctionBuilder::new(&func_name, any_sig.clone(), Visibility::Public);
        let fallback = any_fb.create_block();
        any_fb.br(fallback, &[]);
        any_fb.switch_to_block(fallback);
        any_fb.ret(None);
        let built_any = any_fb.build();

        // Register stub name first so the FuncId exists in the runtime registry.
        let any_fid = mb.module_mut().register_runtime(&func_name, any_sig);
        // Replace the empty stub body with the one that has a real entry block.
        let module = mb.module_mut();
        module.functions[any_fid].blocks = built_any.blocks;
        module.functions[any_fid].insts = built_any.insts;
        module.functions[any_fid].value_types = built_any.value_types;
        module.functions[any_fid].entry = built_any.entry;
        module.functions[any_fid].specializations = specializations;

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

        mb.add_function(func);
        mb.build()
    }

    /// Build a module with a manually created empty stub that has specializations,
    /// and a function that calls that stub.
    fn make_module_with_manual_stub() -> Module {
        let sig = FunctionSig {
            params: vec![Type::Value],
            return_ty: Type::Value,
            ..Default::default()
        };

        // Register the stub first so its FuncId is available.
        let mut mb = ModuleBuilder::new("test");
        let stub_id = mb.register_runtime(
            "test.stub_func",
            FunctionSig {
                params: vec![Type::Value],
                return_ty: Type::Value,
                ..Default::default()
            },
        );

        // Create a caller that invokes the stub.
        let mut fb = FunctionBuilder::new("test_fn", sig.clone(), Visibility::Private);
        let a = fb.param(0);
        let v = fb.call(stub_id, &[a], Type::Value);
        fb.ret(Some(v));
        let func = fb.build();

        mb.add_function(func);
        let mut module = mb.build();

        // Add a fake specialization entry so the stub detection triggers.
        module.functions[stub_id]
            .specializations
            .insert(vec![Type::Float(64)], stub_id);
        module
    }

    #[test]
    fn test_calls_to_stub_produce_diagnostic() {
        let module = make_module_with_manual_stub();
        let result = ValidateCalledStubs.apply(module, None).unwrap();

        let called_stub_diags: Vec<_> = result
            .module
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::Rc(RcDiagnostic::CalledStub))
            .collect();
        assert!(
            !called_stub_diags.is_empty(),
            "expected CalledStub diagnostic for unresolved stub"
        );
        assert!(called_stub_diags[0].message.contains("test.stub_func"));
        assert!(!result.changed);
    }

    #[test]
    fn test_any_builtins_with_real_bodies_no_diagnostic() {
        // add_any now has a real dispatch body, so it should NOT be
        // detected as a stub even when called with Unknown args.
        let module = make_module_with_func("add", &[Type::Value, Type::Value]);
        let result = ValidateCalledStubs.apply(module, None).unwrap();

        let called_stub_diags: Vec<_> = result
            .module
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::Rc(RcDiagnostic::CalledStub))
            .collect();
        assert!(
            called_stub_diags.is_empty(),
            "add_any has a real body — should not produce CalledStub diagnostic"
        );
    }

    #[test]
    fn test_calls_to_resolved_overload_no_diagnostic() {
        // Float(64) args — overload select will resolve to add_f64.
        let module = make_module_with_func("add", &[Type::Float(64), Type::Float(64)]);
        let result = BuiltinOverloadSelect.apply(module, None).unwrap();
        assert!(result.changed);

        let result = ValidateCalledStubs.apply(result.module, None).unwrap();
        let called_stub_diags: Vec<_> = result
            .module
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::Rc(RcDiagnostic::CalledStub))
            .collect();
        assert!(
            called_stub_diags.is_empty(),
            "expected no CalledStub diagnostic after successful overload select"
        );
    }

    #[test]
    fn test_non_stub_call_no_diagnostic() {
        // Call a regular (non-stub) function — it has no specializations so no stub diagnostic.
        let mut mb = ModuleBuilder::new("test");
        let callee_fid = mb.register_runtime(
            "user.some_func",
            FunctionSig {
                params: vec![Type::Float(64)],
                return_ty: Type::Float(64),
                ..Default::default()
            },
        );

        let sig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Private);
        let a = fb.param(0);
        let v = fb.call(callee_fid, &[a], Type::Float(64));
        fb.ret(Some(v));
        let func = fb.build();

        mb.add_function(func);
        let module = mb.build();

        let result = ValidateCalledStubs.apply(module, None).unwrap();
        let called_stub_diags: Vec<_> = result
            .module
            .diagnostics
            .iter()
            .filter(|d| d.code == DiagnosticCode::Rc(RcDiagnostic::CalledStub))
            .collect();
        assert!(
            called_stub_diags.is_empty(),
            "expected no CalledStub diagnostic for non-stub function"
        );
    }
}
