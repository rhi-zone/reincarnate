//! Cross-pass interaction tests — verify that passes compose correctly.

use crate::entity::EntityRef;
use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
use crate::ir::ty::FunctionSig;
use crate::ir::{CmpKind, FuncId, Module, Op, Type, Visibility};
use crate::pipeline::{PassConfig, Transform};
use crate::transforms::util::test_helpers::assert_well_formed;
use crate::transforms::{
    CfgSimplify, ConstantFolding, ConstraintSolveHM, DeadCodeElimination, Mem2Reg,
    RedundantCastElimination,
};

/// ConstantFolding folds `1 + 2` → `3`, leaving the original Const(1) and Const(2)
/// dead. DCE should remove them.
#[test]
fn const_fold_then_dce() {
    let sig = FunctionSig {
        params: vec![],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
    let a = fb.const_int(1, 64);
    let b = fb.const_int(2, 64);
    let sum = fb.add(a, b);
    fb.ret(Some(sum));
    let func = fb.build();

    let mut mb = ModuleBuilder::new("test");
    mb.add_function(func);
    let module = mb.build();

    // Fold constants.
    let r1 = ConstantFolding.apply(module, None).unwrap();
    assert!(r1.changed, "should fold 1+2");

    // The original Const(1) and Const(2) are now dead. DCE removes them.
    let r2 = DeadCodeElimination.apply(r1.module, None).unwrap();
    assert!(r2.changed, "DCE should remove dead constants after folding");

    let func = &r2.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
    assert_well_formed(func);

    // Only the folded Const(3) and Return should remain.
    let live_ops: Vec<_> = func.blocks[func.entry]
        .insts
        .iter()
        .map(|&id| &func.insts[id].op)
        .collect();
    assert!(
        live_ops
            .iter()
            .filter(|op| matches!(op, Op::Const(_)))
            .count()
            <= 1,
        "at most one constant should survive after fold+DCE"
    );
}

/// ConstraintSolveHM infers that a Cmp result is Bool (from the Cmp constraint),
/// making a Cast(v, Bool) redundant. RedundantCastElim should then eliminate it
/// entirely, substituting all uses with the source value.
#[test]
fn hm_solver_then_red_cast_elim() {
    let sig = FunctionSig {
        params: vec![Type::Int(64)],
        return_ty: Type::Bool,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
    let p = fb.param(0);
    let zero = fb.const_int(0, 64);
    let cmp_result = fb.cmp(CmpKind::Eq, p, zero); // produces Bool
    let cast = fb.cast(cmp_result, Type::Bool); // redundant: Bool -> Bool
    fb.ret(Some(cast));
    let func = fb.build();

    let mut mb = ModuleBuilder::new("test");
    mb.add_function(func);
    let module = mb.build();

    // HM solver should confirm cmp_result is Bool.
    let r1 = ConstraintSolveHM.apply(module, None).unwrap();

    // Redundant cast elimination should eliminate Cast(Bool, Bool) entirely.
    let r2 = RedundantCastElimination.apply(r1.module, None).unwrap();
    assert!(r2.changed, "cast should become redundant after HM solver");

    let func = &r2.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
    assert_well_formed(func);
    // The cast instruction should not appear in any block.
    let live: std::collections::HashSet<_> = func
        .blocks
        .values()
        .flat_map(|b| b.insts.iter().copied())
        .collect();
    assert!(
        !live.iter().any(|&id| func.insts[id].result == Some(cast)),
        "redundant Bool->Bool cast should be eliminated entirely"
    );
}

/// Mem2Reg promotes alloc/store/load to SSA values, substituting loads with
/// the stored value directly. DCE then cleans up dead instructions.
/// Note: Store is a side-effect in DCE and stays; pipeline compaction handles cleanup.
#[test]
fn mem2reg_then_dce() {
    let sig = FunctionSig {
        params: vec![],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
    let ptr = fb.alloc(Type::Int(64));
    let val = fb.const_int(42, 64);
    fb.store(ptr, val);
    let loaded = fb.load(ptr, Type::Int(64));
    // Add a dead intermediate so DCE has something to remove.
    let dead = fb.const_int(999, 64);
    let _dead_add = fb.add(dead, dead);
    fb.ret(Some(loaded));
    let func = fb.build();

    let mut mb = ModuleBuilder::new("test");
    mb.add_function(func);
    let module = mb.build();

    // Mem2Reg promotes to SSA (Load substituted with stored value, removed from blocks).
    let r1 = Mem2Reg.apply(module, None).unwrap();
    assert!(r1.changed);
    let func = &r1.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
    // Check block-owned instructions (not the full arena, which retains dead entries).
    let has_load = func.blocks.values().any(|b| {
        b.insts
            .iter()
            .any(|&id| matches!(func.insts[id].op, Op::Load(_)))
    });
    assert!(
        !has_load,
        "Load should be removed from blocks after mem2reg"
    );

    // DCE removes the dead constant and add.
    let r2 = DeadCodeElimination.apply(r1.module, None).unwrap();
    assert!(r2.changed, "DCE should remove dead ops after mem2reg");

    let func = &r2.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
    assert_well_formed(func);
}

/// CfgSimplify merges empty forwarding blocks, enabling Mem2Reg to see a
/// simpler CFG and promote allocs.
#[test]
fn cfg_simplify_then_mem2reg() {
    let sig = FunctionSig {
        params: vec![],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("test", sig, Visibility::Private);
    let ptr = fb.alloc(Type::Int(64));
    let val = fb.const_int(99, 64);
    fb.store(ptr, val);

    // Create an empty forwarding block.
    let mid = fb.create_block();
    let exit = fb.create_block();
    fb.br(mid, &[]);

    fb.switch_to_block(mid);
    fb.br(exit, &[]);

    fb.switch_to_block(exit);
    let loaded = fb.load(ptr, Type::Int(64));
    fb.ret(Some(loaded));
    let func = fb.build();

    let mut mb = ModuleBuilder::new("test");
    mb.add_function(func);
    let module = mb.build();

    // Simplify CFG first.
    let r1 = CfgSimplify.apply(module, None).unwrap();
    assert!(r1.changed, "should merge empty forwarding block");

    // Now Mem2Reg on the simplified CFG.
    let r2 = Mem2Reg.apply(r1.module, None).unwrap();
    assert!(
        r2.changed,
        "mem2reg should promote after CFG simplification"
    );

    let func = &r2.module.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
    assert_well_formed(func);
}

/// Full pipeline on a multi-block function — verify well-formedness of output.
#[test]
fn full_pipeline_well_formed() {
    let sig = FunctionSig {
        params: vec![Type::Bool],
        return_ty: Type::Value,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("test", sig, Visibility::Public);
    let cond = fb.param(0);
    let ptr = fb.alloc(Type::Int(64));

    // Constant arithmetic that can be folded.
    let a = fb.const_int(10, 64);
    let b = fb.const_int(20, 64);
    let sum = fb.add(a, b);
    fb.store(ptr, sum);

    let then_b = fb.create_block();
    let else_b = fb.create_block();
    let merge = fb.create_block();

    fb.br_if(cond, then_b, &[], else_b, &[]);

    fb.switch_to_block(then_b);
    let val_t = fb.const_int(1, 64);
    fb.store(ptr, val_t);
    fb.br(merge, &[]);

    fb.switch_to_block(else_b);
    let val_f = fb.const_int(0, 64);
    fb.store(ptr, val_f);
    fb.br(merge, &[]);

    fb.switch_to_block(merge);
    let loaded = fb.load(ptr, Type::Int(64));
    fb.ret(Some(loaded));
    let func = fb.build();

    let mut mb = ModuleBuilder::new("test");
    mb.add_function(func);
    let module = mb.build();

    let config = PassConfig::default();
    let pipeline = super::build_pipeline(&config);
    let result = pipeline.run(module).unwrap();

    let func = &result.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
    assert_well_formed(func);
}

/// Full pipeline applied twice — second run should produce no changes.
#[test]
fn full_pipeline_idempotent() {
    let sig = FunctionSig {
        params: vec![Type::Bool],
        return_ty: Type::Value,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("test", sig, Visibility::Public);
    let cond = fb.param(0);
    let then_b = fb.create_block();
    let else_b = fb.create_block();

    fb.br_if(cond, then_b, &[], else_b, &[]);

    fb.switch_to_block(then_b);
    let one = fb.const_int(1, 64);
    fb.ret(Some(one));

    fb.switch_to_block(else_b);
    let zero = fb.const_int(0, 64);
    fb.ret(Some(zero));

    let func = fb.build();

    let mut mb = ModuleBuilder::new("test");
    mb.add_function(func);
    let module = mb.build();

    let config = PassConfig::default();

    // First run.
    let pipeline1 = super::build_pipeline(&config);
    let result1 = pipeline1.run(module).unwrap();
    let func1 = &result1.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
    assert_well_formed(func1);

    // Second run on the already-optimized module.
    let pipeline2 = super::build_pipeline(&config);
    let result2 = pipeline2.run(result1).unwrap();
    let func2 = &result2.functions[FuncId::new(Module::NUM_CORE_BUILTINS)];
    assert_well_formed(func2);
}
