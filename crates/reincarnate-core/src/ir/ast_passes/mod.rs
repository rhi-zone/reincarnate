//! AST-to-AST rewrite passes.
//!
//! These run after Shape->AST lowering to detect and simplify patterns that
//! are easier to match on the high-level AST than during lowering.
//!
//! Passes are grouped by theme:
//! - `cleanup` -- dead code removal, self-assign elimination, stub removal
//! - `control_flow` -- ternary, min/max, for-each, loop-to-while, logical simplification
//! - `variables` -- scope narrowing, decl/init merging, const folding, substitution

mod cleanup;
mod control_flow;
mod variables;

use std::collections::{HashMap, HashSet};

use super::ast::{Expr, Stmt};

// Re-export all public pass functions so existing call sites (`ast_passes::foo`)
// continue to work unchanged.
pub use cleanup::{
    absorb_phi_condition, eliminate_duplicate_assigns, eliminate_forwarding_stubs,
    eliminate_self_assigns, eliminate_unreachable_after_exit, fold_identical_branch_assigns,
    invert_empty_then,
};
pub use control_flow::{
    lower_output_nodes, rewrite_foreach_loops, rewrite_loop_to_while, rewrite_minmax,
    rewrite_ternary, simplify_ternary_to_logical,
};
pub use variables::{
    fold_single_use_consts, forward_substitute, inline_ordered_single_use, merge_decl_init,
    narrow_var_scope,
};

// ---------------------------------------------------------------------------
// AstPass trait
// ---------------------------------------------------------------------------

/// A single AST-to-AST rewrite pass.
///
/// Implementors transform a function body in place. The trait is object-safe so
/// backends can register engine-specific passes alongside the core ones and run
/// them through a uniform pipeline.
#[allow(dead_code)]
pub(crate) trait AstPass {
    /// Human-readable name (for `--dump-ast-after` and diagnostics).
    fn name(&self) -> &str;

    /// Rewrite `body` in place. Called once per function.
    fn run(&self, body: &mut Vec<Stmt>);
}

/// Wrap a bare `fn(&mut Vec<Stmt>)` as an [`AstPass`].
#[allow(dead_code)]
pub(crate) struct FnPass {
    pub name: &'static str,
    pub func: fn(&mut Vec<Stmt>),
}

impl AstPass for FnPass {
    fn name(&self) -> &str {
        self.name
    }
    fn run(&self, body: &mut Vec<Stmt>) {
        (self.func)(body);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Count total statements recursively (used as fixpoint termination check).
pub fn count_stmts(body: &[Stmt]) -> usize {
    body.iter()
        .map(|s| match s {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => 1 + count_stmts(then_body) + count_stmts(else_body),
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                1 + count_stmts(body)
            }
            Stmt::For {
                init, update, body, ..
            } => 1 + count_stmts(init) + count_stmts(update) + count_stmts(body),
            Stmt::Dispatch { blocks, .. } => {
                1 + blocks.iter().map(|(_, b)| count_stmts(b)).sum::<usize>()
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                1 + cases.iter().map(|(_, b)| count_stmts(b)).sum::<usize>()
                    + count_stmts(default_body)
            }
            _ => 1,
        })
        .sum()
}

/// Recurse a rewrite pass into all nested statement bodies.
pub(crate) fn recurse_into_stmt(stmt: &mut Stmt, pass: fn(&mut [Stmt])) {
    match stmt {
        Stmt::If {
            then_body,
            else_body,
            ..
        } => {
            pass(then_body);
            pass(else_body);
        }
        Stmt::While { body, .. } => {
            pass(body);
        }
        Stmt::For {
            init, update, body, ..
        } => {
            pass(init);
            pass(update);
            pass(body);
        }
        Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
            pass(body);
        }
        Stmt::Dispatch { blocks, .. } => {
            for (_, block_body) in blocks {
                pass(block_body);
            }
        }
        Stmt::Switch {
            cases,
            default_body,
            ..
        } => {
            for (_, case_body) in cases {
                pass(case_body);
            }
            pass(default_body);
        }
        _ => {}
    }
}

/// Negate an expression, folding into comparisons when possible.
pub(crate) fn negate_expr(expr: Expr) -> Expr {
    match expr {
        Expr::Not(inner) => *inner,
        Expr::Cmp { kind, lhs, rhs } => Expr::Cmp {
            kind: kind.inverse(),
            lhs,
            rhs,
        },
        other => Expr::Not(Box::new(other)),
    }
}

/// Check whether every path through a body exits unconditionally
/// (return, break, continue, or labeled break).
pub(crate) fn body_always_exits(body: &[Stmt]) -> bool {
    match body.last() {
        Some(Stmt::Return(_) | Stmt::Break | Stmt::Continue | Stmt::LabeledBreak { .. }) => true,
        Some(Stmt::If {
            then_body,
            else_body,
            ..
        }) => !else_body.is_empty() && body_always_exits(then_body) && body_always_exits(else_body),
        _ => false,
    }
}

/// Whether an expression could have observable side effects (calls).
pub(crate) fn expr_has_side_effects(expr: &Expr) -> bool {
    match expr {
        Expr::Call { .. }
        | Expr::CallIndirect { .. }
        | Expr::SystemCall { .. }
        | Expr::MethodCall { .. }
        | Expr::CoroutineCreate { .. }
        | Expr::CoroutineResume(_)
        | Expr::Yield(_)
        | Expr::PostIncrement(_) => true,
        Expr::Literal(_) | Expr::Var(_) | Expr::GlobalRef(_) => false,
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            expr_has_side_effects(lhs) || expr_has_side_effects(rhs)
        }
        Expr::LogicalOr { lhs, rhs } | Expr::LogicalAnd { lhs, rhs } => {
            expr_has_side_effects(lhs) || expr_has_side_effects(rhs)
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::Spread(inner) => expr_has_side_effects(inner),
        Expr::Field { object, .. } => expr_has_side_effects(object),
        Expr::Index { collection, index } => {
            expr_has_side_effects(collection) || expr_has_side_effects(index)
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            expr_has_side_effects(cond)
                || expr_has_side_effects(then_val)
                || expr_has_side_effects(else_val)
        }
        Expr::ArrayInit(elems) | Expr::TupleInit(elems) => elems.iter().any(expr_has_side_effects),
        Expr::StructInit { fields, .. } => {
            fields.iter().any(|(_, v)| expr_has_side_effects(v)) // closure needed: tuple destructure
        }
        Expr::MakeClosure { captures, .. } => captures.iter().any(expr_has_side_effects),
    }
}

