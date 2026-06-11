use std::collections::{HashMap, HashSet};

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::block::BlockParam;
use reincarnate_core::ir::func::FuncId;
use reincarnate_core::ir::{Module, Op, Type};
use reincarnate_core::pipeline::{PureIrPass, Transform, TransformResult};

/// Interprocedural call-site arity widening — appends optional `Unknown`
/// parameters to functions that are called with more arguments than they
/// declare.
///
/// GML uses a loose calling convention: any function can be called with extra
/// arguments beyond its declared parameter list, which are accessible via
/// `argument[N]`. TypeScript does not allow extra arguments (TS2554 "Expected N
/// arguments, but got M"). This pass detects such over-applications and widens
/// the callee signature to accept the extra arguments as optional `Unknown`
/// parameters with a `Null` default.
///
/// Design decisions:
/// - Only extends, never removes params.
/// - Extra params are typed `Unknown` with a `Null` default (matching GML
///   semantics: an unset `argument[N]` is the undefined value).
/// - `run_once = true`: call sites are fully observable in one pass; repeating
///   would be a no-op and the pass is not idempotent-safe in fixpoint mode.
/// - Skips functions that declare a rest parameter (`has_rest_param = true`) —
///   those already accept arbitrary extra args.
/// - `Op::Call` args map to callee params at the same index.
/// - `Op::MethodCall` args map to callee params starting at index 1 (index 0
///   is self).
pub struct CallSiteArityWiden;

/// Collect the maximum observed argument count per callee.
///
/// Returns two maps:
/// - : for , keyed by FuncId, arg count =
/// - : for , keyed by method name, arg count =
fn collect_max_arities(module: &Module) -> (HashMap<FuncId, usize>, HashMap<String, usize>) {
    let mut direct_arities: HashMap<FuncId, usize> = HashMap::new();
    let mut method_arities: HashMap<String, usize> = HashMap::new();

    for (caller_fid, func) in module.functions.iter() {
        for block in func.blocks.values() {
            for &inst_id in &block.insts {
                let inst = &func.insts[inst_id];
                match &inst.op {
                    Op::Call {
                        func: callee_fid,
                        args,
                    } => {
                        // Skip self-calls.
                        if *callee_fid == caller_fid {
                            continue;
                        }
                        let entry = direct_arities.entry(*callee_fid).or_insert(0);
                        *entry = (*entry).max(args.len());
                    }
                    Op::MethodCall { method, args, .. } => {
                        // Skip self-calls.
                        if method == module.name_table.func_name(caller_fid) {
                            continue;
                        }
                        // args exclude self (param[0]), so total param count needed
                        // is args.len() + 1.
                        let needed = args.len() + 1;
                        let entry = method_arities.entry(method.clone()).or_insert(0);
                        *entry = (*entry).max(needed);
                    }
                    _ => {}
                }
            }
        }
    }

    (direct_arities, method_arities)
}

/// Widen a single function's entry params and sig to accept `max_arity` args.
/// Returns `true` if any change was made.
fn widen_function(module: &mut Module, func_id: FuncId, max_arity: usize) -> bool {
    let func = &module.functions[func_id];

    // If the callee already accepts enough params, nothing to do.
    let current_count = func.sig.params.len();
    if max_arity <= current_count {
        return false;
    }

    // Don't extend rest-param functions — they already accept any
    // number of args.
    if func.sig.has_rest_param {
        return false;
    }

    let extra = max_arity - current_count;

    // Extra params have no type information — the call sites provide
    // untyped GML values and no constraint can be formed.  Unknown is
    // correct here (genuine opacity, not an inference gap).
    let fresh_types: Vec<Type> = vec![Type::Value; extra];

    // Extend sig.params and sig.defaults.
    let func = &mut module.functions[func_id];
    for ty in &fresh_types {
        func.sig.params.push(ty.clone());
        // Ensure defaults vec is long enough, then set None for the
        // new params (they use the Unknown/null GML default).
        // Extend existing defaults to align with the new param count.
        while func.sig.defaults.len() < func.sig.params.len() - 1 {
            func.sig.defaults.push(None);
        }
        func.sig
            .defaults
            .push(Some(reincarnate_core::ir::value::Constant::Null));
    }

    // Extend entry block params with matching ValueIds.
    let entry = func.entry;
    for ty in fresh_types {
        let value = func.value_types.push(ty.clone());
        func.blocks[entry].params.push(BlockParam { value, ty });
    }
    true
}

