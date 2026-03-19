use std::collections::HashMap;

use crate::error::CoreError;
use crate::ir::{Function, Module, Op, Type, ValueId};
use crate::pipeline::{Transform, TransformResult};

/// Interprocedural call-site type narrowing — collects argument types from all
/// call sites and narrows callee parameter types that are still `Unknown`.
///
/// This pass bridges the gap between intra-function `TypeInference` (which
/// refines types within a function) and `ConstraintSolve` (which flows callee
/// param types into callers, but not the reverse). By observing what types
/// callers actually pass, we can narrow `Unknown` params to concrete types.
///
/// Design decisions:
/// - Only narrows `Unknown` → concrete. Never overrides an already-concrete
///   type (TypeInference's intra-function evidence is more reliable).
/// - If callers disagree on types, the param stays `Unknown` (no union types).
/// - `CallIndirect` is skipped (no name to resolve).
/// - `SystemCall` is skipped (runtime stubs have known signatures).
/// - Self-calls (recursive) are skipped to avoid circular reasoning.
pub struct CallSiteTypeFlow;

/// Collected argument type observations: `(callee_name, param_index) → Vec<Type>`.
///
/// A `Unknown` entry means a caller passed an unresolved value — this prevents
/// narrowing because Unknown means "could be anything at runtime."
pub(crate) type Observations = HashMap<(String, usize), Vec<Type>>;

/// Collect argument types from all call sites in the module.
///
/// Shared by `CallSiteTypeFlow` (narrows Unknown params) and
/// `CallSiteTypeWiden` (widens params whose ConstraintSolve-narrowed type
/// conflicts with what callers actually pass).
pub(crate) fn collect_call_site_types(module: &Module) -> Observations {
    let mut observations: Observations = HashMap::new();

    for (fid, func) in module.functions.iter() {
        let caller_name = module.func_name(fid);
        for block in func.blocks.values() {
            for &inst_id in &block.insts {
                let inst = &func.insts[inst_id];
                match &inst.op {
                    Op::Call {
                        func: callee_name,
                        args,
                    } => {
                        // Skip self-calls (recursive).
                        if callee_name == caller_name {
                            continue;
                        }
                        for (i, &arg) in args.iter().enumerate() {
                            let ty = &func.value_types[arg];
                            observations
                                .entry((callee_name.clone(), i))
                                .or_default()
                                .push(ty.clone());
                        }
                    }
                    Op::MethodCall { method, args, .. } => {
                        // Skip self-calls.
                        if method == caller_name {
                            continue;
                        }
                        // MethodCall args exclude the receiver — args[0] is
                        // the first explicit argument, mapping to param[1]
                        // (param[0] is `self`). We record against param
                        // index i+1 so that write-back aligns with the
                        // callee's sig.params which includes self at [0].
                        for (i, &arg) in args.iter().enumerate() {
                            let ty = &func.value_types[arg];
                            observations
                                .entry((method.clone(), i + 1))
                                .or_default()
                                .push(ty.clone());
                        }
                    }
                    // Other ops don't produce call sites — only Call and MethodCall do.
                    _ => {}
                }
            }
        }
    }

    observations
}

/// Check if a parameter value is used as a collection in the function body —
/// i.e. via `GetIndex(param, _)` or `GetField(param, "length")`. If so,
/// narrowing it to a non-collection type (e.g. Float(64)) would cause
/// `.length` / `[i]` to produce type errors in the emitted code.
fn param_used_as_collection(func: &Function, param_value: ValueId) -> bool {
    let tracked: Vec<ValueId> = vec![param_value];

    for block in func.blocks.values() {
        for &inst_id in &block.insts {
            let inst = &func.insts[inst_id];
            match &inst.op {
                Op::GetIndex { collection, .. } if tracked.contains(collection) => {
                    return true;
                }
                Op::GetField { object, field } if tracked.contains(object) => {
                    if field == "length" {
                        return true;
                    }
                }
                // GML's `array_length(arr)` is emitted as `arr.length` in TypeScript.
                // In the IR it's a Call, not a GetField — check explicitly so that
                // narrowing to Float(64) is suppressed when the body calls array_length.
                Op::Call { func: fname, args } if fname == "array_length" => {
                    if args.first().map(|a| tracked.contains(a)).unwrap_or(false) {
                        return true;
                    }
                }
                _ => {}
            }
        }
    }
    false
}