/// Whether a statement could have observable side effects.
pub(crate) fn stmt_has_side_effects(stmt: &Stmt) -> bool {
    match stmt {
        Stmt::VarDecl { init: Some(e), .. } => expr_has_side_effects(e),
        Stmt::VarDecl { init: None, .. } => false,
        Stmt::Assign { target, value } => {
            expr_has_side_effects(target) || expr_has_side_effects(value)
        }
        Stmt::CompoundAssign { target, value, .. } => {
            expr_has_side_effects(target) || expr_has_side_effects(value)
        }
        Stmt::Expr(e) => expr_has_side_effects(e),
        Stmt::Return(e) => e.as_ref().is_some_and(expr_has_side_effects),
        // Control flow bodies may contain calls -- conservative barrier.
        Stmt::If { .. }
        | Stmt::While { .. }
        | Stmt::Loop { .. }
        | Stmt::For { .. }
        | Stmt::ForOf { .. }
        | Stmt::Switch { .. }
        | Stmt::Dispatch { .. } => true,
        Stmt::Break | Stmt::Continue | Stmt::LabeledBreak { .. } => false,
    }
}

// ---------------------------------------------------------------------------
// Variable reference counting and substitution
// ---------------------------------------------------------------------------
//
// Three functions with subtly different semantics:
//
// - `count_var_reads_in_{expr,stmt}` -- counts *reads* of `name`. Bare `Var`
//   assignment targets are skipped (they're writes, not reads). Use for
//   substitution safety: "how many reads would be replaced?"
//
// - `stmt_references_var` -- returns true if `name` appears *anywhere*
//   (reads or writes). Use for scope narrowing: "does this block touch
//   the variable at all?"
//
// - `var_is_reassigned` -- returns true if `name` is *written* (Assign or
//   CompoundAssign target). Use for loop safety: "is the init value stale
//   after a back-edge write?"
//

/// Count occurrences of `Var(name)` reads in an expression.
pub(crate) fn count_var_reads_in_expr(expr: &Expr, name: &str) -> usize {
    match expr {
        Expr::Var(n) => usize::from(n == name),
        Expr::Literal(_) | Expr::GlobalRef(_) => 0,
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            count_var_reads_in_expr(lhs, name) + count_var_reads_in_expr(rhs, name)
        }
        Expr::LogicalOr { lhs, rhs } | Expr::LogicalAnd { lhs, rhs } => {
            count_var_reads_in_expr(lhs, name) + count_var_reads_in_expr(rhs, name)
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::CoroutineResume(inner)
        | Expr::PostIncrement(inner)
        | Expr::Spread(inner) => count_var_reads_in_expr(inner, name),
        Expr::Field { object, .. } => count_var_reads_in_expr(object, name),
        Expr::Index { collection, index } => {
            count_var_reads_in_expr(collection, name) + count_var_reads_in_expr(index, name)
        }
        Expr::Call { args, .. } | Expr::CoroutineCreate { args, .. } => {
            args.iter().map(|a| count_var_reads_in_expr(a, name)).sum()
        }
        Expr::CallIndirect { callee, args } => {
            count_var_reads_in_expr(callee, name)
                + args
                    .iter()
                    .map(|a| count_var_reads_in_expr(a, name))
                    .sum::<usize>()
        }
        Expr::SystemCall { args, .. } => {
            args.iter().map(|a| count_var_reads_in_expr(a, name)).sum()
        }
        Expr::MethodCall { receiver, args, .. } => {
            count_var_reads_in_expr(receiver, name)
                + args
                    .iter()
                    .map(|a| count_var_reads_in_expr(a, name))
                    .sum::<usize>()
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            count_var_reads_in_expr(cond, name)
                + count_var_reads_in_expr(then_val, name)
                + count_var_reads_in_expr(else_val, name)
        }
        Expr::ArrayInit(elems) | Expr::TupleInit(elems) => {
            elems.iter().map(|e| count_var_reads_in_expr(e, name)).sum()
        }
        Expr::StructInit { fields, .. } => fields
            .iter()
            .map(|(_, v)| count_var_reads_in_expr(v, name))
            .sum(),
        Expr::Yield(v) => v.as_ref().map_or(0, |e| count_var_reads_in_expr(e, name)),
        Expr::MakeClosure { captures, .. } => captures
            .iter()
            .map(|c| count_var_reads_in_expr(c, name))
            .sum(),
    }
}

