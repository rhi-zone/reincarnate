use std::collections::HashMap;

use crate::error::CoreError;
use crate::ir::{Module, Type};
use crate::pipeline::{Transform, TransformResult};
use crate::transforms::call_site_flow::collect_call_site_types;

/// Interprocedural call-site type widening — widens callee parameter types
/// that were narrowed by `ConstraintSolve` when callers pass incompatible types.
///
/// `ConstraintSolve` is intra-procedural: it narrows a `Dynamic` param to a
/// concrete type when the function body constrains it (e.g.
/// `cmp.eq(param, i64_value)` causes the union-find to unify them → param
/// becomes `Int(64)`). This can conflict with what callers actually pass — for
/// example, a function that compares its parameter against a GML instance index
/// (an i64) but is called with `ClassRef` values from every call site.
///
/// This pass detects such conflicts and widens the param back to `Dynamic`,
/// producing an `any` annotation in TypeScript output (which is valid for
/// `any === number` comparisons).
///
/// Design decisions:
/// - Runs AFTER `ConstraintSolve`, which is when conflicting narrowing can occur.
/// - Only widens concrete → Dynamic. Never makes other changes.
/// - If any non-Dynamic caller passes a type that differs from the param type,
///   the param is widened to Dynamic.
/// - Dynamic callers are ignored — they are already compatible with any param
///   type and don't signal a type conflict.
/// - `run_once = true`: prevents oscillation in fixpoint mode. Without this,
///   `ConstraintSolve` would re-narrow in each iteration and this pass would
///   re-widen, producing an infinite loop.
pub struct CallSiteTypeWiden;

