//! GML logical-op pattern normalization.
//!
//! GML bytecode compiles `a || b` as:
//!   if a { result = 1.0 } else { result = b }
//! and `a && b` as:
//!   if a { result = b } else { result = 0.0 }
//!
//! After Mem2Reg these become block-param patterns:
//!   BrIf cond, then_block[], else_block[]
//!   then_block: { Const(1.0), Br merge(v_1) }  ← trivially pure, passes truthy
//!   else_block: { ...real computation..., Br merge(v_rhs) }
//!
//! The shared structurizer detects the *canonical* `||`/`&&` form where the
//! short-circuit branch forwards the condition itself (not a constant 1/0).
//! This pass normalizes the const-truthy/falsy arg to `v_cond` so the standard
//! detection fires, emitting `cond || rhs` / `cond && rhs` instead of a ternary.
//!
//! Guard: the non-trivial branch must have real (non-const) computation, to
//! avoid converting genuine ternaries like `cond ? 1 : 2` to `cond || 2`.

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::{BlockId, Constant, Function, InstId, Module, Op, ValueId};
use reincarnate_core::pipeline::{Transform, TransformResult};

pub struct GmlLogicalOpNormalize;

impl Transform for GmlLogicalOpNormalize {
    fn name(&self) -> &str {
        "gml-logical-op-normalize"
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        let mut changed = false;
        for func in module.functions.values_mut() {
            changed |= normalize_logical_ops(func);
        }
        Ok(TransformResult { module, changed })
    }
}

fn normalize_logical_ops(func: &mut Function) -> bool {
    let mut changed = false;
    // Collect all BrIf instructions up front to avoid borrow conflicts.
    let brifs: Vec<(InstId, ValueId, BlockId, BlockId)> = func
        .insts
        .iter()
        .filter_map(|(id, inst)| {
            if let Op::BrIf { cond, then_target, else_target, .. } = &inst.op {
                Some((id, *cond, *then_target, *else_target))
            } else {
                None
            }
        })
        .collect();

    for (_, cond, then_target, else_target) in brifs {
        // Try GML OR: then-block is trivially pure with a const-truthy result,
        // else-block has real computation.
        if let Some(br_inst_id) = trivially_pure_const_branch(func, then_target, true) {
            if !is_trivially_pure_block(func, else_target) {
                func.insts[br_inst_id].op = replace_br_arg(func, br_inst_id, cond);
                changed = true;
                continue;
            }
        }
        // Try GML AND: else-block is trivially pure with a const-falsy result,
        // then-block has real computation.
        if let Some(br_inst_id) = trivially_pure_const_branch(func, else_target, false) {
            if !is_trivially_pure_block(func, then_target) {
                func.insts[br_inst_id].op = replace_br_arg(func, br_inst_id, cond);
                changed = true;
            }
        }
    }
    changed
}

/// If `block` is trivially pure (only `Op::Const` instructions) and its
/// terminator `Br` passes a single arg that is const-truthy (for OR, `want_truthy=true`)
/// or const-falsy (for AND, `want_truthy=false`), return the `InstId` of the Br.
fn trivially_pure_const_branch(
    func: &Function,
    block: BlockId,
    want_truthy: bool,
) -> Option<InstId> {
    let blk = &func.blocks[block];
    // Block must take no params (empty — it's just the intermediate branch block).
    if !blk.params.is_empty() {
        return None;
    }
    // All instructions except the last must be Op::Const.
    let n = blk.insts.len();
    if n == 0 {
        return None;
    }
    for &inst_id in &blk.insts[..n - 1] {
        if !matches!(func.insts[inst_id].op, Op::Const(_)) {
            return None;
        }
    }
    // Last instruction must be Br with a single arg.
    let last_id = blk.insts[n - 1];
    let Op::Br { args, .. } = &func.insts[last_id].op else {
        return None;
    };
    if args.len() != 1 {
        return None;
    }
    let arg = args[0];
    // The arg must be a const truthy or falsy value.
    let is_match = func.insts.iter().any(|(_, inst)| {
        inst.result == Some(arg)
            && match &inst.op {
                Op::Const(c) => {
                    if want_truthy {
                        is_const_truthy(c)
                    } else {
                        is_const_falsy(c)
                    }
                }
                _ => false,
            }
    });
    if is_match { Some(last_id) } else { None }
}

/// Return true if `block` contains only `Op::Const` instructions (plus a Br
/// terminator). Used to reject genuine ternaries where BOTH branches are pure.
fn is_trivially_pure_block(func: &Function, block: BlockId) -> bool {
    let blk = &func.blocks[block];
    blk.insts.iter().all(|&id| {
        matches!(
            func.insts[id].op,
            Op::Const(_) | Op::Br { .. } | Op::BrIf { .. } | Op::Switch { .. } | Op::Return(_)
        )
    })
}

/// Build a new `Op::Br` for `br_inst_id` with the single arg replaced by `new_arg`.
fn replace_br_arg(func: &Function, br_inst_id: InstId, new_arg: ValueId) -> Op {
    let Op::Br { target, .. } = &func.insts[br_inst_id].op else {
        unreachable!("replace_br_arg called on non-Br");
    };
    Op::Br { target: *target, args: vec![new_arg] }
}

fn is_const_truthy(c: &Constant) -> bool {
    match c {
        Constant::Bool(true) | Constant::Int(1) => true,
        Constant::Float(f) => *f == 1.0,
        _ => false,
    }
}

fn is_const_falsy(c: &Constant) -> bool {
    match c {
        Constant::Bool(false) | Constant::Int(0) | Constant::Null => true,
        Constant::Float(f) => *f == 0.0,
        _ => false,
    }
}