/// Count occurrences of `Var(name)` reads in a statement (recursing into nested bodies).
/// Bare `Var` assignment targets are skipped -- see the section comment above.
pub(crate) fn count_var_reads_in_stmt(stmt: &Stmt, name: &str) -> usize {
    match stmt {
        Stmt::VarDecl { name: n, init, .. } => {
            usize::from(n == name)
                + init
                    .as_ref()
                    .map_or(0, |e| count_var_reads_in_expr(e, name))
        }
        Stmt::Assign { target, value } => {
            // A bare Var target is a write, not a read -- don't count it.
            // Complex targets (Field, Index) contain reads of sub-expressions.
            let target_refs = if matches!(target, Expr::Var(v) if v == name) {
                0
            } else {
                count_var_reads_in_expr(target, name)
            };
            target_refs + count_var_reads_in_expr(value, name)
        }
        Stmt::CompoundAssign { target, value, .. } => {
            // CompoundAssign reads AND writes the target, so always count it.
            count_var_reads_in_expr(target, name) + count_var_reads_in_expr(value, name)
        }
        Stmt::Expr(e) => count_var_reads_in_expr(e, name),
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            count_var_reads_in_expr(cond, name)
                + then_body
                    .iter()
                    .map(|s| count_var_reads_in_stmt(s, name))
                    .sum::<usize>()
                + else_body
                    .iter()
                    .map(|s| count_var_reads_in_stmt(s, name))
                    .sum::<usize>()
        }
        Stmt::While { cond, body } => {
            count_var_reads_in_expr(cond, name)
                + body
                    .iter()
                    .map(|s| count_var_reads_in_stmt(s, name))
                    .sum::<usize>()
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            init.iter()
                .map(|s| count_var_reads_in_stmt(s, name))
                .sum::<usize>()
                + count_var_reads_in_expr(cond, name)
                + update
                    .iter()
                    .map(|s| count_var_reads_in_stmt(s, name))
                    .sum::<usize>()
                + body
                    .iter()
                    .map(|s| count_var_reads_in_stmt(s, name))
                    .sum::<usize>()
        }
        Stmt::Loop { body } => body.iter().map(|s| count_var_reads_in_stmt(s, name)).sum(),
        Stmt::ForOf {
            binding,
            iterable,
            body,
            ..
        } => {
            usize::from(binding == name)
                + count_var_reads_in_expr(iterable, name)
                + body
                    .iter()
                    .map(|s| count_var_reads_in_stmt(s, name))
                    .sum::<usize>()
        }
        Stmt::Return(e) => e.as_ref().map_or(0, |e| count_var_reads_in_expr(e, name)),
        Stmt::Dispatch { blocks, .. } => blocks
            .iter()
            .flat_map(|(_, stmts)| stmts.iter())
            .map(|s| count_var_reads_in_stmt(s, name))
            .sum(),
        Stmt::Switch {
            value,
            cases,
            default_body,
        } => {
            count_var_reads_in_expr(value, name)
                + cases
                    .iter()
                    .flat_map(|(_, stmts)| stmts.iter())
                    .map(|s| count_var_reads_in_stmt(s, name))
                    .sum::<usize>()
                + default_body
                    .iter()
                    .map(|s| count_var_reads_in_stmt(s, name))
                    .sum::<usize>()
        }
        Stmt::Break | Stmt::Continue | Stmt::LabeledBreak { .. } => 0,
    }
}

/// Check whether `name` is reassigned anywhere in `stmts` (including nested bodies).
/// This catches `Assign { target: Var(name), .. }` and `CompoundAssign { target: Var(name), .. }`.
pub(crate) fn var_is_reassigned(stmts: &[Stmt], name: &str) -> bool {
    stmts.iter().any(|s| stmt_reassigns_var(s, name))
}

fn stmt_reassigns_var(stmt: &Stmt, name: &str) -> bool {
    match stmt {
        Stmt::Assign {
            target: Expr::Var(v),
            ..
        } if v == name => true,
        Stmt::CompoundAssign {
            target: Expr::Var(v),
            ..
        } if v == name => true,
        // Recurse into nested bodies.
        Stmt::If {
            then_body,
            else_body,
            ..
        } => var_is_reassigned(then_body, name) || var_is_reassigned(else_body, name),
        Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
            var_is_reassigned(body, name)
        }
        Stmt::For {
            init, update, body, ..
        } => {
            var_is_reassigned(init, name)
                || var_is_reassigned(update, name)
                || var_is_reassigned(body, name)
        }
        Stmt::Switch {
            cases,
            default_body,
            ..
        } => {
            cases.iter().any(|(_, b)| var_is_reassigned(b, name))
                || var_is_reassigned(default_body, name)
        }
        Stmt::Dispatch { blocks, .. } => blocks.iter().any(|(_, b)| var_is_reassigned(b, name)),
        _ => false,
    }
}

/// Replace the first `Var(name)` with `replacement` in an expression.
/// Returns `true` if the substitution was performed.
pub(crate) fn substitute_var_in_expr(
    expr: &mut Expr,
    name: &str,
    replacement: &mut Option<Expr>,
) -> bool {
    if replacement.is_none() {
        return false;
    }

    if let Expr::Var(n) = expr {
        if n.as_str() == name {
            *expr = replacement.take().unwrap();
            return true;
        }
        return false;
    }

    match expr {
        Expr::Literal(_) | Expr::GlobalRef(_) | Expr::Var(_) => false,
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            substitute_var_in_expr(lhs, name, replacement)
                || substitute_var_in_expr(rhs, name, replacement)
        }
        Expr::LogicalOr { lhs, rhs } | Expr::LogicalAnd { lhs, rhs } => {
            substitute_var_in_expr(lhs, name, replacement)
                || substitute_var_in_expr(rhs, name, replacement)
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::CoroutineResume(inner)
        | Expr::PostIncrement(inner)
        | Expr::Spread(inner) => substitute_var_in_expr(inner, name, replacement),
        Expr::Field { object, .. } => substitute_var_in_expr(object, name, replacement),
        Expr::Index { collection, index } => {
            substitute_var_in_expr(collection, name, replacement)
                || substitute_var_in_expr(index, name, replacement)
        }
        Expr::Call { args, .. } | Expr::CoroutineCreate { args, .. } => args
            .iter_mut()
            .any(|a| substitute_var_in_expr(a, name, replacement)),
        Expr::CallIndirect { callee, args } => {
            substitute_var_in_expr(callee, name, replacement)
                || args
                    .iter_mut()
                    .any(|a| substitute_var_in_expr(a, name, replacement))
        }
        Expr::SystemCall { args, .. } => args
            .iter_mut()
            .any(|a| substitute_var_in_expr(a, name, replacement)),
        Expr::MethodCall { receiver, args, .. } => {
            substitute_var_in_expr(receiver, name, replacement)
                || args
                    .iter_mut()
                    .any(|a| substitute_var_in_expr(a, name, replacement))
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            substitute_var_in_expr(cond, name, replacement)
                || substitute_var_in_expr(then_val, name, replacement)
                || substitute_var_in_expr(else_val, name, replacement)
        }
        Expr::ArrayInit(elems) | Expr::TupleInit(elems) => elems
            .iter_mut()
            .any(|e| substitute_var_in_expr(e, name, replacement)),
        Expr::StructInit { fields, .. } => fields
            .iter_mut()
            .any(|(_, v)| substitute_var_in_expr(v, name, replacement)),
        Expr::Yield(v) => v
            .as_mut()
            .is_some_and(|e| substitute_var_in_expr(e, name, replacement)),
        Expr::MakeClosure { captures, .. } => captures
            .iter_mut()
            .any(|c| substitute_var_in_expr(c, name, replacement)),
    }
}