impl Transform for CallSiteTypeWiden {
    fn name(&self) -> &str {
        "call-site-type-widen"
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
            .iter()
            .map(|(id, f)| (f.name.clone(), id))
            .collect();

        // Group observations by callee name.
        let mut per_callee: HashMap<String, Vec<(usize, Vec<Type>)>> = HashMap::new();
        for ((name, idx), types) in &observations {
            per_callee
                .entry(name.clone())
                .or_default()
                .push((*idx, types.clone()));
        }

        for (callee_name, params) in &per_callee {
            let func_id = match name_to_id.get(callee_name) {
                Some(&id) => id,
                None => continue, // External function — skip.
            };

            for &(param_idx, ref caller_types) in params {
                let func = &module.functions[func_id];

                // Bounds check: use sig.params length as the canonical count.
                if param_idx >= func.sig.params.len() {
                    continue;
                }

                // Determine the actual param type. ConstraintSolve updates
                // entry-block param `.ty` and `value_types` but NOT `sig.params`,
                // so we must read from the entry block to see post-CS narrowing.
                let entry = func.entry;
                let param_ty = if param_idx < func.blocks[entry].params.len() {
                    func.blocks[entry].params[param_idx].ty.clone()
                } else {
                    func.sig.params[param_idx].clone()
                };

                // Only widen non-Dynamic params — Dynamic is already the widest.
                if param_ty == Type::Dynamic {
                    continue;
                }

                // Widen if any non-Dynamic caller passes a type that differs
                // from the current param type. Dynamic callers are skipped —
                // they don't indicate a type conflict (they're already
                // compatible with any concrete type).
                let should_widen = caller_types
                    .iter()
                    .any(|t| t != &Type::Dynamic && t != &param_ty);

                if should_widen {
                    let func = &mut module.functions[func_id];

                    // Update entry block param type, value_types, and sig
                    // (keep all three in sync — CallSiteTypeFlow writes all
                    // three, and the backend reads from entry block params).
                    let entry = func.entry;
                    if param_idx < func.blocks[entry].params.len() {
                        func.blocks[entry].params[param_idx].ty = Type::Dynamic;
                        let value = func.blocks[entry].params[param_idx].value;
                        func.value_types[value] = Type::Dynamic;
                    }
                    func.sig.params[param_idx] = Type::Dynamic;

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

    fn run(mb: ModuleBuilder) -> TransformResult {
        CallSiteTypeWiden.apply(mb.build()).unwrap()
    }

    // ---- Incompatible caller widens param ----

    /// Param is Int(64) (narrowed by ConstraintSolve), but a caller passes
    /// String → param should be widened back to Dynamic.
    #[test]
    fn widen_when_caller_passes_incompatible_type() {
        let mut mb = ModuleBuilder::new("test");

        // Callee: fn target(x: Int(64)) — as if ConstraintSolve narrowed it
        let callee_sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        // Caller passes a string — incompatible with Int(64).
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller = FunctionBuilder::new("caller", sig, Visibility::Private);
        let s = caller.const_string("hello");
        caller.call("target", &[s], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::Dynamic);
        // Entry block param and value_types should also be widened.
        let entry = target.entry;
        assert_eq!(target.blocks[entry].params[0].ty, Type::Dynamic);
        let val = target.blocks[entry].params[0].value;
        assert_eq!(target.value_types[val], Type::Dynamic);
    }

    // ---- Compatible caller leaves param unchanged ----

    /// Param is String, caller passes String → no widening needed.
    #[test]
    fn no_widen_when_caller_compatible() {
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
        let s = caller.const_string("hello");
        caller.call("target", &[s], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::String);
    }

    // ---- Dynamic caller does not trigger widening ----

    /// Param is Int(64), one caller passes Int(64) and another passes Dynamic.
    /// Dynamic callers are ignored — only concrete-type callers signal conflicts.
    #[test]
    fn dynamic_caller_does_not_widen() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        // Caller A: passes Int(64) — compatible.
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller_a = FunctionBuilder::new("caller_a", sig, Visibility::Private);
        let n = caller_a.const_int(42);
        caller_a.call("target", &[n], Type::Void);
        caller_a.ret(None);
        mb.add_function(caller_a.build());

        // Caller B: passes Dynamic — ignored (not a type conflict).
        let sig_b = FunctionSig {
            params: vec![Type::Dynamic],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller_b = FunctionBuilder::new("caller_b", sig_b, Visibility::Private);
        let p = caller_b.param(0); // Dynamic
        caller_b.call("target", &[p], Type::Void);
        caller_b.ret(None);
        mb.add_function(caller_b.build());

        let result = run(mb);
        assert!(!result.changed, "Dynamic caller should not trigger widening");
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::Int(64));
    }

    // ---- Already Dynamic → no-op ----

    /// If param is already Dynamic, nothing to widen.
    #[test]
    fn already_dynamic_no_op() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Dynamic],
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
        let s = caller.const_string("hello");
        caller.call("target", &[s], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::Dynamic);
    }

    // ---- No callers → no change ----

    #[test]
    fn no_callers_no_change() {
        let mut mb = ModuleBuilder::new("test");

        let callee_sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("target", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        let result = run(mb);
        assert!(!result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::Int(64));
    }

    // ---- ClassRef conflict (the playerIsCharacter scenario) ----

    /// Param is Int(64) (ConstraintSolve narrowed via cmp.eq against an i64),
    /// but callers pass ClassRef values → widen to Dynamic.
    #[test]
    fn classref_caller_widens_int_param() {
        let mut mb = ModuleBuilder::new("test");

        // Callee: fn playerIsCharacter(x: Int(64)) — narrowed by ConstraintSolve
        let callee_sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut callee = FunctionBuilder::new("playerIsCharacter", callee_sig, Visibility::Private);
        callee.ret(None);
        mb.add_function(callee.build());

        // Caller A: passes ClassRef("OAxel")
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut caller_a = FunctionBuilder::new("caller_a", sig.clone(), Visibility::Private);
        let r = caller_a.global_ref("OAxel", Type::ClassRef("OAxel".into()));
        caller_a.call("playerIsCharacter", &[r], Type::Void);
        caller_a.ret(None);
        mb.add_function(caller_a.build());

        // Caller B: passes ClassRef("OFuji")
        let mut caller_b = FunctionBuilder::new("caller_b", sig, Visibility::Private);
        let r = caller_b.global_ref("OFuji", Type::ClassRef("OFuji".into()));
        caller_b.call("playerIsCharacter", &[r], Type::Void);
        caller_b.ret(None);
        mb.add_function(caller_b.build());

        let result = run(mb);
        assert!(result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::Dynamic, "ClassRef callers should widen Int param to Dynamic");
    }

    // ---- Multiple params: widen one, keep one ----

    #[test]
    fn mixed_params_widen_one_keep_one() {
        let mut mb = ModuleBuilder::new("test");

        // target(x: String, y: Int(64))
        let callee_sig = FunctionSig {
            params: vec![Type::String, Type::Int(64)],
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

        // Caller passes (String, Float) — y has incompatible type
        let mut caller = FunctionBuilder::new("caller", sig, Visibility::Private);
        let s = caller.const_string("hi");
        let f = caller.const_float(1.0);
        caller.call("target", &[s, f], Type::Void);
        caller.ret(None);
        mb.add_function(caller.build());

        let result = run(mb);
        assert!(result.changed);
        let target = &result.module.functions[FuncId::new(0)];
        assert_eq!(target.sig.params[0], Type::String); // x compatible, unchanged
        assert_eq!(target.sig.params[1], Type::Dynamic); // y incompatible, widened
    }
}
