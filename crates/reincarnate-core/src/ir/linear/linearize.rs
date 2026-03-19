//! Phase 1: linearize — Shape -> Vec<LinearStmt>
//!
//! Convert a structurized shape tree into a flat sequence of LinearStmts.
//! This is a faithful translation: no inlining decisions, no dead code
//! elimination, no expression building.

use super::LinearStmt;
use crate::entity::EntityRef;
use crate::ir::block::BlockId;
use crate::ir::func::Function;
use crate::ir::inst::Terminator;
use crate::ir::structurize::{BlockArgAssign, Shape};

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

fn linearize_into(func: &Function, shape: &Shape, out: &mut Vec<LinearStmt>, skip_init: bool) {
    match shape {
        Shape::Block(block_id) => {
            emit_block_insts(func, *block_id, out);
        }

        Shape::Seq(parts) => {
            for (i, part) in parts.iter().enumerate() {
                let next_is_loop = matches!(parts.get(i + 1), Some(Shape::ForLoop { .. }));

                // When a non-Block shape precedes a ForLoop in a Seq, its
                // trailing assigns already set the loop header's block
                // params — the ForLoop's own init_assigns would duplicate them.
                let this_skip_init = if i > 0 {
                    let prev = &parts[i - 1];
                    let is_loop = matches!(part, Shape::ForLoop { .. });
                    is_loop && !matches!(prev, Shape::Block(_))
                } else {
                    false
                };

                linearize_into(func, part, out, this_skip_init);

                // After a Block, emit Br target assignments — unless the
                // next shape is a ForLoop (it has its own init_assigns field).
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
            emit_arg_assigns(then_assigns, func, &mut then_stmts);
            linearize_into(func, then_body, &mut then_stmts, false);
            emit_arg_assigns(then_trailing_assigns, func, &mut then_stmts);

            let mut else_stmts = Vec::new();
            emit_arg_assigns(else_assigns, func, &mut else_stmts);
            linearize_into(func, else_body, &mut else_stmts, false);
            emit_arg_assigns(else_trailing_assigns, func, &mut else_stmts);

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
                emit_arg_assigns(init_assigns, func, &mut stmts);
                stmts
            };

            let mut header_stmts = Vec::new();
            emit_block_insts(func, *header, &mut header_stmts);

            let mut body_stmts = Vec::new();
            linearize_into(func, body, &mut body_stmts, false);

            // The body's back-edge block emits br assigns via emit_br_assigns,
            // but ForLoop captures those same assigns in update_assigns.
            // Strip the duplicates from the body.
            strip_back_edge_assigns(&mut body_stmts, update_assigns);

            let mut update_stmts = Vec::new();
            emit_arg_assigns(update_assigns, func, &mut update_stmts);

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

        Shape::Switch {
            block,
            value,
            cases,
            default_assigns,
            default_body,
            default_trailing_assigns,
        } => {
            emit_block_insts(func, *block, out);

            let mut case_stmts = Vec::with_capacity(cases.len());
            for case in cases {
                let mut stmts = Vec::new();
                emit_arg_assigns(&case.entry_assigns, func, &mut stmts);
                linearize_into(func, &case.body, &mut stmts, false);
                emit_arg_assigns(&case.trailing_assigns, func, &mut stmts);
                case_stmts.push((case.value.clone(), stmts));
            }

            let mut default_stmts = Vec::new();
            emit_arg_assigns(default_assigns, func, &mut default_stmts);
            linearize_into(func, default_body, &mut default_stmts, false);
            emit_arg_assigns(default_trailing_assigns, func, &mut default_stmts);

            out.push(LinearStmt::Switch {
                value: *value,
                cases: case_stmts,
                default_body: default_stmts,
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
fn emit_block_insts(func: &Function, block_id: BlockId, out: &mut Vec<LinearStmt>) {
    let block = &func.blocks[block_id];
    for &inst_id in &block.insts {
        let inst = &func.insts[inst_id];
        if let Some(result) = inst.result {
            out.push(LinearStmt::Def { result, inst_id });
        } else {
            out.push(LinearStmt::Effect { inst_id });
        }
    }
    // Emit Return from block terminator (branch terminators are absorbed by Shape).
    if let Terminator::Return(v) = &block.terminator {
        out.push(LinearStmt::Return { value: *v });
    }
}

/// Emit all instructions from a dispatch block (including terminators).
fn emit_dispatch_block_insts(func: &Function, block_id: BlockId, out: &mut Vec<LinearStmt>) {
    let block = &func.blocks[block_id];
    for &inst_id in &block.insts {
        let inst = &func.insts[inst_id];
        if let Some(result) = inst.result {
            out.push(LinearStmt::Def { result, inst_id });
        } else {
            out.push(LinearStmt::Effect { inst_id });
        }
    }
    if let Terminator::Return(v) = &block.terminator {
        out.push(LinearStmt::Return { value: *v });
    }
}

/// Emit branch-arg assignments from a block's unconditional Br terminator.
fn emit_br_assigns(func: &Function, block_id: BlockId, out: &mut Vec<LinearStmt>) {
    let block = &func.blocks[block_id];
    if let Terminator::Br { target, ref args } = block.terminator {
        let target_block = &func.blocks[target];
        for (param, &src) in target_block.params.iter().zip(args.iter()) {
            if param.value == src || func.null_sentinel_values.contains(&src) {
                continue;
            }
            out.push(LinearStmt::Assign {
                dst: param.value,
                src,
            });
        }
    }
}

/// Emit BlockArgAssign entries as Assign statements, skipping any assignment
/// whose source is a Mem2Reg null sentinel.
fn emit_arg_assigns(assigns: &[BlockArgAssign], func: &Function, out: &mut Vec<LinearStmt>) {
    for assign in assigns {
        if func.null_sentinel_values.contains(&assign.src) {
            continue;
        }
        out.push(LinearStmt::Assign {
            dst: assign.dst,
            src: assign.src,
        });
    }
}

/// Remove trailing Assign stmts (before any Continue) that duplicate `update` entries.
fn strip_back_edge_assigns(body: &mut Vec<LinearStmt>, update: &[BlockArgAssign]) {
    let mut end = body.len();
    if matches!(body.last(), Some(LinearStmt::Continue)) {
        end -= 1;
    }
    while end > 0 {
        if let LinearStmt::Assign { dst, src } = &body[end - 1] {
            if update.iter().any(|a| a.dst == *dst && a.src == *src) {
                body.remove(end - 1);
                end -= 1;
                continue;
            }
        }
        break;
    }
}