/// Replace the first `Var(name)` with `replacement` in a statement.
/// Returns `true` if the substitution was performed.
pub(crate) fn substitute_var_in_stmt(
    stmt: &mut Stmt,
    name: &str,
    replacement: &mut Option<Expr>,
) -> bool {
    if replacement.is_none() {
        return false;
    }

    match stmt {
        Stmt::VarDecl { init, .. } => init
            .as_mut()
            .is_some_and(|e| substitute_var_in_expr(e, name, replacement)),
        Stmt::Assign { target, value } => {
            // A bare Var target is a write -- don't substitute into it.
            // Complex targets (Field, Index) contain reads that can be substituted.
            let target_sub = if matches!(target, Expr::Var(v) if v == name) {
                false
            } else {
                substitute_var_in_expr(target, name, replacement)
            };
            target_sub || substitute_var_in_expr(value, name, replacement)
        }
        Stmt::CompoundAssign { target, value, .. } => {
            // CompoundAssign reads AND writes the target, so always substitute.
            substitute_var_in_expr(target, name, replacement)
                || substitute_var_in_expr(value, name, replacement)
        }
        Stmt::Expr(e) => substitute_var_in_expr(e, name, replacement),
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            substitute_var_in_expr(cond, name, replacement)
                || then_body
                    .iter_mut()
                    .any(|s| substitute_var_in_stmt(s, name, replacement))
                || else_body
                    .iter_mut()
                    .any(|s| substitute_var_in_stmt(s, name, replacement))
        }
        Stmt::While { cond, body } => {
            substitute_var_in_expr(cond, name, replacement)
                || body
                    .iter_mut()
                    .any(|s| substitute_var_in_stmt(s, name, replacement))
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            init.iter_mut()
                .any(|s| substitute_var_in_stmt(s, name, replacement))
                || substitute_var_in_expr(cond, name, replacement)
                || update
                    .iter_mut()
                    .any(|s| substitute_var_in_stmt(s, name, replacement))
                || body
                    .iter_mut()
                    .any(|s| substitute_var_in_stmt(s, name, replacement))
        }
        Stmt::Loop { body } => body
            .iter_mut()
            .any(|s| substitute_var_in_stmt(s, name, replacement)),
        Stmt::ForOf { iterable, body, .. } => {
            substitute_var_in_expr(iterable, name, replacement)
                || body
                    .iter_mut()
                    .any(|s| substitute_var_in_stmt(s, name, replacement))
        }
        Stmt::Return(e) => e
            .as_mut()
            .is_some_and(|e| substitute_var_in_expr(e, name, replacement)),
        Stmt::Dispatch { blocks, .. } => blocks.iter_mut().any(|(_, stmts)| {
            stmts
                .iter_mut()
                .any(|s| substitute_var_in_stmt(s, name, replacement))
        }),
        Stmt::Switch {
            value,
            cases,
            default_body,
        } => {
            substitute_var_in_expr(value, name, replacement)
                || cases.iter_mut().any(|(_, stmts)| {
                    stmts
                        .iter_mut()
                        .any(|s| substitute_var_in_stmt(s, name, replacement))
                })
                || default_body
                    .iter_mut()
                    .any(|s| substitute_var_in_stmt(s, name, replacement))
        }
        Stmt::Break | Stmt::Continue | Stmt::LabeledBreak { .. } => false,
    }
}

/// Whether a statement references a named variable (in any position).
pub(crate) fn stmt_references_var(stmt: &Stmt, name: &str) -> bool {
    match stmt {
        Stmt::VarDecl { name: n, init, .. } => {
            n == name || init.as_ref().is_some_and(|e| expr_references_var(e, name))
        }

        Stmt::Assign { target, value } => {
            expr_references_var(target, name) || expr_references_var(value, name)
        }

        Stmt::CompoundAssign { target, value, .. } => {
            expr_references_var(target, name) || expr_references_var(value, name)
        }

        Stmt::Expr(e) => expr_references_var(e, name),

        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            expr_references_var(cond, name)
                || then_body.iter().any(|s| stmt_references_var(s, name))
                || else_body.iter().any(|s| stmt_references_var(s, name))
        }

        Stmt::While { cond, body } => {
            expr_references_var(cond, name) || body.iter().any(|s| stmt_references_var(s, name))
        }

        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            init.iter().any(|s| stmt_references_var(s, name))
                || expr_references_var(cond, name)
                || update.iter().any(|s| stmt_references_var(s, name))
                || body.iter().any(|s| stmt_references_var(s, name))
        }

        Stmt::Loop { body } => body.iter().any(|s| stmt_references_var(s, name)),

        Stmt::ForOf {
            binding,
            iterable,
            body,
            ..
        } => {
            binding == name
                || expr_references_var(iterable, name)
                || body.iter().any(|s| stmt_references_var(s, name))
        }

        Stmt::Return(e) => e.as_ref().is_some_and(|e| expr_references_var(e, name)),

        Stmt::Dispatch { blocks, .. } => blocks
            .iter()
            .any(|(_, stmts)| stmts.iter().any(|s| stmt_references_var(s, name))),

        Stmt::Switch {
            value,
            cases,
            default_body,
        } => {
            expr_references_var(value, name)
                || cases
                    .iter()
                    .any(|(_, stmts)| stmts.iter().any(|s| stmt_references_var(s, name)))
                || default_body.iter().any(|s| stmt_references_var(s, name))
        }

        Stmt::Break | Stmt::Continue | Stmt::LabeledBreak { .. } => false,
    }
}