/// Narrow a set of observed types to a single type, or `None` if they disagree
/// or if any caller passes `Unknown` (meaning "could be anything at runtime").
fn narrow(types: &[Type]) -> Option<Type> {
    if types.is_empty() {
        return None;
    }
    // If any caller passes Unknown, we can't narrow — the param genuinely
    // receives unknown types at runtime.
    if types.contains(&Type::Unknown) {
        return None;
    }
    // ClassRef callers block narrowing. ClassRef values represent class constructors
    // (`typeof ClassName` in TypeScript), which are incompatible with numeric or
    // other concrete types the callee body might expect. Narrowing a callee param
    // to ClassRef would cause type errors whenever that param is used in non-class
    // contexts — the param type should stay Unknown.
    if types.iter().any(|t| matches!(t, Type::ClassRef(_))) {
        return None;
    }
    let first = &types[0];
    if types.iter().all(|t| t == first) {
        Some(first.clone())
    } else {
        // Callers disagree — leave as Unknown (no union types for now).
        None
    }
}

impl Transform for CallSiteTypeFlow {
    fn name(&self) -> &str {
        "call-site-type-flow"
    }

    fn requires(&self) -> &[&str] {
        &["type-inference"]
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        let observations = collect_call_site_types(&module);
        let mut changed = false;

        // Build a name → func_id map for write-back.
        let name_to_id: HashMap<String, _> = module
            .functions
            .keys()
            .map(|id| (module.func_name(id).to_string(), id))
            .collect();

        // For each observation, try to narrow the callee's param type.
        // Group observations by callee name first to avoid repeated lookups.
        let mut per_callee: HashMap<String, Vec<(usize, Vec<Type>)>> = HashMap::new();
        for ((name, idx), types) in &observations {
            per_callee
                .entry(name.clone())
                .or_default()
                .push((idx.to_owned(), types.clone()));
        }

        for (callee_name, params) in &per_callee {
            let func_id = match name_to_id.get(callee_name) {
                Some(&id) => id,
                None => continue, // External function — skip.
            };

            for &(param_idx, ref types) in params {
                let func = &module.functions[func_id];

                // Bounds check.
                if param_idx >= func.sig.params.len() {
                    continue;
                }

                // Only narrow Unknown params.
                if func.sig.params[param_idx] != Type::Unknown {
                    continue;
                }

                if let Some(narrowed) = narrow(types) {
                    // Don't narrow if the body uses this param as a collection
                    // (GetIndex, .length) — narrowing to e.g. Float(64) would
                    // break those accesses.
                    if !matches!(&narrowed, Type::Array(_)) {
                        let entry = func.entry;
                        if param_idx < func.blocks[entry].params.len() {
                            let param_val = func.blocks[entry].params[param_idx].value;
                            if param_used_as_collection(func, param_val) {
                                continue;
                            }
                        }
                    }

                    let func = &mut module.functions[func_id];

                    // Update signature.
                    func.sig.params[param_idx] = narrowed.clone();

                    // Update entry block param type.
                    let entry = func.entry;
                    if param_idx < func.blocks[entry].params.len() {
                        func.blocks[entry].params[param_idx].ty = narrowed.clone();
                        let value = func.blocks[entry].params[param_idx].value;
                        func.value_types[value] = narrowed;
                    }

                    changed = true;
                }
            }
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

    /// Helper: build a module, apply CallSiteTypeFlow, return result.
    fn run(mb: ModuleBuilder) -> TransformResult {
        CallSiteTypeFlow.apply(mb.build()).unwrap()
    }

    // ---- Basic narrowing ----

    /// Caller passes String to callee's Unknown param → narrows to String.
    #[test]
    fn basic_narrowing() {
        let mut mb = ModuleBuilder::new("test");

        // Callee: fn target(x: Unknown) → Void
        let callee_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        // Caller: fn caller() → Void; calls target(string_val)
        let caller_sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", caller_sig, Visibility::Private);
        let s = caller.const_string("hello");
        caller.call("target", &[s], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(result.changed);

        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::String);

        // Entry block param and value_types should also be updated.
        let entry = target.entry;
        assert_eq!(target.blocks[entry].params[0].ty, Type::String);
        let val = target.blocks[entry].params[0].value;
        assert_eq!(target.value_types[val], Type::String);
    }

    // ---- Multiple callers agree ----

    /// Two callers both pass Float(64) → narrows to Float(64).
    #[test]
    fn multiple_callers_agree() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        // Caller A
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller_a = FunctionBuilder::new("caller_a", sig.clone(), Visibility::Private);
        let v = caller_a.const_float(1.0);
        caller_a.call("target", &[v], Type::Void);
        caller_a.ret(None);
        mb.add_function(caller_a.build());

        // Caller B
        let mut caller_b = FunctionBuilder::new("caller_b", sig, Visibility::Private);
        let v = caller_b.const_float(2.0);
        caller_b.call("target", &[v], Type::Void);
        caller_b.ret(None);
        mb.add_function(caller_b.build());

        let result = run(mb);
        assert!(result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::Float(64));
    }

    // ---- Multiple callers disagree ----

    /// One passes String, another Float(64) → stays Unknown.
    #[test]
    fn multiple_callers_disagree() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller_a = FunctionBuilder::new("caller_a", sig.clone(), Visibility::Private);
        let v = caller_a.const_string("hi");
        caller_a.call("target", &[v], Type::Void);
        caller_a.ret(None);
        mb.add_function(caller_a.build());

        let mut caller_b = FunctionBuilder::new("caller_b", sig, Visibility::Private);
        let v = caller_b.const_float(2.0);
        caller_b.call("target", &[v], Type::Void);
        caller_b.ret(None);
        mb.add_function(caller_b.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::Unknown);
    }

