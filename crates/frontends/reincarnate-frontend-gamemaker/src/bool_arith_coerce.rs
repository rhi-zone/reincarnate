//! GML boolean-in-arithmetic coercion pass.
//!
//! GML treats booleans as integers (0/1) in arithmetic contexts.  A comparison
//! like `this.hp < half_hp` produces a `Bool`-typed value that the game author
//! then uses directly in arithmetic: `speed * half_hp`, `base + half_hp * 10`.
//!
//! This is valid GML semantics — booleans ARE numbers in GML.  TypeScript does
//! not allow `bool * number` (`TS2362`/`TS2365`), so we insert an explicit cast
//! `Bool → Float(64)` before each Bool operand of an arithmetic instruction.
//! The cast prints as a no-op at JS runtime (`true | 0` → `1`) but satisfies
//! TypeScript's type checker.
//!
//! Arithmetic ops covered: `Add`, `Sub`, `Mul`, `Div`, `Rem`.
//! Bitwise ops (`BitAnd`, `BitOr`, `BitXor`, `Shl`, `Shr`) are excluded: they
//! emit as `|`, `&`, etc. in TypeScript which already accepts bool operands
//! (TypeScript widens boolean to number for bitwise).

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::inst::{CastKind, Inst, Op};
use reincarnate_core::ir::ty::Type;
use reincarnate_core::ir::{Function, Module, ValueId};
use reincarnate_core::pipeline::{Transform, TransformResult};

pub struct GmlBoolArithCoerce;

impl Transform for GmlBoolArithCoerce {
    fn name(&self) -> &str {
        "gml-bool-arith-coerce"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        let mut changed = false;
        for func in module.functions.values_mut() {
            changed |= coerce_bool_arithmetic(func);
        }
        Ok(TransformResult { module, changed })
    }
}

/// Returns true if `v` has type `Bool` in `func.value_types`.
fn is_bool(func: &Function, v: ValueId) -> bool {
    matches!(func.value_types.get(v), Some(Type::Bool))
}

/// Insert `Cast(v, Float(64), Coerce)` before `before_inst_id` in the block
/// that contains it, and return the new ValueId.
fn insert_cast_before(func: &mut Function, v: ValueId, before_inst_id: reincarnate_core::ir::InstId) -> ValueId {
    let cast_vid = func.value_types.push(Type::Float(64));
    let cast_inst_id = func.insts.push(Inst {
        op: Op::Cast(v, Type::Float(64), CastKind::Coerce),
        result: Some(cast_vid),
        span: None,
    });
    // Find the block containing before_inst_id and insert cast just before it.
    'outer: for block in func.blocks.values_mut() {
        for (pos, &iid) in block.insts.iter().enumerate() {
            if iid == before_inst_id {
                block.insts.insert(pos, cast_inst_id);
                break 'outer;
            }
        }
    }
    cast_vid
}

fn coerce_bool_arithmetic(func: &mut Function) -> bool {
    // Collect all arithmetic ops where at least one Bool operand exists.
    // We need (inst_id, lhs, rhs, lhs_is_bool, rhs_is_bool).
    let targets: Vec<(reincarnate_core::ir::InstId, ValueId, ValueId, bool, bool)> = func
        .insts
        .iter()
        .filter_map(|(id, inst)| {
            let (a, b) = match &inst.op {
                Op::Add(a, b) | Op::Sub(a, b) | Op::Mul(a, b) | Op::Div(a, b) | Op::Rem(a, b) => (*a, *b),
                _ => return None,
            };
            let a_bool = is_bool(func, a);
            let b_bool = is_bool(func, b);
            if a_bool || b_bool {
                Some((id, a, b, a_bool, b_bool))
            } else {
                None
            }
        })
        .collect();

    if targets.is_empty() {
        return false;
    }

    for (inst_id, lhs, rhs, lhs_is_bool, rhs_is_bool) in targets {
        let new_lhs = if lhs_is_bool {
            insert_cast_before(func, lhs, inst_id)
        } else {
            lhs
        };
        let new_rhs = if rhs_is_bool {
            insert_cast_before(func, rhs, inst_id)
        } else {
            rhs
        };
        // Replace the operands in the arithmetic instruction.
        match &mut func.insts[inst_id].op {
            Op::Add(a, b) | Op::Sub(a, b) | Op::Mul(a, b) | Op::Div(a, b) | Op::Rem(a, b) => {
                *a = new_lhs;
                *b = new_rhs;
            }
            _ => {}
        }
    }

    true
}