/// Whether an expression references a named variable.
pub(crate) fn expr_references_var(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Var(n) => n == name,
        Expr::Literal(_) | Expr::GlobalRef(_) => false,
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            expr_references_var(lhs, name) || expr_references_var(rhs, name)
        }
        Expr::LogicalOr { lhs, rhs } | Expr::LogicalAnd { lhs, rhs } => {
            expr_references_var(lhs, name) || expr_references_var(rhs, name)
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::CoroutineResume(inner)
        | Expr::PostIncrement(inner)
        | Expr::Spread(inner) => expr_references_var(inner, name),
        Expr::Field { object, .. } => expr_references_var(object, name),
        Expr::Index { collection, index } => {
            expr_references_var(collection, name) || expr_references_var(index, name)
        }
        Expr::Call { args, .. } | Expr::CoroutineCreate { args, .. } => {
            args.iter().any(|a| expr_references_var(a, name))
        }
        Expr::CallIndirect { callee, args } => {
            expr_references_var(callee, name) || args.iter().any(|a| expr_references_var(a, name))
        }
        Expr::SystemCall { args, .. } => args.iter().any(|a| expr_references_var(a, name)),
        Expr::MethodCall { receiver, args, .. } => {
            expr_references_var(receiver, name) || args.iter().any(|a| expr_references_var(a, name))
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            expr_references_var(cond, name)
                || expr_references_var(then_val, name)
                || expr_references_var(else_val, name)
        }
        Expr::ArrayInit(elems) | Expr::TupleInit(elems) => {
            elems.iter().any(|e| expr_references_var(e, name))
        }
        Expr::StructInit { fields, .. } => fields.iter().any(|(_, v)| expr_references_var(v, name)),
        Expr::Yield(v) => v.as_ref().is_some_and(|e| expr_references_var(e, name)),
        Expr::MakeClosure { captures, .. } => captures.iter().any(|c| expr_references_var(c, name)),
    }
}

/// Whether an expression contains any variable reference at all.
///
/// Used to identify trivially constant expressions (like `[]`, `42`, `"str"`)
/// that can be freely reordered past any statement.
pub(crate) fn expr_has_any_var_ref(expr: &Expr) -> bool {
    match expr {
        Expr::Var(_) => true,
        Expr::Literal(_) | Expr::GlobalRef(_) => false,
        Expr::Binary { lhs, rhs, .. }
        | Expr::Cmp { lhs, rhs, .. }
        | Expr::LogicalOr { lhs, rhs }
        | Expr::LogicalAnd { lhs, rhs } => expr_has_any_var_ref(lhs) || expr_has_any_var_ref(rhs),
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::CoroutineResume(inner)
        | Expr::PostIncrement(inner)
        | Expr::Spread(inner) => expr_has_any_var_ref(inner),
        Expr::Field { object, .. } => expr_has_any_var_ref(object),
        Expr::Index { collection, index } => {
            expr_has_any_var_ref(collection) || expr_has_any_var_ref(index)
        }
        Expr::Call { args, .. } | Expr::CoroutineCreate { args, .. } => {
            args.iter().any(expr_has_any_var_ref)
        }
        Expr::CallIndirect { callee, args } => {
            expr_has_any_var_ref(callee) || args.iter().any(expr_has_any_var_ref)
        }
        Expr::SystemCall { args, .. } => args.iter().any(expr_has_any_var_ref),
        Expr::MethodCall { receiver, args, .. } => {
            expr_has_any_var_ref(receiver) || args.iter().any(expr_has_any_var_ref)
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            expr_has_any_var_ref(cond)
                || expr_has_any_var_ref(then_val)
                || expr_has_any_var_ref(else_val)
        }
        Expr::ArrayInit(elems) | Expr::TupleInit(elems) => elems.iter().any(expr_has_any_var_ref),
        Expr::StructInit { fields, .. } => fields.iter().any(|(_, v)| expr_has_any_var_ref(v)),
        Expr::Yield(v) => v.as_ref().is_some_and(|e| expr_has_any_var_ref(e)),
        Expr::MakeClosure { captures, .. } => captures.iter().any(expr_has_any_var_ref),
    }
}

/// Precompute read counts for all variable names in a single O(n) pass.
///
/// Semantics match `count_var_reads_in_stmt`: VarDecl names are counted
/// (as self-references), bare Assign targets are NOT counted (they're writes).
pub(crate) fn precompute_var_read_counts(stmts: &[Stmt]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    accumulate_reads_stmts(stmts, &mut counts);
    counts
}

fn accumulate_reads_stmts(stmts: &[Stmt], counts: &mut HashMap<String, usize>) {
    for stmt in stmts {
        accumulate_reads_stmt(stmt, counts);
    }
}

