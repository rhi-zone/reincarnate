//! Structured linear IR for the hybrid lowering pipeline.
//!
//! The pipeline converts `Shape + Function` → `Vec<Stmt>` in three phases:
//!
//! - **Phase 1** (`linearize`): Walk the Shape tree and produce a flat
//!   `Vec<LinearStmt>` where every instruction is a `Def(ValueId, InstId)`,
//!   control flow comes from shapes, and branch args become `Assign(dst, src)`.
//!   No inlining decisions — trivial shape walk.
//!
//! - **Phase 2** (`resolve`): Pure resolution on LinearStmt. Constants always
//!   inlined, scope lookups marked always-rebuild, pure single-use values
//!   marked for substitution, dead pure code dropped. This handles ~90% of
//!   inlining with zero side-effect concerns.
//!
//! - **Phase 3** (`emit`): LinearStmt → Vec<Stmt>. Remaining side-effecting
//!   single-use values inlined if no intervening side effects. Multi-use
//!   values get `const`/`let` declarations. Produces the AST for existing
//!   AST-to-AST passes.

#![allow(dead_code)] // Phases 2 & 3 will consume these types.

use super::func::Function;
use super::inst::{InstId, Op};
use super::structurize::{BlockArgAssign, Shape};
use super::value::ValueId;

use crate::entity::EntityRef;

// -----------------------------------------------------------------------
// LinearStmt — structured IR with ValueId/InstId references
// -----------------------------------------------------------------------

/// A statement in the structured linear IR.
///
/// References IR entities (ValueId, InstId) rather than carrying string names
/// or materialized expressions. The `Function` provides context for looking up
/// instruction ops and value metadata.
#[derive(Debug, Clone)]
pub(crate) enum LinearStmt {
    /// Instruction with a result value: `result = op(...)`.
    Def { result: ValueId, inst_id: InstId },
    /// Instruction without a useful result (void calls, stores, etc.).
    Effect { inst_id: InstId },
    /// Branch argument assignment: `dst = src`.
    Assign { dst: ValueId, src: ValueId },
    /// Conditional: `if (cond) { then } else { else }`.
    If {
        cond: ValueId,
        then_body: Vec<LinearStmt>,
        else_body: Vec<LinearStmt>,
    },
    /// While loop. Header instructions compute the condition each iteration.
    While {
        header: Vec<LinearStmt>,
        cond: ValueId,
        cond_negated: bool,
        body: Vec<LinearStmt>,
    },
    /// For loop: init; header+cond; body; update.
    For {
        init: Vec<LinearStmt>,
        header: Vec<LinearStmt>,
        cond: ValueId,
        cond_negated: bool,
        update: Vec<LinearStmt>,
        body: Vec<LinearStmt>,
    },
    /// Infinite loop (`while (true) { ... }`).
    Loop { body: Vec<LinearStmt> },
    /// Return from function.
    Return { value: Option<ValueId> },
    /// Break out of innermost loop.
    Break,
    /// Continue to next iteration.
    Continue,
    /// Break to an outer loop (`depth` levels up).
    LabeledBreak { depth: usize },
    /// Short-circuit OR: `phi = cond || rhs`.
    LogicalOr {
        cond: ValueId,
        phi: ValueId,
        rhs_body: Vec<LinearStmt>,
        rhs: ValueId,
    },
    /// Short-circuit AND: `phi = cond && rhs`.
    LogicalAnd {
        cond: ValueId,
        phi: ValueId,
        rhs_body: Vec<LinearStmt>,
        rhs: ValueId,
    },
    /// Dispatch (fallback for irreducible CFGs).
    Dispatch {
        blocks: Vec<(usize, Vec<LinearStmt>)>,
        entry: usize,
    },
}

// -----------------------------------------------------------------------
// Phase 1: linearize — Shape → Vec<LinearStmt>
// -----------------------------------------------------------------------

/// Convert a structurized shape tree into a flat sequence of LinearStmts.
///
/// This is a faithful translation: no inlining decisions, no dead code
/// elimination, no expression building. Every non-terminator instruction
/// becomes a `Def` or `Effect`, every branch arg becomes an `Assign`, and
/// control flow shapes map 1:1 to LinearStmt control flow variants.
pub(crate) fn linearize(func: &Function, shape: &Shape) -> Vec<LinearStmt> {
    let mut out = Vec::new();
    linearize_into(func, shape, &mut out, false);
    out
}