    // ---- No callers ----

    /// Function with Unknown params but no call sites → stays Unknown.
    #[test]
    fn no_callers() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::Unknown);
    }

    // ---- Already typed ----

    /// Param is String from TypeInference, caller passes Float(64) → stays String.
    #[test]
    fn already_typed_not_overridden() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::String],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", sig, Visibility::Private);
        let v = caller.const_float(1.0);
        caller.call("target", &[v], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::String);
    }

    // ---- MethodCall ----

    /// Method call narrows method's params (skipping receiver at index 0).
    #[test]
    fn method_call_narrows_params() {
        let mut mb = ModuleBuilder::new("test");

        // Method: fn Foo::bar(self: Struct(Foo), x: Unknown) → Void
        let method_sig = FunctionSig {
            params: vec![Type::Struct("Foo".into()), Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut method = FunctionBuilder::new("Foo::bar", method_sig, Visibility::Private);
        method.ret(None);
        mb.add_function(method.build());

        // Caller calls receiver.bar(int_val)
        let sig = FunctionSig {
            params: vec![Type::Struct("Foo".into())],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", sig, Visibility::Private);
        let recv = caller.param(0);
        let v = caller.const_int(42);
        caller.call_method(recv, "Foo::bar", &[v], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(result.changed);
        let method = &result.module.functions[FuncId::new(0)];
        // param[0] (self) should be untouched, param[1] should be narrowed.
        assert_eq!(method.sig.params[0], Type::Struct("Foo".into()));
        assert_eq!(method.sig.params[1], Type::Int(64));
    }

    // ---- Self-call (recursive) ----

    /// Recursive function doesn't use its own Unknown params as evidence.
    #[test]
    fn self_call_ignored() {
        let mut mb = ModuleBuilder::new("test");

        let sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut func = FunctionBuilder::new("recurse", sig, Visibility::Private);
        let p = func.param(0); // Unknown
        func.call("recurse", &[p], Type::Void);
        func.ret(None);
        mb.add_function(func.build());

        let result = run(mb);
        // Self-call passes Unknown (which is skipped) and is also a self-call.
        // Either way, no narrowing.
        assert!(!result.changed);
        let f = &result.module.functions[FuncId::new(0)];
        assert_eq!(f.sig.params[0], Type::Unknown);
    }

    // ---- Idempotent ----

    /// Running the pass twice produces no changes on second run.
    #[test]
    fn idempotent() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", sig, Visibility::Private);
        let v = caller.const_string("hello");
        caller.call("target", &[v], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let r1 = CallSiteTypeFlow.apply(mb.build()).unwrap();
        assert!(r1.changed);
        let r2 = CallSiteTypeFlow.apply(r1.module).unwrap();
        assert!(!r2.changed, "second run should report no changes");
    }

    // ---- Unknown arg prevents narrowing ----

    /// A caller passing Unknown prevents narrowing — Unknown means "could be
    /// anything at runtime", so we can't safely narrow to a specific type.
    #[test]
    fn dynamic_arg_prevents_narrowing() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        // Caller A passes String.
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller_a = FunctionBuilder::new("caller_a", sig.clone(), Visibility::Private);
        let v = caller_a.const_string("hello");
        caller_a.call("target", &[v], Type::Void);
        caller_a.ret(None);
        mb.add_function(caller_a.build());

        // Caller B passes Unknown (its own Unknown param).
        let sig_b = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller_b = FunctionBuilder::new("caller_b", sig_b, Visibility::Private);
        let p = caller_b.param(0); // Unknown — prevents narrowing
        caller_b.call("target", &[p], Type::Void);
        caller_b.ret(None);
        mb.add_function(caller_b.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        // Unknown arg prevents narrowing — stays Unknown.
        assert_eq!(target.sig.params[0], Type::Unknown);
    }

    // ---- Multiple params ----

    /// Multiple params: one narrows, another stays Unknown due to disagreement.
    #[test]
    fn multiple_params_mixed() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Unknown, Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };

        // Caller A: target(string, int)
        let mut caller_a = FunctionBuilder::new("caller_a", sig.clone(), Visibility::Private);
        let s = caller_a.const_string("hi");
        let n = caller_a.const_int(1);
        caller_a.call("target", &[s, n], Type::Void);
        caller_a.ret(None);
        mb.add_function(caller_a.build());

        // Caller B: target(string, float) — param 0 agrees, param 1 disagrees
        let mut caller_b = FunctionBuilder::new("caller_b", sig, Visibility::Private);
        let s = caller_b.const_string("bye");
        let f = caller_b.const_float(2.0);
        caller_b.call("target", &[s, f], Type::Void);
        caller_b.ret(None);
        mb.add_function(caller_b.build());

        let result = run(mb);
        assert!(result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::String); // Agreed.
        assert_eq!(target.sig.params[1], Type::Unknown); // Disagreed.
    }

    // ---- Collection body usage prevents narrowing ----

    /// Callers all pass Float(64), but body uses GetIndex on param → stays Unknown.
    #[test]
    fn collection_body_usage_prevents_narrowing() {
        let mut mb = ModuleBuilder::new("test");

        // Callee: fn target(arr: Unknown) → Void; body does GetIndex(arr, 0)
        let callee_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        let arr = callee.param(0);
        let idx = callee.const_int(0);
        callee.get_index(arr, idx, Type::Unknown);
        callee.ret(None);
        mb.add_function(callee.build());

        // Caller passes Float(64).
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", sig, Visibility::Private);
        let v = caller.const_float(1.0);
        caller.call("target", &[v], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        // Body uses param as collection → stays Unknown despite callers agreeing.
        assert_eq!(target.sig.params[0], Type::Unknown);
    }

    /// Callers all pass Float(64), but body uses GetField "length" on param → stays Unknown.
    #[test]
    fn length_access_prevents_narrowing() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        let arr = callee.param(0);
        callee.get_field(arr, "length", Type::Float(64));
        callee.ret(None);
        mb.add_function(callee.build());

        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", sig, Visibility::Private);
        let v = caller.const_float(42.0);
        caller.call("target", &[v], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::Unknown);
    }
}