fn accumulate_reads_stmt(stmt: &Stmt, counts: &mut HashMap<String, usize>) {
    match stmt {
        Stmt::VarDecl { name, init, .. } => {
            *counts.entry(name.clone()).or_default() += 1;
            if let Some(e) = init {
                accumulate_reads_expr(e, counts);
            }
        }
        Stmt::Assign { target, value } => {
            // Bare Var target is a write -- don't count.
            if !matches!(target, Expr::Var(_)) {
                accumulate_reads_expr(target, counts);
            }
            accumulate_reads_expr(value, counts);
        }
        Stmt::CompoundAssign { target, value, .. } => {
            accumulate_reads_expr(target, counts);
            accumulate_reads_expr(value, counts);
        }
        Stmt::Expr(e) => accumulate_reads_expr(e, counts),
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            accumulate_reads_expr(cond, counts);
            accumulate_reads_stmts(then_body, counts);
            accumulate_reads_stmts(else_body, counts);
        }
        Stmt::While { cond, body } => {
            accumulate_reads_expr(cond, counts);
            accumulate_reads_stmts(body, counts);
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            accumulate_reads_stmts(init, counts);
            accumulate_reads_expr(cond, counts);
            accumulate_reads_stmts(update, counts);
            accumulate_reads_stmts(body, counts);
        }
        Stmt::Loop { body } => accumulate_reads_stmts(body, counts),
        Stmt::ForOf {
            binding,
            iterable,
            body,
            ..
        } => {
            *counts.entry(binding.clone()).or_default() += 1;
            accumulate_reads_expr(iterable, counts);
            accumulate_reads_stmts(body, counts);
        }
        Stmt::Return(e) => {
            if let Some(e) = e {
                accumulate_reads_expr(e, counts);
            }
        }
        Stmt::Switch {
            value,
            cases,
            default_body,
        } => {
            accumulate_reads_expr(value, counts);
            for (_, stmts) in cases {
                accumulate_reads_stmts(stmts, counts);
            }
            accumulate_reads_stmts(default_body, counts);
        }
        Stmt::Dispatch { blocks, .. } => {
            for (_, stmts) in blocks {
                accumulate_reads_stmts(stmts, counts);
            }
        }
        Stmt::Break | Stmt::Continue | Stmt::LabeledBreak { .. } => {}
    }
}

fn accumulate_reads_expr(expr: &Expr, counts: &mut HashMap<String, usize>) {
    match expr {
        Expr::Var(n) => {
            *counts.entry(n.clone()).or_default() += 1;
        }
        Expr::Literal(_) | Expr::GlobalRef(_) => {}
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            accumulate_reads_expr(lhs, counts);
            accumulate_reads_expr(rhs, counts);
        }
        Expr::LogicalOr { lhs, rhs } | Expr::LogicalAnd { lhs, rhs } => {
            accumulate_reads_expr(lhs, counts);
            accumulate_reads_expr(rhs, counts);
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::CoroutineResume(inner)
        | Expr::PostIncrement(inner)
        | Expr::Spread(inner) => accumulate_reads_expr(inner, counts),
        Expr::Field { object, .. } => accumulate_reads_expr(object, counts),
        Expr::Index { collection, index } => {
            accumulate_reads_expr(collection, counts);
            accumulate_reads_expr(index, counts);
        }
        Expr::Call { args, .. } | Expr::CoroutineCreate { args, .. } => {
            for a in args {
                accumulate_reads_expr(a, counts);
            }
        }
        Expr::CallIndirect { callee, args } => {
            accumulate_reads_expr(callee, counts);
            for a in args {
                accumulate_reads_expr(a, counts);
            }
        }
        Expr::SystemCall { args, .. } => {
            for a in args {
                accumulate_reads_expr(a, counts);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            accumulate_reads_expr(receiver, counts);
            for a in args {
                accumulate_reads_expr(a, counts);
            }
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            accumulate_reads_expr(cond, counts);
            accumulate_reads_expr(then_val, counts);
            accumulate_reads_expr(else_val, counts);
        }
        Expr::ArrayInit(elems) | Expr::TupleInit(elems) => {
            for e in elems {
                accumulate_reads_expr(e, counts);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, v) in fields {
                accumulate_reads_expr(v, counts);
            }
        }
        Expr::Yield(v) => {
            if let Some(e) = v {
                accumulate_reads_expr(e, counts);
            }
        }
        Expr::MakeClosure { captures, .. } => {
            for c in captures {
                accumulate_reads_expr(c, counts);
            }
        }
    }
}

/// Remove all bare assignments to `name` from `body` (recursing into nested scopes).
///
/// Called after removing a dead uninit decl with 0 read refs -- all assignments to
/// that variable are dead writes. Side-effecting RHS values are preserved as
/// expression statements.
/// Remove a dead VarDecl at `idx` and clean up all orphaned assignments to it.
/// If the init expression has side effects, it's preserved as an expression statement.
pub(crate) fn remove_dead_var_decl(body: &mut Vec<Stmt>, idx: usize) {
    let name = match &body[idx] {
        Stmt::VarDecl { name, .. } => name.clone(),
        _ => panic!("remove_dead_var_decl called on non-VarDecl"),
    };
    let has_side_effects = matches!(
        &body[idx],
        Stmt::VarDecl { init: Some(e), .. } if expr_has_side_effects(e)
    );
    if has_side_effects {
        let init = match body.remove(idx) {
            Stmt::VarDecl {
                init: Some(expr), ..
            } => expr,
            _ => unreachable!(),
        };
        body.insert(idx, Stmt::Expr(init));
    } else {
        body.remove(idx);
    }
    remove_dead_assigns(body, &name);
}

fn remove_dead_assigns(body: &mut Vec<Stmt>, name: &str) {
    let mut i = 0;
    while i < body.len() {
        // Recurse into nested bodies first.
        match &mut body[i] {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                remove_dead_assigns(then_body, name);
                remove_dead_assigns(else_body, name);
            }
            Stmt::While { body: inner, .. }
            | Stmt::Loop { body: inner }
            | Stmt::ForOf { body: inner, .. } => {
                remove_dead_assigns(inner, name);
            }
            Stmt::For {
                init,
                update,
                body: inner,
                ..
            } => {
                remove_dead_assigns(init, name);
                remove_dead_assigns(update, name);
                remove_dead_assigns(inner, name);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    remove_dead_assigns(block_body, name);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    remove_dead_assigns(case_body, name);
                }
                remove_dead_assigns(default_body, name);
            }
            _ => {}
        }
        // Check if this statement is a bare assign to the dead variable.
        let is_dead_assign = matches!(
            &body[i],
            Stmt::Assign { target: Expr::Var(n), .. } if n == name
        );
        if is_dead_assign {
            let stmt = body.remove(i);
            if let Stmt::Assign { value, .. } = stmt {
                if expr_has_side_effects(&value) {
                    body.insert(i, Stmt::Expr(value));
                    i += 1;
                }
            }
        } else {
            i += 1;
        }
    }
}