fn linearize_into(
    func: &Function,
    shape: &Shape,
    out: &mut Vec<LinearStmt>,
    skip_init: bool,
) {
    match shape {
        Shape::Block(block_id) => {
            emit_block_insts(func, *block_id, out);
        }

        Shape::Seq(parts) => {
            for (i, part) in parts.iter().enumerate() {
                let next_is_loop = matches!(
                    parts.get(i + 1),
                    Some(Shape::WhileLoop { .. })
                        | Some(Shape::ForLoop { .. })
                        | Some(Shape::Loop { .. })
                );

                // When a non-Block shape precedes a loop in a Seq, its
                // trailing assigns already set the loop header's block
                // params — the loop's own init_assigns would duplicate them.
                let this_skip_init = if i > 0 {
                    let prev = &parts[i - 1];
                    let is_loop = matches!(
                        part,
                        Shape::WhileLoop { .. }
                            | Shape::ForLoop { .. }
                            | Shape::Loop { .. }
                    );
                    is_loop && !matches!(prev, Shape::Block(_))
                } else {
                    false
                };

                linearize_into(func, part, out, this_skip_init);

                // After a Block, emit Br target assignments — unless the
                // next shape is a loop (it captures its own init assigns).
                if let Shape::Block(block_id) = part {
                    if !next_is_loop {
                        emit_br_assigns(func, *block_id, out);
                    }
                }
            }
        }

        Shape::IfElse {
            block,
            cond,
            then_assigns,
            then_body,
            then_trailing_assigns,
            else_assigns,
            else_body,
            else_trailing_assigns,
        } => {
            // Header block instructions (setup for the branch condition).
            emit_block_insts(func, *block, out);

            let mut then_stmts = Vec::new();
            emit_arg_assigns(then_assigns, &mut then_stmts);
            linearize_into(func, then_body, &mut then_stmts, false);
            emit_arg_assigns(then_trailing_assigns, &mut then_stmts);

            let mut else_stmts = Vec::new();
            emit_arg_assigns(else_assigns, &mut else_stmts);
            linearize_into(func, else_body, &mut else_stmts, false);
            emit_arg_assigns(else_trailing_assigns, &mut else_stmts);

            out.push(LinearStmt::If {
                cond: *cond,
                then_body: then_stmts,
                else_body: else_stmts,
            });
        }

        Shape::WhileLoop {
            header,
            cond,
            cond_negated,
            body,
        } => {
            let mut header_stmts = Vec::new();
            emit_block_insts(func, *header, &mut header_stmts);

            let mut body_stmts = Vec::new();
            linearize_into(func, body, &mut body_stmts, false);

            out.push(LinearStmt::While {
                header: header_stmts,
                cond: *cond,
                cond_negated: *cond_negated,
                body: body_stmts,
            });
        }

        Shape::ForLoop {
            header,
            init_assigns,
            cond,
            cond_negated,
            update_assigns,
            body,
        } => {
            let init = if skip_init {
                Vec::new()
            } else {
                let mut stmts = Vec::new();
                emit_arg_assigns(init_assigns, &mut stmts);
                stmts
            };

            let mut header_stmts = Vec::new();
            emit_block_insts(func, *header, &mut header_stmts);

            let mut body_stmts = Vec::new();
            linearize_into(func, body, &mut body_stmts, false);

            let mut update_stmts = Vec::new();
            emit_arg_assigns(update_assigns, &mut update_stmts);

            out.push(LinearStmt::For {
                init,
                header: header_stmts,
                cond: *cond,
                cond_negated: *cond_negated,
                update: update_stmts,
                body: body_stmts,
            });
        }

        Shape::Loop { header: _, body } => {
            let mut body_stmts = Vec::new();
            linearize_into(func, body, &mut body_stmts, false);
            out.push(LinearStmt::Loop { body: body_stmts });
        }

        Shape::Break => out.push(LinearStmt::Break),
        Shape::Continue => out.push(LinearStmt::Continue),
        Shape::LabeledBreak { depth } => out.push(LinearStmt::LabeledBreak { depth: *depth }),

        Shape::LogicalOr {
            block,
            cond,
            phi,
            rhs_body,
            rhs,
        } => {
            emit_block_insts(func, *block, out);
            let mut rhs_stmts = Vec::new();
            linearize_into(func, rhs_body, &mut rhs_stmts, false);
            out.push(LinearStmt::LogicalOr {
                cond: *cond,
                phi: *phi,
                rhs_body: rhs_stmts,
                rhs: *rhs,
            });
        }

        Shape::LogicalAnd {
            block,
            cond,
            phi,
            rhs_body,
            rhs,
        } => {
            emit_block_insts(func, *block, out);
            let mut rhs_stmts = Vec::new();
            linearize_into(func, rhs_body, &mut rhs_stmts, false);
            out.push(LinearStmt::LogicalAnd {
                cond: *cond,
                phi: *phi,
                rhs_body: rhs_stmts,
                rhs: *rhs,
            });
        }

        Shape::Dispatch { blocks, entry } => {
            let mut dispatch_blocks = Vec::new();
            for &block_id in blocks {
                let mut block_stmts = Vec::new();
                emit_dispatch_block_insts(func, block_id, &mut block_stmts);
                dispatch_blocks.push((block_id.index() as usize, block_stmts));
            }
            out.push(LinearStmt::Dispatch {
                blocks: dispatch_blocks,
                entry: entry.index() as usize,
            });
        }
    }
}

// -----------------------------------------------------------------------
// Block instruction helpers
// -----------------------------------------------------------------------

