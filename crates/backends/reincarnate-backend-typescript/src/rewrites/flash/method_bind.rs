//! AS3 method closure auto-binding: cachedBind(this, this.method).

use std::collections::HashSet;

use reincarnate_core::ir::Constant;

use crate::js_ast::{JsExpr, JsStmt};

/// Extract an object-literal key string from a JsExpr key.
pub(super) fn extract_object_key(expr: &JsExpr) -> String {
    match expr {
        JsExpr::Literal(Constant::String(s)) => s.clone(),
        _ => format!("{:?}", expr),
    }
}

/// Post-rewrite pass: wrap `this.method` references (not in callee position)
/// with `cachedBind(this, this.method)` for identity-stable method closures.
pub(super) fn bind_method_refs_stmts(stmts: &mut [JsStmt], bindable: &HashSet<String>) {
    for stmt in stmts.iter_mut() {
        bind_method_refs_stmt(stmt, bindable);
    }
}

fn bind_method_refs_stmt(stmt: &mut JsStmt, bindable: &HashSet<String>) {
    match stmt {
        JsStmt::VarDecl { init, .. } => {
            if let Some(e) = init {
                bind_method_refs_expr(e, bindable, false);
            }
        }
        JsStmt::Assign { target, value } => {
            bind_method_refs_expr(target, bindable, false);
            bind_method_refs_expr(value, bindable, false);
        }
        JsStmt::CompoundAssign { target, value, .. } => {
            bind_method_refs_expr(target, bindable, false);
            bind_method_refs_expr(value, bindable, false);
        }
        JsStmt::Expr(e) => bind_method_refs_expr(e, bindable, false),
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            bind_method_refs_expr(cond, bindable, false);
            bind_method_refs_stmts(then_body, bindable);
            bind_method_refs_stmts(else_body, bindable);
        }
        JsStmt::While { cond, body } => {
            bind_method_refs_expr(cond, bindable, false);
            bind_method_refs_stmts(body, bindable);
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            bind_method_refs_stmts(init, bindable);
            bind_method_refs_expr(cond, bindable, false);
            bind_method_refs_stmts(update, bindable);
            bind_method_refs_stmts(body, bindable);
        }
        JsStmt::Loop { body } => {
            bind_method_refs_stmts(body, bindable);
        }
        JsStmt::ForOf { iterable, body, .. } => {
            bind_method_refs_expr(iterable, bindable, false);
            bind_method_refs_stmts(body, bindable);
        }
        JsStmt::Return(Some(e)) | JsStmt::Throw(e) => {
            bind_method_refs_expr(e, bindable, false);
        }
        JsStmt::Dispatch { blocks, .. } => {
            for (_, stmts) in blocks.iter_mut() {
                bind_method_refs_stmts(stmts, bindable);
            }
        }
        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            bind_method_refs_expr(value, bindable, false);
            for (_, stmts) in cases.iter_mut() {
                bind_method_refs_stmts(stmts, bindable);
            }
            bind_method_refs_stmts(default_body, bindable);
        }
        JsStmt::Return(None) | JsStmt::Break | JsStmt::Continue | JsStmt::LabeledBreak { .. } => {}
    }
}