/// Whether an expression is a stable path (Var or Var.field.field... chain).
///
/// Stable paths can be safely re-evaluated past field assignments to different
/// targets because field writes don't change object identity.
pub(crate) fn is_stable_path(expr: &Expr) -> bool {
    match expr {
        Expr::Var(_) => true,
        Expr::Field { object, .. } => is_stable_path(object),
        _ => false,
    }
}

/// Whether a statement assigns to a prefix of the given path expression.
///
/// `this.foo` is a prefix of `this.foo` and `this.foo.bar`, but not of
/// `this.baz` or `this.foo_other`. A prefix assignment would change what
/// the path evaluates to, making it unsafe to sink past.
pub(crate) fn stmt_assigns_to_prefix_of(stmt: &Stmt, path: &Expr) -> bool {
    match stmt {
        Stmt::Assign { target, .. } | Stmt::CompoundAssign { target, .. } => {
            expr_is_prefix_of(target, path)
        }
        // Other statement types (calls, control flow) are conservatively unsafe.
        _ => stmt_has_side_effects(stmt),
    }
}

/// Whether `prefix` is a path prefix of `path`.
///
/// `this.foo` is a prefix of `this.foo` and `this.foo.bar`.
/// `this.foo.bar` is NOT a prefix of `this.foo` (deeper path doesn't
/// invalidate a shallower read).
fn expr_is_prefix_of(prefix: &Expr, path: &Expr) -> bool {
    if prefix == path {
        return true;
    }
    // Walk up the path chain -- if any ancestor matches the prefix, it's a hit.
    match path {
        Expr::Field { object, .. } => expr_is_prefix_of(prefix, object),
        _ => false,
    }
}

/// Count reads of `name` that are in **unconditional** (guaranteed-to-execute)
/// positions within a statement. Reads inside if/loop/switch bodies are
/// conditional and not counted.
pub(crate) fn count_unconditional_reads(stmt: &Stmt, name: &str) -> usize {
    match stmt {
        Stmt::VarDecl { name: n, init, .. } => {
            usize::from(n == name)
                + init
                    .as_ref()
                    .map_or(0, |e| count_var_reads_in_expr(e, name))
        }
        Stmt::Assign { target, value } => {
            let t = if matches!(target, Expr::Var(_)) {
                0
            } else {
                count_var_reads_in_expr(target, name)
            };
            t + count_var_reads_in_expr(value, name)
        }
        Stmt::CompoundAssign { target, value, .. } => {
            count_var_reads_in_expr(target, name) + count_var_reads_in_expr(value, name)
        }
        Stmt::Expr(e) => count_var_reads_in_expr(e, name),
        Stmt::Return(e) => e.as_ref().map_or(0, |e| count_var_reads_in_expr(e, name)),
        // Only the condition is unconditional; bodies are conditional.
        Stmt::If { cond, .. } => count_var_reads_in_expr(cond, name),
        Stmt::While { cond, .. } => count_var_reads_in_expr(cond, name),
        Stmt::For { init, cond, .. } => {
            init.iter()
                .map(|s| count_unconditional_reads(s, name))
                .sum::<usize>()
                + count_var_reads_in_expr(cond, name)
        }
        Stmt::ForOf { iterable, .. } => count_var_reads_in_expr(iterable, name),
        Stmt::Switch { value, .. } => count_var_reads_in_expr(value, name),
        Stmt::Loop { .. } | Stmt::Dispatch { .. } => 0,
        Stmt::Break | Stmt::Continue | Stmt::LabeledBreak { .. } => 0,
    }
}

/// Precompute how many top-level statements reference each variable name.
///
/// For each statement, collects the set of variable names it references
/// (reads, writes, and definitions), then increments the count for each.
pub(crate) fn precompute_stmt_ref_counts(stmts: &[Stmt]) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    let mut names = HashSet::new();
    for stmt in stmts {
        names.clear();
        collect_names_in_stmt(stmt, &mut names);
        for name in &names {
            *counts.entry(name.clone()).or_default() += 1;
        }
    }
    counts
}

fn collect_names_in_stmt(stmt: &Stmt, names: &mut HashSet<String>) {
    match stmt {
        Stmt::VarDecl { name, init, .. } => {
            names.insert(name.clone());
            if let Some(e) = init {
                collect_names_in_expr(e, names);
            }
        }
        Stmt::Assign { target, value } => {
            collect_names_in_expr(target, names);
            collect_names_in_expr(value, names);
        }
        Stmt::CompoundAssign { target, value, .. } => {
            collect_names_in_expr(target, names);
            collect_names_in_expr(value, names);
        }
        Stmt::Expr(e) => collect_names_in_expr(e, names),
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            collect_names_in_expr(cond, names);
            for s in then_body {
                collect_names_in_stmt(s, names);
            }
            for s in else_body {
                collect_names_in_stmt(s, names);
            }
        }
        Stmt::While { cond, body } => {
            collect_names_in_expr(cond, names);
            for s in body {
                collect_names_in_stmt(s, names);
            }
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            for s in init {
                collect_names_in_stmt(s, names);
            }
            collect_names_in_expr(cond, names);
            for s in update {
                collect_names_in_stmt(s, names);
            }
            for s in body {
                collect_names_in_stmt(s, names);
            }
        }
        Stmt::Loop { body } => {
            for s in body {
                collect_names_in_stmt(s, names);
            }
        }
        Stmt::ForOf {
            binding,
            iterable,
            body,
            ..
        } => {
            names.insert(binding.clone());
            collect_names_in_expr(iterable, names);
            for s in body {
                collect_names_in_stmt(s, names);
            }
        }
        Stmt::Return(e) => {
            if let Some(e) = e {
                collect_names_in_expr(e, names);
            }
        }
        Stmt::Switch {
            value,
            cases,
            default_body,
        } => {
            collect_names_in_expr(value, names);
            for (_, stmts) in cases {
                for s in stmts {
                    collect_names_in_stmt(s, names);
                }
            }
            for s in default_body {
                collect_names_in_stmt(s, names);
            }
        }
        Stmt::Dispatch { blocks, .. } => {
            for (_, stmts) in blocks {
                for s in stmts {
                    collect_names_in_stmt(s, names);
                }
            }
        }
        Stmt::Break | Stmt::Continue | Stmt::LabeledBreak { .. } => {}
    }
}

