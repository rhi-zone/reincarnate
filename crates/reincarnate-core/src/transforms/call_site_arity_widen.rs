use std::collections::HashMap;

use crate::error::CoreError;
use crate::ir::block::BlockParam;
use crate::ir::{Module, Op, Type};
use crate::pipeline::{Transform, TransformResult};

/// Interprocedural call-site arity widening — appends optional `Dynamic`
/// parameters to functions that are called with more arguments than they
/// declare.
///
/// GML uses a loose calling convention: any function can be called with extra
/// arguments beyond its declared parameter list, which are accessible via
/// `argument[N]`. TypeScript does not allow extra arguments (TS2554 "Expected N
/// arguments, but got M"). This pass detects such over-applications and widens
/// the callee signature to accept the extra arguments as optional `Dynamic`
/// parameters with a `Null` default.
///
/// Design decisions:
/// - Only extends, never removes params.
/// - Extra params are typed `Dynamic` with a `Null` default (matching GML
///   semantics: an unset `argument[N]` is the undefined value).
/// - `run_once = true`: call sites are fully observable in one pass; repeating
///   would be a no-op and the pass is not idempotent-safe in fixpoint mode.
/// - Skips functions that declare a rest parameter (`has_rest_param = true`) —
///   those already accept arbitrary extra args.
/// - `Op::Call` args map to callee params at the same index.
/// - `Op::MethodCall` args map to callee params starting at index 1 (index 0
///   is self).
pub struct CallSiteArityWiden;

/// Collect the maximum observed argument count per callee name.
///
/// For `Op::Call`, the arg count is `args.len()`.
/// For `Op::MethodCall`, the arg count is `args.len() + 1` (adding self at [0]).
fn collect_max_arities(module: &Module) -> HashMap<String, usize> {
    let mut max_arities: HashMap<String, usize> = HashMap::new();

    for func in module.functions.values() {
        for block in func.blocks.values() {
            for &inst_id in &block.insts {
                let inst = &func.insts[inst_id];
                match &inst.op {
                    Op::Call {
                        func: callee_name,
                        args,
                    } => {
                        // Skip self-calls.
                        if callee_name == &func.name {
                            continue;
                        }
                        let entry = max_arities.entry(callee_name.clone()).or_insert(0);
                        *entry = (*entry).max(args.len());
                    }
                    Op::MethodCall { method, args, .. } => {
                        // Skip self-calls.
                        if method == &func.name {
                            continue;
                        }
                        // args exclude self (param[0]), so total param count needed
                        // is args.len() + 1.
                        let needed = args.len() + 1;
                        let entry = max_arities.entry(method.clone()).or_insert(0);
                        *entry = (*entry).max(needed);
                    }
                    _ => {}
                }
            }
        }
    }

    max_arities
}

impl Transform for CallSiteArityWiden {
    fn name(&self) -> &str {
        "call-site-arity-widen"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        let max_arities = collect_max_arities(&module);
        let mut changed = false;

        // Build a name → func_id map for write-back.
        let name_to_id: HashMap<String, _> = module
            .functions
            .iter()
            .map(|(id, f)| (f.name.clone(), id))
            .collect();

        for (callee_name, max_arity) in &max_arities {
            let func_id = match name_to_id.get(callee_name) {
                Some(&id) => id,
                None => continue, // External function — skip.
            };

            let func = &module.functions[func_id];

            // If the callee already accepts enough params, nothing to do.
            let current_count = func.sig.params.len();
            if *max_arity <= current_count {
                continue;
            }

            // Don't extend rest-param functions — they already accept any
            // number of args.
            if func.sig.has_rest_param {
                continue;
            }

            let extra = max_arity - current_count;

            // Extend sig.params and sig.defaults.
            let func = &mut module.functions[func_id];
            for _ in 0..extra {
                func.sig.params.push(Type::Dynamic);
                // Ensure defaults vec is long enough, then set None for the
                // new params (they use the Dynamic/null GML default).
                // Extend existing defaults to align with the new param count.
                while func.sig.defaults.len() < func.sig.params.len() - 1 {
                    func.sig.defaults.push(None);
                }
                func.sig.defaults.push(Some(crate::ir::value::Constant::Null));
            }

            // Extend entry block params with matching ValueIds.
            let entry = func.entry;
            for _ in 0..extra {
                let value = func.value_types.push(Type::Dynamic);
                func.blocks[entry].params.push(BlockParam {
                    value,
                    ty: Type::Dynamic,
                });
            }

            changed = true;
        }

        Ok(TransformResult { module, changed })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityRef;
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::ty::FunctionSig;
    use crate::ir::{FuncId, Type, Visibility};

    fn run(mb: ModuleBuilder) -> TransformResult {
        CallSiteArityWiden.apply(mb.build()).unwrap()
    }

    /// Callee declares 1 param; caller passes 2 → widens to 2 params.
    #[test]
    fn basic_arity_widen() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        let caller_sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        let a = caller.const_float(1.0);
        let b = caller.const_float(2.0);
        caller.call("target", &[a, b], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(result.changed);

        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params.len(), 2);
        assert_eq!(target.sig.params[1], Type::Dynamic);
        // New param should have a Null default.
        assert!(target.sig.defaults.len() >= 2);
        assert!(matches!(
            target.sig.defaults[1],
            Some(crate::ir::value::Constant::Null)
        ));
        // Entry block should have 2 params.
        let entry = target.entry;
        assert_eq!(target.blocks[entry].params.len(), 2);
    }

    /// Callee already has enough params → no change.
    #[test]
    fn no_change_when_arity_sufficient() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Dynamic, Type::Dynamic],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        let caller_sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        let a = caller.const_float(1.0);
        let b = caller.const_float(2.0);
        caller.call("target", &[a, b], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params.len(), 2);
    }

    /// Rest-param functions are not extended.
    #[test]
    fn rest_param_not_extended() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Void,
            has_rest_param: true,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        let caller_sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        let a = caller.const_float(1.0);
        let b = caller.const_float(2.0);
        let c = caller.const_float(3.0);
        caller.call("target", &[a, b, c], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params.len(), 1);
    }

    /// Multiple callers — takes the max.
    #[test]
    fn takes_max_across_callers() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        let sig = FunctionSig { params: vec![], return_ty: Type::Void, ..Default::default() };

        // Caller A: 2 args
        let mut caller_a = FunctionBuilder::new("caller_a", sig.clone(), Visibility::Private);
        let a = caller_a.const_float(1.0);
        let b = caller_a.const_float(2.0);
        caller_a.call("target", &[a, b], Type::Void);
        caller_a.ret(None);
        mb.add_function(caller_a.build());

        // Caller B: 3 args
        let mut caller_b = FunctionBuilder::new("caller_b", sig, Visibility::Private);
        let a = caller_b.const_float(1.0);
        let b = caller_b.const_float(2.0);
        let c = caller_b.const_float(3.0);
        caller_b.call("target", &[a, b, c], Type::Void);
        caller_b.ret(None);
        mb.add_function(caller_b.build());

        let result = run(mb);
        assert!(result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params.len(), 3);
    }
}
