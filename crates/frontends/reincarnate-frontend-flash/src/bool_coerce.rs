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

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::block::BlockId;
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

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        let mut changed = false;
        for func in module.functions.values_mut() {
            changed |= coerce_bool_cmp(func);
            changed |= coerce_void_brif(func);
        }
        Ok(TransformResult { module, changed })
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

fn coerce_bool_cmp(func: &mut Function) -> bool {
    use reincarnate_core::ir::inst::CmpKind;

    // Collect: (inst_id, lhs, rhs, coerce_lhs, coerce_rhs)
    let targets: Vec<(InstId, ValueId, ValueId, bool, bool)> = func
        .insts
        .iter()
        .filter_map(|(id, inst)| {
            let (kind, lhs, rhs) = match &inst.op {
                Op::Cmp(k, a, b) => (*k, *a, *b),
                _ => return None,
            };
            // Only ordering comparisons — Eq/Ne are fine as-is in TypeScript.
            if !matches!(kind, CmpKind::Lt | CmpKind::Le | CmpKind::Gt | CmpKind::Ge) {
                return None;
            }
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
        if let Op::Cmp(_, a, b) = &mut func.insts[inst_id].op {
            *a = new_lhs;
            *b = new_rhs;
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
        let cast_vid = func.value_types.push(Type::Unknown);
        let cast_iid = func.insts.push(Inst {
            op: Op::Cast(cond, Type::Unknown, CastKind::NullableCoerce),
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