/// Recursively bind method refs in an expression.
/// `in_callee` is true when this expression is the direct callee of a Call or New.
pub(super) fn bind_method_refs_expr(
    expr: &mut JsExpr,
    bindable: &HashSet<String>,
    in_callee: bool,
) {
    // First, recurse into children with correct in_callee propagation.
    match expr {
        JsExpr::Call { callee, args } => {
            // Skip recursing into cachedBind's method-ref arg to prevent
            // double-wrapping when an inlined closure's body is revisited.
            let is_cached_bind =
                matches!(callee.as_ref(), JsExpr::Var(n) if n == "cachedBind") && args.len() == 2;
            bind_method_refs_expr(callee, bindable, true);
            if is_cached_bind {
                // Only recurse into the thisArg (args[0]), not the method ref (args[1]).
                bind_method_refs_expr(&mut args[0], bindable, false);
            } else {
                for a in args.iter_mut() {
                    bind_method_refs_expr(a, bindable, false);
                }
            }
        }
        JsExpr::New { callee, args } => {
            bind_method_refs_expr(callee, bindable, true);
            for a in args.iter_mut() {
                bind_method_refs_expr(a, bindable, false);
            }
        }
        JsExpr::Binary { lhs, rhs, .. }
        | JsExpr::Cmp { lhs, rhs, .. }
        | JsExpr::LooseEq { lhs, rhs }
        | JsExpr::LooseNe { lhs, rhs }
        | JsExpr::LogicalOr { lhs, rhs }
        | JsExpr::LogicalAnd { lhs, rhs }
        | JsExpr::In {
            key: lhs,
            object: rhs,
        }
        | JsExpr::Delete {
            object: lhs,
            key: rhs,
        } => {
            bind_method_refs_expr(lhs, bindable, false);
            bind_method_refs_expr(rhs, bindable, false);
        }
        JsExpr::Unary { expr: e, .. }
        | JsExpr::Cast { expr: e, .. }
        | JsExpr::TypeCheck { expr: e, .. }
        | JsExpr::Not(e)
        | JsExpr::PostIncrement(e)
        | JsExpr::Spread(e)
        | JsExpr::TypeOf(e)
        | JsExpr::GeneratorResume(e)
        | JsExpr::NonNull(e) => {
            bind_method_refs_expr(e, bindable, false);
        }
        JsExpr::Field { object, .. } => {
            bind_method_refs_expr(object, bindable, false);
        }
        JsExpr::Index { collection, index } => {
            bind_method_refs_expr(collection, bindable, false);
            bind_method_refs_expr(index, bindable, false);
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            bind_method_refs_expr(cond, bindable, false);
            bind_method_refs_expr(then_val, bindable, false);
            bind_method_refs_expr(else_val, bindable, false);
        }
        JsExpr::ArrayInit(elems) | JsExpr::TupleInit(elems) | JsExpr::SuperCall(elems) => {
            for e in elems.iter_mut() {
                bind_method_refs_expr(e, bindable, false);
            }
        }
        JsExpr::ObjectInit(pairs) => {
            for (_, e) in pairs.iter_mut() {
                bind_method_refs_expr(e, bindable, false);
            }
        }
        JsExpr::SuperMethodCall { args, .. }
        | JsExpr::GeneratorCreate { args, .. }
        | JsExpr::SystemCall { args, .. } => {
            for a in args.iter_mut() {
                bind_method_refs_expr(a, bindable, false);
            }
        }
        JsExpr::SuperSet { value, .. } => {
            bind_method_refs_expr(value, bindable, false);
        }
        JsExpr::Yield(opt) => {
            if let Some(e) = opt {
                bind_method_refs_expr(e, bindable, false);
            }
        }
        JsExpr::ArrowFunction { body, .. } => {
            bind_method_refs_stmts(body, bindable);
        }
        JsExpr::NullCoalesceAssign { target, value } => {
            bind_method_refs_expr(target, bindable, false);
            bind_method_refs_expr(value, bindable, false);
        }
        JsExpr::Assign { lhs, rhs } => {
            bind_method_refs_expr(lhs, bindable, false);
            bind_method_refs_expr(rhs, bindable, false);
        }
        JsExpr::Literal(_)
        | JsExpr::Var(_)
        | JsExpr::This
        | JsExpr::Activation
        | JsExpr::SuperGet(_) => {}
    }

    // After recursing, check if this expr should be wrapped with cachedBind.
    if !in_callee {
        if let JsExpr::Field { object, field } = expr {
            if matches!(object.as_ref(), JsExpr::This) && bindable.contains(field.as_str()) {
                // Replace `this.method` with `cachedBind(this, this.method)`.
                let original = std::mem::replace(expr, JsExpr::This); // placeholder
                *expr = JsExpr::Call {
                    callee: Box::new(JsExpr::Var("cachedBind".to_string())),
                    args: vec![JsExpr::This, original],
                };
            }
        }
    }
}
