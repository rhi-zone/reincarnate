use std::collections::{HashMap, HashSet};

use crate::error::CoreError;
use crate::ir::func::FuncId;
use crate::ir::value::ValueId;
use crate::ir::{Function, Module, Op};
use crate::pipeline::{Transform, TransformResult};

use super::util::{substitute_values_in_op, substitute_values_in_terminator};

/// Redundant cast elimination — removes `Cast(v, ty)` entirely when
/// `value_types[v]` already matches `ty`, substituting all uses of the cast
/// result with the source value.
///
/// This runs after type inference refines types so that casts inserted by the
/// frontend (e.g., `as boolean` on a method that already returns Bool) become
/// redundant.
pub struct RedundantCastElimination;

/// Eliminate redundant casts in a single function.
/// Returns true if any changes were made.
fn elim_function(func: &mut Function) -> bool {
    let mut subst: HashMap<ValueId, ValueId> = HashMap::new();
    let mut dead_insts: HashSet<crate::ir::InstId> = HashSet::new();

    // Only examine instructions that are live (referenced from blocks).
    let live_insts: Vec<_> = func
        .blocks
        .values()
        .flat_map(|b| b.insts.iter().copied())
        .collect();

    for inst_id in live_insts {
        if let Op::Cast(value, ref ty, _) = func.insts[inst_id].op {
            if func.value_types[value] == *ty {
                if let Some(result) = func.insts[inst_id].result {
                    subst.insert(result, value);
                    dead_insts.insert(inst_id);
                }
            }
        }
    }

    if subst.is_empty() {
        return false;
    }

    // Resolve transitive chains (e.g. Cast(Cast(x, T), T)).
    loop {
        let mut changed = false;
        let snapshot: Vec<_> = subst.iter().map(|(k, v)| (*k, *v)).collect();
        for (key, target) in snapshot {
            if let Some(&next) = subst.get(&target) {
                if next != subst[&key] {
                    subst.insert(key, next);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    // Transfer names from removed cast results to their sources.
    for &inst_id in &dead_insts {
        if let Some(result) = func.insts[inst_id].result {
            if let Op::Cast(src, ..) = func.insts[inst_id].op {
                let final_src = subst.get(&result).copied().unwrap_or(src);
                if let Some(name) = func.value_names.remove(&result) {
                    func.value_names.entry(final_src).or_insert(name);
                }
            }
        }
    }

    // Apply substitution to all surviving instructions.
    let inst_ids: Vec<_> = func.insts.keys().collect();
    for inst_id in inst_ids {
        if dead_insts.contains(&inst_id) {
            continue;
        }
        substitute_values_in_op(&mut func.insts[inst_id].op, &subst);
    }

    // Substitute in block terminators.
    for block_id in func.blocks.keys().collect::<Vec<_>>() {
        substitute_values_in_terminator(&mut func.blocks[block_id].terminator, &subst);
    }

    // Remove dead instructions from blocks.
    for block_id in func.blocks.keys().collect::<Vec<_>>() {
        func.blocks[block_id]
            .insts
            .retain(|id| !dead_insts.contains(id));
    }

    true
}

impl Transform for RedundantCastElimination {
    fn name(&self) -> &str {
        "redundant-cast-elimination"
    }

    fn requires(&self) -> &[&str] {
        &["constraint-solve-hm", "mem2reg", "constant-folding"]
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
            if elim_function(&mut module.functions[func_id]) {
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

/// Collect the set of instruction IDs that are live (referenced from blocks).
#[cfg(test)]
fn live_inst_ids(func: &crate::ir::Function) -> HashSet<crate::ir::InstId> {
    let mut live = HashSet::new();
    for block in func.blocks.values() {
        for &id in &block.insts {
            live.insert(id);
        }
    }
    live
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::entity::EntityRef;
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::ty::FunctionSig;
    use crate::ir::{FuncId, Type, Visibility};

    // ---- Identity & idempotency tests ----

    /// All casts cross types (Int -> Bool) -> no changes.
    #[test]
    fn identity_no_change() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let val = fb.const_int(1, 64); // Int(64)
        let cast = fb.cast(val, Type::Bool);
        fb.ret(Some(cast));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        let result = RedundantCastElimination.apply(module, None).unwrap();
        assert!(!result.changed);
    }

    /// Redundant cast elimination is idempotent.
    #[test]
    fn idempotent_after_transform() {
        use crate::transforms::util::test_helpers::assert_idempotent;
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let a = fb.const_int(1, 64);
        let b = fb.const_int(1, 64);
        let val = fb.cmp(crate::ir::CmpKind::Eq, a, b);
        let cast = fb.cast(val, Type::Bool);
        fb.ret(Some(cast));
        assert_idempotent(&RedundantCastElimination, fb.build());
    }

    /// Redundant cast (Bool -> Bool) is eliminated: removed from blocks
    /// and all uses substituted with the source value.
    #[test]
    fn redundant_cast_eliminated() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let a = fb.const_int(1, 64);
        let b = fb.const_int(1, 64);
        let val = fb.cmp(crate::ir::CmpKind::Eq, a, b);
        let cast = fb.cast(val, Type::Bool);
        fb.ret(Some(cast));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let result = RedundantCastElimination.apply(module, None).unwrap();
        assert!(result.changed);

        let func = &result.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
        // The redundant cast instruction should not be in any block.
        let live = live_inst_ids(func);
        assert!(
            !live.iter().any(|&id| func.insts[id].result == Some(cast)),
            "redundant cast should be removed from blocks"
        );
        // Return should reference the cmp result directly.
        if let crate::ir::inst::Terminator::Return(Some(v)) = &func.blocks[func.entry].terminator {
            assert_eq!(*v, val, "return should reference cmp result directly");
        } else {
            panic!("expected Return(Some(val))");
        }
    }

    // ---- Edge case tests ----

    /// Coerce vs NullableCoerce -- both kinds tested when redundant.
    #[test]
    fn coerce_redundant_eliminated() {
        let sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0);
        let coerced = fb.coerce(p, Type::Int(64));
        fb.ret(Some(coerced));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        let result = RedundantCastElimination.apply(module, None).unwrap();
        assert!(result.changed, "same-type coerce should be eliminated");
        let func = &result.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
        let live = live_inst_ids(func);
        assert!(!live
            .iter()
            .any(|&id| func.insts[id].result == Some(coerced)));
    }

    /// Chain of same-type casts: Cast(Cast(x, Int), Int) -> both eliminated.
    #[test]
    fn chain_of_casts() {
        let sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0);
        let c1 = fb.cast(p, Type::Int(64));
        let c2 = fb.cast(c1, Type::Int(64));
        fb.ret(Some(c2));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        let result = RedundantCastElimination.apply(module, None).unwrap();
        assert!(result.changed);
        let func = &result.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
        let live = live_inst_ids(func);
        // Both casts should be removed from blocks.
        assert!(!live.iter().any(|&id| func.insts[id].result == Some(c1)));
        assert!(!live.iter().any(|&id| func.insts[id].result == Some(c2)));
        // Return should reference param directly.
        if let crate::ir::inst::Terminator::Return(Some(v)) = &func.blocks[func.entry].terminator {
            assert_eq!(
                *v, p,
                "return should reference param directly after chain elimination"
            );
        } else {
            panic!("expected Return(Some(p))");
        }
    }

    // ---- Adversarial tests ----

    /// Unknown value cast to Int is NOT redundant (type mismatch).
    #[test]
    fn dynamic_to_int_not_redundant() {
        let sig = FunctionSig {
            params: vec![Type::Value],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0); // Unknown
        let cast = fb.cast(p, Type::Int(64));
        fb.ret(Some(cast));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let result = RedundantCastElimination.apply(mb.build(), None).unwrap();
        assert!(!result.changed, "Unknown -> Int cast is NOT redundant");
    }

    /// NullableCoerce(x: Foo, Foo) where source is Instance(Foo) -> redundant, eliminated.
    #[test]
    fn astype_same_struct() {
        let mut mb = ModuleBuilder::new("test");
        let foo_id = mb.intern_type("Foo");
        let sig = FunctionSig {
            params: vec![Type::Instance(foo_id)],
            return_ty: Type::Instance(foo_id),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0);
        // cast() produces NullableCoerce kind.
        let cast = fb.cast(p, Type::Instance(foo_id));
        fb.ret(Some(cast));

        mb.add_function(fb.build());
        let result = RedundantCastElimination.apply(mb.build(), None).unwrap();
        assert!(
            result.changed,
            "NullableCoerce(Foo, Foo) should be redundant"
        );
        let func = &result.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
        let live = live_inst_ids(func);
        assert!(
            !live.iter().any(|&id| func.insts[id].result == Some(cast)),
            "same-struct NullableCoerce should be eliminated"
        );
    }

    /// Coerce(x: Int(32), Int(32)) -> redundant, eliminated.
    #[test]
    fn coerce_same_primitive() {
        let sig = FunctionSig {
            params: vec![Type::Int(32)],
            return_ty: Type::Int(32),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let p = fb.param(0);
        let coerced = fb.coerce(p, Type::Int(32));
        fb.ret(Some(coerced));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let result = RedundantCastElimination.apply(mb.build(), None).unwrap();
        assert!(
            result.changed,
            "Coerce(Int(32), Int(32)) should be redundant"
        );
        let func = &result.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
        let live = live_inst_ids(func);
        assert!(!live
            .iter()
            .any(|&id| func.insts[id].result == Some(coerced)));
    }

    /// Non-redundant cast (Int -> Bool) is left unchanged.
    #[test]
    fn non_redundant_cast_unchanged() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
        let val = fb.const_int(1, 64); // Type::Int(64)
        let cast = fb.cast(val, Type::Bool);
        fb.ret(Some(cast));
        let func = fb.build();

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let result = RedundantCastElimination.apply(module, None).unwrap();
        assert!(!result.changed);

        let func = &result.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
        let live = live_inst_ids(func);
        let cast_still_live = live.iter().any(|&id| func.insts[id].result == Some(cast));
        assert!(cast_still_live, "non-redundant cast should remain");
        let cast_inst = live
            .iter()
            .find(|&&id| func.insts[id].result == Some(cast))
            .map(|&id| &func.insts[id].op);
        assert!(
            matches!(cast_inst, Some(Op::Cast(_, ty, _)) if *ty == Type::Bool),
            "non-redundant cast should remain Cast"
        );
    }
}
