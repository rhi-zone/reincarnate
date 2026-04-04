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
        // 1. Build set of stub function names.
        let stub_names: HashSet<String> = module
            .functions
            .values()
            .filter(|f| {
                if f.specializations.is_empty() {
                    return false;
                }
                // Entry block = first block (BlockId(0)).
                let Some(entry) = f.blocks.values().next() else {
                    return false;
                };
                entry.insts.is_empty() && matches!(entry.terminator, Terminator::Return(None))
            })
            .map(|f| f.name.clone())
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
                if let Op::Call { func: fname, .. } = &func.insts[inst_id].op {
                    if stub_names.contains(fname) {
                        module.diagnostics.push(Diagnostic {
                            file: caller_name.clone(),
                            line: 0,
                            col: 0,
                            code: DiagnosticCode::Rc(RcDiagnostic::CalledStub),
                            severity: Severity::Error,
                            message: format!(
                                "call to unresolved stub `{}` \u{2014} argument types could not be inferred",
                                fname
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
    use crate::ir::ty::FunctionSig;
    use crate::ir::{Type, Visibility};
    use crate::pipeline::Transform;
    use crate::transforms::BuiltinOverloadSelect;

    /// Register a minimal `_any` overload stub for testing, with a real dispatch
    /// body so that `ValidateCalledStubs` does not flag it as an empty stub.
    ///
    /// The stub has a non-empty entry block (contains a `coerce` + `call` instruction),
    /// so it passes the "entry block non-empty" check in `ValidateCalledStubs`.
    fn register_test_any_stub_with_body(
        module: &mut Module,
        op_name: &str,
        builtin_name: &str,
        param_count: usize,
    ) {
        use crate::ir::func::Visibility;

        let params = vec![Type::Unknown; param_count];
        let sig = FunctionSig {
            params: params.clone(),
            return_ty: Type::Unknown,
            ..Default::default()
        };
        // Build a minimal non-empty body: coerce first arg and call the typed variant.
        let mut fb =
            FunctionBuilder::new(format!("{op_name}_any"), sig.clone(), Visibility::Public);
        let a = fb.param(0);
        let a_coerced = fb.coerce(a, Type::Float(64));
        let args_coerced: Vec<_> = if param_count == 2 {
            let b = fb.param(1);
            let b_coerced = fb.coerce(b, Type::Float(64));
            vec![a_coerced, b_coerced]
        } else {
            vec![a_coerced]
        };
        let result = fb.call(builtin_name, &args_coerced, Type::Float(64));
        fb.ret(Some(result));
        let built = fb.build();

        let any_id = module.register_runtime(format!("{op_name}_any"), sig);
        module.functions[any_id].blocks = built.blocks;
        module.functions[any_id].insts = built.insts;
        module.functions[any_id].value_types = built.value_types;
        module.functions[any_id].entry = built.entry;

        // Add specializations so BuiltinOverloadSelect can resolve typed calls.
        let typed_fid = module.runtime_registry[builtin_name];
        let typed_args = vec![Type::Float(64); param_count];
        module.functions[any_id]
            .specializations
            .insert(typed_args, typed_fid);
    }

    /// Build a module containing one function that calls `<op_name>_any`
    /// with `arg_types.len()` parameters of the given types.
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
        // Register an _any stub with a real dispatch body for testing.
        register_test_any_stub_with_body(&mut module, op_name, "builtin.add_f64", 2);
        module
    }

    /// Build a module with a manually created empty stub that has specializations,
    /// and a function that calls that stub.
    fn make_module_with_manual_stub() -> Module {
        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        // Create a caller that invokes the stub.
        let mut fb = FunctionBuilder::new("test_fn", sig.clone(), Visibility::Private);
        let a = fb.param(0);
        let v = fb.call("test.stub_func", &[a], Type::Unknown);
        fb.ret(Some(v));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let mut module = mb.build();

        // Register a manual empty stub with specializations (simulating an
        // unresolved _any-style function).
        let stub_id = module.register_runtime(
            "test.stub_func",
            FunctionSig {
                params: vec![Type::Unknown],
                return_ty: Type::Unknown,
                ..Default::default()
            },
        );
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
        let module = make_module_with_func("add", &[Type::Unknown, Type::Unknown]);
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
        // Float(64) args — overload select will resolve to builtin.add_f64.
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
        // Call a regular (non-stub) function.
        let sig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_fn", sig, Visibility::Private);
        let a = fb.param(0);
        let v = fb.call("user.some_func", &[a], Type::Float(64));
        fb.ret(Some(v));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
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