fn collect_names_in_expr(expr: &Expr, names: &mut HashSet<String>) {
    match expr {
        Expr::Var(n) => {
            names.insert(n.clone());
        }
        Expr::Literal(_) | Expr::GlobalRef(_) => {}
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            collect_names_in_expr(lhs, names);
            collect_names_in_expr(rhs, names);
        }
        Expr::LogicalOr { lhs, rhs } | Expr::LogicalAnd { lhs, rhs } => {
            collect_names_in_expr(lhs, names);
            collect_names_in_expr(rhs, names);
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::CoroutineResume(inner)
        | Expr::PostIncrement(inner)
        | Expr::Spread(inner) => collect_names_in_expr(inner, names),
        Expr::Field { object, .. } => collect_names_in_expr(object, names),
        Expr::Index { collection, index } => {
            collect_names_in_expr(collection, names);
            collect_names_in_expr(index, names);
        }
        Expr::Call { args, .. } | Expr::CoroutineCreate { args, .. } => {
            for a in args {
                collect_names_in_expr(a, names);
            }
        }
        Expr::CallIndirect { callee, args } => {
            collect_names_in_expr(callee, names);
            for a in args {
                collect_names_in_expr(a, names);
            }
        }
        Expr::SystemCall { args, .. } => {
            for a in args {
                collect_names_in_expr(a, names);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            collect_names_in_expr(receiver, names);
            for a in args {
                collect_names_in_expr(a, names);
            }
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            collect_names_in_expr(cond, names);
            collect_names_in_expr(then_val, names);
            collect_names_in_expr(else_val, names);
        }
        Expr::ArrayInit(elems) | Expr::TupleInit(elems) => {
            for e in elems {
                collect_names_in_expr(e, names);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, v) in fields {
                collect_names_in_expr(v, names);
            }
        }
        Expr::Yield(v) => {
            if let Some(e) = v {
                collect_names_in_expr(e, names);
            }
        }
        Expr::MakeClosure { captures, .. } => {
            for c in captures {
                collect_names_in_expr(c, names);
            }
        }
    }
}

/// If ALL references to `name` in `stmt` are inside exactly one child body,
/// return a mutable reference to that body. Otherwise return `None`.
pub(crate) fn find_unique_child_body<'a>(
    stmt: &'a mut Stmt,
    name: &str,
) -> Option<&'a mut Vec<Stmt>> {
    match stmt {
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            // References in the condition are not inside a child body.
            if expr_references_var(cond, name) {
                return None;
            }
            let in_then = then_body.iter().any(|s| stmt_references_var(s, name));
            let in_else = else_body.iter().any(|s| stmt_references_var(s, name));
            match (in_then, in_else) {
                (true, false) => Some(then_body),
                (false, true) => Some(else_body),
                _ => None, // both or neither
            }
        }
        Stmt::While { cond, body } => {
            if expr_references_var(cond, name) {
                return None;
            }
            if body.iter().any(|s| stmt_references_var(s, name)) {
                Some(body)
            } else {
                None
            }
        }
        Stmt::Loop { body } => {
            if body.iter().any(|s| stmt_references_var(s, name)) {
                Some(body)
            } else {
                None
            }
        }
        Stmt::ForOf {
            binding,
            iterable,
            body,
            ..
        } => {
            if binding == name || expr_references_var(iterable, name) {
                return None;
            }
            if body.iter().any(|s| stmt_references_var(s, name)) {
                Some(body)
            } else {
                None
            }
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            if expr_references_var(cond, name) {
                return None;
            }
            let in_init = init.iter().any(|s| stmt_references_var(s, name));
            let in_update = update.iter().any(|s| stmt_references_var(s, name));
            let in_body = body.iter().any(|s| stmt_references_var(s, name));
            let count = usize::from(in_init) + usize::from(in_update) + usize::from(in_body);
            if count != 1 {
                return None;
            }
            if in_init {
                Some(init)
            } else if in_body {
                Some(body)
            } else {
                Some(update)
            }
        }
        Stmt::Dispatch { blocks, .. } => {
            let mut found = None;
            for (idx, (_, block_body)) in blocks.iter().enumerate() {
                if block_body.iter().any(|s| stmt_references_var(s, name)) {
                    if found.is_some() {
                        return None; // multiple blocks reference it
                    }
                    found = Some(idx);
                }
            }
            found.map(|idx| &mut blocks[idx].1)
        }
        Stmt::Switch {
            cases,
            default_body,
            ..
        } => {
            let mut found_case = None;
            let in_default = default_body.iter().any(|s| stmt_references_var(s, name));
            for (idx, (_, case_body)) in cases.iter().enumerate() {
                if case_body.iter().any(|s| stmt_references_var(s, name)) {
                    if found_case.is_some() || in_default {
                        return None; // multiple branches reference it
                    }
                    found_case = Some(idx);
                }
            }
            if let Some(idx) = found_case {
                if in_default {
                    return None;
                }
                Some(&mut cases[idx].1)
            } else if in_default {
                Some(default_body)
            } else {
                None
            }
        }
        // Not a compound statement -- can't narrow into it.
        _ => None,
    }
}
