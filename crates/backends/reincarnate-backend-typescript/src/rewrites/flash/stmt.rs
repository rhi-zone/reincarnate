//! Statement rewriting.

use reincarnate_core::ir::Constant;

use crate::js_ast::{JsExpr, JsStmt};

use super::context::FlashRewriteCtx;
use super::expr::{rewrite_expr, rewrite_exprs};

pub(super) fn rewrite_stmts(stmts: Vec<JsStmt>, ctx: &FlashRewriteCtx) -> Vec<JsStmt> {
    stmts
        .into_iter()
        .filter_map(|s| rewrite_stmt(s, ctx))
        .collect()
}

/// Rewrite a single JS statement. Returns `None` to skip (e.g. suppressed super).
pub(super) fn rewrite_stmt(stmt: JsStmt, ctx: &FlashRewriteCtx) -> Option<JsStmt> {
    // Check statement-level SystemCall patterns BEFORE recursing.
    if let JsStmt::Expr(JsExpr::SystemCall {
        ref system,
        ref method,
        ref args,
        ..
    }) = stmt
    {
        // constructSuper → super(_shims, ...args), super(...args), or skip
        if system == "Flash.Class" && method == "constructSuper" {
            if ctx.suppress_super {
                return None;
            }
            let rewritten_args = rewrite_exprs(args.clone(), ctx);
            // Skip first arg (this).
            let rest: Vec<JsExpr> = rewritten_args.into_iter().skip(1).collect();
            if ctx.parent_is_runtime {
                // Runtime parent (MovieClip, Font, EventDispatcher, etc.) doesn't
                // accept _shims — emit super() with the original non-shims args only.
                return Some(JsStmt::Expr(JsExpr::SuperCall(rest)));
            }
            // User-defined parent: prepend _shims so the parent constructor can
            // thread the shims reference up to the root.
            let mut super_args = vec![JsExpr::Var("_shims".to_string())];
            super_args.extend(rest);
            return Some(JsStmt::Expr(JsExpr::SuperCall(super_args)));
        }

        // throw(x) → throw x;
        if system == "Flash.Exception" && method == "throw" && args.len() == 1 {
            let arg = rewrite_expr(args[0].clone(), ctx);
            return Some(JsStmt::Throw(arg));
        }

        // setSuper(this, "prop", value) → super.prop = value;
        if system == "Flash.Class" && method == "setSuper" && args.len() == 3 {
            if let JsExpr::Literal(Constant::String(ref name)) = args[1] {
                let short = name.rsplit("::").next().unwrap_or(name);
                let value = rewrite_expr(args[2].clone(), ctx);
                return Some(JsStmt::Assign {
                    target: JsExpr::SuperGet(short.to_string()),
                    value,
                });
            }
        }

        // findPropStrict/findProperty as standalone statement → skip
        if system == "Flash.Scope" && (method == "findPropStrict" || method == "findProperty") {
            return None;
        }
    }

    Some(match stmt {
        JsStmt::VarDecl {
            name,
            ty,
            init,
            mutable,
        } => JsStmt::VarDecl {
            name,
            ty,
            init: init.map(|e| rewrite_expr(e, ctx)),
            mutable,
        },

        JsStmt::Assign { target, value } => JsStmt::Assign {
            target: rewrite_expr(target, ctx),
            value: rewrite_expr(value, ctx),
        },

        JsStmt::CompoundAssign { target, op, value } => JsStmt::CompoundAssign {
            target: rewrite_expr(target, ctx),
            op,
            value: rewrite_expr(value, ctx),
        },

        JsStmt::Expr(expr) => {
            let rewritten = rewrite_expr(expr, ctx);
            // Skip empty var references (standalone scope lookups that resolved to nothing).
            if let JsExpr::Var(ref name) = rewritten {
                if name.is_empty() {
                    return None;
                }
            }
            JsStmt::Expr(rewritten)
        }

        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => JsStmt::If {
            cond: rewrite_expr(cond, ctx),
            then_body: rewrite_stmts(then_body, ctx),
            else_body: rewrite_stmts(else_body, ctx),
        },

        JsStmt::While { cond, body } => JsStmt::While {
            cond: rewrite_expr(cond, ctx),
            body: rewrite_stmts(body, ctx),
        },

        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => JsStmt::For {
            init: rewrite_stmts(init, ctx),
            cond: rewrite_expr(cond, ctx),
            update: rewrite_stmts(update, ctx),
            body: rewrite_stmts(body, ctx),
        },

        JsStmt::Loop { body } => JsStmt::Loop {
            body: rewrite_stmts(body, ctx),
        },

        JsStmt::ForOf {
            binding,
            declare,
            iterable,
            body,
        } => JsStmt::ForOf {
            binding,
            declare,
            iterable: rewrite_expr(iterable, ctx),
            body: rewrite_stmts(body, ctx),
        },

        JsStmt::Return(expr) => JsStmt::Return(expr.map(|e| rewrite_expr(e, ctx))),
        JsStmt::Throw(expr) => JsStmt::Throw(rewrite_expr(expr, ctx)),
        JsStmt::Break | JsStmt::Continue | JsStmt::LabeledBreak { .. } => stmt,

        JsStmt::Dispatch { blocks, entry } => JsStmt::Dispatch {
            blocks: blocks
                .into_iter()
                .map(|(idx, stmts)| (idx, rewrite_stmts(stmts, ctx)))
                .collect(),
            entry,
        },

        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => JsStmt::Switch {
            value: rewrite_expr(value, ctx),
            cases: cases
                .into_iter()
                .map(|(c, stmts)| (rewrite_expr(c, ctx), rewrite_stmts(stmts, ctx)))
                .collect(),
            default_body: rewrite_stmts(default_body, ctx),
        },
    })
}