/// Emit non-terminator instructions from a block as Def/Effect/Return.
fn emit_block_insts(func: &Function, block_id: super::block::BlockId, out: &mut Vec<LinearStmt>) {
    let block = &func.blocks[block_id];
    for &inst_id in &block.insts {
        let inst = &func.insts[inst_id];
        match &inst.op {
            // Terminators absorbed by Shape structure.
            Op::Br { .. } | Op::BrIf { .. } | Op::Switch { .. } => break,
            Op::Return(v) => {
                out.push(LinearStmt::Return { value: *v });
            }
            _ => {
                if let Some(result) = inst.result {
                    out.push(LinearStmt::Def { result, inst_id });
                } else {
                    out.push(LinearStmt::Effect { inst_id });
                }
            }
        }
    }
}

/// Emit all instructions from a dispatch block (including terminators).
fn emit_dispatch_block_insts(
    func: &Function,
    block_id: super::block::BlockId,
    out: &mut Vec<LinearStmt>,
) {
    let block = &func.blocks[block_id];
    for &inst_id in &block.insts {
        let inst = &func.insts[inst_id];
        match &inst.op {
            Op::Return(v) => out.push(LinearStmt::Return { value: *v }),
            _ => {
                if let Some(result) = inst.result {
                    out.push(LinearStmt::Def { result, inst_id });
                } else {
                    out.push(LinearStmt::Effect { inst_id });
                }
            }
        }
    }
}

/// Emit branch-arg assignments from a block's unconditional Br terminator.
fn emit_br_assigns(func: &Function, block_id: super::block::BlockId, out: &mut Vec<LinearStmt>) {
    let block = &func.blocks[block_id];
    let Some(&last_inst) = block.insts.last() else {
        return;
    };
    if let Op::Br { target, ref args } = func.insts[last_inst].op {
        let target_block = &func.blocks[target];
        for (param, &src) in target_block.params.iter().zip(args.iter()) {
            if param.value == src {
                continue;
            }
            out.push(LinearStmt::Assign {
                dst: param.value,
                src,
            });
        }
    }
}

/// Emit BlockArgAssign entries as Assign statements.
fn emit_arg_assigns(assigns: &[BlockArgAssign], out: &mut Vec<LinearStmt>) {
    for assign in assigns {
        out.push(LinearStmt::Assign {
            dst: assign.dst,
            src: assign.src,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::FunctionBuilder;
    use crate::ir::func::Visibility;
    use crate::ir::structurize::structurize;
    use crate::ir::ty::{FunctionSig, Type};
    use crate::ir::value::Constant;

    #[test]
    fn linearize_simple_block() {
        let sig = FunctionSig {
            params: vec![Type::Int(64), Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("add", sig, Visibility::Public);
        let a = fb.param(0);
        let b = fb.param(1);
        let sum = fb.add(a, b);
        fb.ret(Some(sum));
        let func = fb.build();

        let shape = Shape::Block(func.entry);
        let linear = linearize(&func, &shape);

        // Should have: Def(sum, add_inst), Return(Some(sum))
        assert_eq!(linear.len(), 2);
        assert!(matches!(&linear[0], LinearStmt::Def { result, .. } if *result == sum));
        assert!(matches!(&linear[1], LinearStmt::Return { value: Some(v) } if *v == sum));
    }

    #[test]
    fn linearize_if_else() {
        let sig = FunctionSig {
            params: vec![Type::Bool, Type::Int(64), Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("choose", sig, Visibility::Public);
        let cond = fb.param(0);
        let x = fb.param(1);
        let y = fb.param(2);

        let (then_block, then_vals) = fb.create_block_with_params(&[Type::Int(64)]);
        let (else_block, else_vals) = fb.create_block_with_params(&[Type::Int(64)]);

        fb.br_if(cond, then_block, &[x], else_block, &[y]);

        fb.switch_to_block(then_block);
        fb.ret(Some(then_vals[0]));

        fb.switch_to_block(else_block);
        fb.ret(Some(else_vals[0]));

        let mut func = fb.build();
        let shape = structurize(&mut func);
        let linear = linearize(&func, &shape);

        // Should contain an If with Return in each branch.
        let has_if = linear.iter().any(|s| matches!(s, LinearStmt::If { .. }));
        assert!(has_if, "Expected an If in linearized output: {linear:?}");
    }

    #[test]
    fn linearize_constant_def() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
        let c = fb.const_int(42);
        fb.ret(Some(c));
        let func = fb.build();

        let shape = Shape::Block(func.entry);
        let linear = linearize(&func, &shape);

        // Const produces a Def, then Return.
        assert_eq!(linear.len(), 2);
        assert!(matches!(&linear[0], LinearStmt::Def { result, .. } if *result == c));
        match &func.insts[match &linear[0] {
            LinearStmt::Def { inst_id, .. } => *inst_id,
            _ => unreachable!(),
        }]
        .op
        {
            Op::Const(Constant::Int(42)) => {}
            other => panic!("Expected Const(Int(42)), got {other:?}"),
        }
    }
}
