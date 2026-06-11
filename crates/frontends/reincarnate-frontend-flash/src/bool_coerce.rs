//! AS3 boolean coercion passes.
//!
//! Two problems arise from AS3's implicit boolean coercion rules:
//!
//! ## TS2365 — Bool in ordering comparison
//!
//! AS3 allows `boolVal > 0`, `boolVal >= 2`, etc. because booleans are
//! implicitly coerced to 0/1 before comparison.  TypeScript's `strict` mode
//! rejects ordering comparisons (`<`, `<=`, `>`, `>=`) between `boolean` and
//! `number`.
//!
//! Fix: for `Cmp(Lt|Le|Gt|Ge, lhs, rhs)` where either operand is `Bool`-typed,
//! insert `Cast(v, Float(64), Coerce)` (prints as `Number(v)`) before the op.
//!
//! ## TS1345 — Void expression in boolean context
//!
//! AS3 allows calling a `void`-returning function and using the result as a
//! condition: `if (this.loadGame(slot))`.  Void is falsy, so the branch is
//! never taken — but TypeScript rejects testing a `void` value for truthiness.
//!
//! Fix: for `BrIf(cond, ...)` where `cond` is `Void`-typed, insert
//! `Cast(cond, Unknown, NullableCoerce)` before the BrIf.  The printer emits
//! `(expr as any)`, which TypeScript accepts and preserves runtime behaviour
//! (the original void/undefined value is still falsy).

use std::collections::HashSet;

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::block::BlockId;
use reincarnate_core::ir::func::FuncId;
use reincarnate_core::ir::inst::{CastKind, Inst, InstId, Op, Terminator};
use reincarnate_core::ir::ty::Type;
use reincarnate_core::ir::{Function, Module, ValueId};
use reincarnate_core::pipeline::{PureIrPass, Transform, TransformResult};

pub struct FlashBoolCoerce;

impl Transform for FlashBoolCoerce {
    fn name(&self) -> &str {
        "flash-bool-coerce"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(
        &self,
        mut module: Module,
        dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        // Collect FuncIds of ordering comparison builtins (exclude Eq/Ne).
        let ordering_cmp_fids: HashSet<FuncId> = ["cmp_lt", "cmp_le", "cmp_gt", "cmp_ge"]
            .iter()
            .filter_map(|name| module.lookup_runtime(name))
            .collect();

        let mut changed_funcs: HashSet<FuncId> = HashSet::new();
        for func_id in module.functions.keys().collect::<Vec<_>>() {
            if dirty.is_some_and(|d| !d.contains(&func_id)) {
                continue;
            }
            let func = &mut module.functions[func_id];
            let mut func_changed = coerce_bool_cmp(func, &ordering_cmp_fids);
            func_changed |= coerce_void_brif(func);
            if func_changed {
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

impl PureIrPass for FlashBoolCoerce {}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_bool(func: &Function, v: ValueId) -> bool {
    matches!(func.value_types.get(v), Some(Type::Bool))
}

fn is_void(func: &Function, v: ValueId) -> bool {
    matches!(func.value_types.get(v), Some(Type::Void))
}

/// Insert `Cast(v, to_type, kind)` immediately before `before_inst_id`.
fn insert_cast_before(
    func: &mut Function,
    v: ValueId,
    before_inst_id: InstId,
    to_type: Type,
    kind: CastKind,
) -> ValueId {
    let cast_vid = func.value_types.push(to_type.clone());
    let cast_iid = func.insts.push(Inst {
        op: Op::Cast(v, to_type, kind),
        result: Some(cast_vid),
        span: None,
    });
    'outer: for block in func.blocks.values_mut() {
        for (pos, &iid) in block.insts.iter().enumerate() {
            if iid == before_inst_id {
                block.insts.insert(pos, cast_iid);
                break 'outer;
            }
        }
    }
    cast_vid
}

// ---------------------------------------------------------------------------
// Pass 1 — Bool operands in ordering comparisons (TS2365)
// ---------------------------------------------------------------------------

fn coerce_bool_cmp(func: &mut Function, ordering_cmp_fids: &HashSet<FuncId>) -> bool {
    // Collect: (inst_id, lhs, rhs, coerce_lhs, coerce_rhs)
    let targets: Vec<(InstId, ValueId, ValueId, bool, bool)> = func
        .insts
        .iter()
        .filter_map(|(id, inst)| {
            let (lhs, rhs) = match &inst.op {
                Op::Call { func: fid, args }
                    if ordering_cmp_fids.contains(fid) && args.len() == 2 =>
                {
                    (args[0], args[1])
                }
                _ => return None,
            };
            let lhs_c = is_bool(func, lhs);
            let rhs_c = is_bool(func, rhs);
            if lhs_c || rhs_c {
                Some((id, lhs, rhs, lhs_c, rhs_c))
            } else {
                None
            }
        })
        .collect();

    if targets.is_empty() {
        return false;
    }

    for (inst_id, lhs, rhs, lhs_c, rhs_c) in targets {
        let new_lhs = if lhs_c {
            insert_cast_before(func, lhs, inst_id, Type::Float(64), CastKind::Coerce)
        } else {
            lhs
        };
        let new_rhs = if rhs_c {
            insert_cast_before(func, rhs, inst_id, Type::Float(64), CastKind::Coerce)
        } else {
            rhs
        };
        if let Op::Call { args, .. } = &mut func.insts[inst_id].op {
            if args.len() == 2 {
                args[0] = new_lhs;
                args[1] = new_rhs;
            }
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Pass 2 — Void condition in BrIf (TS1345)
// ---------------------------------------------------------------------------

fn coerce_void_brif(func: &mut Function) -> bool {
    // Collect blocks whose BrIf terminator has a void condition.
    let targets: Vec<(BlockId, ValueId)> = func
        .blocks
        .iter()
        .filter_map(|(bid, block)| {
            if let Terminator::BrIf { cond, .. } = &block.terminator {
                if is_void(func, *cond) {
                    return Some((bid, *cond));
                }
            }
            None
        })
        .collect();

    if targets.is_empty() {
        return false;
    }

    for (block_id, cond) in targets {
        // Insert cast at end of block's insts.
        let cast_vid = func.value_types.push(Type::Value);
        let cast_iid = func.insts.push(Inst {
            op: Op::Cast(cond, Type::Value, CastKind::NullableCoerce),
            result: Some(cast_vid),
            span: None,
        });
        func.blocks[block_id].insts.push(cast_iid);
        if let Terminator::BrIf { cond: c, .. } = &mut func.blocks[block_id].terminator {
            *c = cast_vid;
        }
    }

    true
}