impl Transform for CallSiteArityWiden {
    fn name(&self) -> &str {
        "call-site-arity-widen"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(
        &self,
        mut module: Module,
        _dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        let (direct_arities, method_arities) = collect_max_arities(&module);
        let mut changed_funcs: HashSet<FuncId> = HashSet::new();

        // Process direct calls (Op::Call with FuncId).
        for (func_id, max_arity) in &direct_arities {
            if widen_function(&mut module, *func_id, *max_arity) {
                changed_funcs.insert(*func_id);
            }
        }

        // Build a name → func_id map for method call write-back.
        let name_to_id: HashMap<String, _> = module
            .functions
            .iter()
            .map(|(id, f)| (f.name.clone(), id))
            .collect();

        for (callee_name, max_arity) in &method_arities {
            let func_id = match name_to_id.get(callee_name) {
                Some(&id) => id,
                None => continue, // External function — skip.
            };
            if widen_function(&mut module, func_id, *max_arity) {
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

impl PureIrPass for CallSiteArityWiden {}

#[cfg(test)]
mod tests {
    use super::*;
    use reincarnate_core::ir::builder::{FunctionBuilder, ModuleBuilder};
    use reincarnate_core::ir::ty::FunctionSig;
    use reincarnate_core::ir::{Type, Visibility};
    use std::collections::HashMap;

    fn run(mb: ModuleBuilder) -> TransformResult {
        CallSiteArityWiden.apply(mb.build(), None).unwrap()
    }

    /// Callee declares 1 param; caller passes 2 → widens to 2 params.
    #[test]
    fn basic_arity_widen() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Value],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        let target_fid = mb.add_function(callee.build());

        let caller_sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        caller.set_registry(HashMap::from([("target".to_string(), target_fid)]));
        let a = caller.const_float(1.0);
        let b = caller.const_float(2.0);
        caller.call_named("target", &[a, b], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(result.changed);

        let target = &result.module.functions[target_fid];
        assert_eq!(target.sig.params.len(), 2);
        assert_eq!(target.sig.params[1], Type::Value);
        // New param should have a Null default.
        assert!(target.sig.defaults.len() >= 2);
        assert!(matches!(
            target.sig.defaults[1],
            Some(reincarnate_core::ir::value::Constant::Null)
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
            params: vec![Type::Value, Type::Value],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        let target_fid = mb.add_function(callee.build());

        let caller_sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        caller.set_registry(HashMap::from([("target".to_string(), target_fid)]));
        let a = caller.const_float(1.0);
        let b = caller.const_float(2.0);
        caller.call_named("target", &[a, b], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[target_fid];
        assert_eq!(target.sig.params.len(), 2);
    }

    /// Rest-param functions are not extended.
    #[test]
    fn rest_param_not_extended() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Value],
            return_ty: Type::Void,
            has_rest_param: true,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        let target_fid = mb.add_function(callee.build());

        let caller_sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        caller.set_registry(HashMap::from([("target".to_string(), target_fid)]));
        let a = caller.const_float(1.0);
        let b = caller.const_float(2.0);
        let c = caller.const_float(3.0);
        caller.call_named("target", &[a, b, c], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[target_fid];
        assert_eq!(target.sig.params.len(), 1);
    }

    /// Multiple callers — takes the max.
    #[test]
    fn takes_max_across_callers() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Value],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        let target_fid = mb.add_function(callee.build());

        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };

        // Caller A: 2 args
        let mut caller_a = FunctionBuilder::new("caller_a", sig.clone(), Visibility::Private);
        caller_a.set_registry(HashMap::from([("target".to_string(), target_fid)]));
        let a = caller_a.const_float(1.0);
        let b = caller_a.const_float(2.0);
        caller_a.call_named("target", &[a, b], Type::Void);
        caller_a.ret(None);
        mb.add_function(caller_a.build());

        // Caller B: 3 args
        let mut caller_b = FunctionBuilder::new("caller_b", sig, Visibility::Private);
        caller_b.set_registry(HashMap::from([("target".to_string(), target_fid)]));
        let a = caller_b.const_float(1.0);
        let b = caller_b.const_float(2.0);
        let c = caller_b.const_float(3.0);
        caller_b.call_named("target", &[a, b, c], Type::Void);
        caller_b.ret(None);
        mb.add_function(caller_b.build());

        let result = run(mb);
        assert!(result.changed);
        let target = &result.module.functions[target_fid];
        assert_eq!(target.sig.params.len(), 3);
    }
}
