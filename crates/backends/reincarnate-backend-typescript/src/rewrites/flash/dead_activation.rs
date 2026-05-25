//! Dead activation object elimination.

use crate::js_ast::{JsExpr, JsStmt};

fn expr_references_var(expr: &JsExpr, name: &str) -> bool {
    match expr {
        JsExpr::Var(n) => n == name,
        JsExpr::Literal(_)
        | JsExpr::Regex(_)
        | JsExpr::This
        | JsExpr::Activation
        | JsExpr::SuperGet(_) => false,
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
        } => expr_references_var(lhs, name) || expr_references_var(rhs, name),
        JsExpr::Unary { expr: e, .. }
        | JsExpr::Cast { expr: e, .. }
        | JsExpr::TypeCheck { expr: e, .. }
        | JsExpr::Not(e)
        | JsExpr::PostIncrement(e)
        | JsExpr::Spread(e)
        | JsExpr::TypeOf(e)
        | JsExpr::GeneratorResume(e)
        | JsExpr::NonNull(e) => expr_references_var(e, name),
        JsExpr::Field { object, .. } => expr_references_var(object, name),
        JsExpr::Index { collection, index } => {
            expr_references_var(collection, name) || expr_references_var(index, name)
        }
        JsExpr::Call { callee, args } | JsExpr::New { callee, args } => {
            expr_references_var(callee, name) || args.iter().any(|a| expr_references_var(a, name))
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            expr_references_var(cond, name)
                || expr_references_var(then_val, name)
                || expr_references_var(else_val, name)
        }
        JsExpr::ArrayInit(elems) | JsExpr::TupleInit(elems) | JsExpr::SuperCall(elems) => {
            elems.iter().any(|e| expr_references_var(e, name))
        }
        JsExpr::ObjectInit(pairs) => pairs.iter().any(|(_, e)| expr_references_var(e, name)),
        JsExpr::SuperMethodCall { args, .. }
        | JsExpr::GeneratorCreate { args, .. }
        | JsExpr::SystemCall { args, .. } => args.iter().any(|a| expr_references_var(a, name)),
        JsExpr::SuperSet { value, .. } => expr_references_var(value, name),
        JsExpr::Yield(opt) => opt.as_ref().is_some_and(|e| expr_references_var(e, name)),
        JsExpr::ArrowFunction { body, .. } => stmts_reference_var(body, name),
        JsExpr::NullCoalesceAssign { target, value } => {
            expr_references_var(target, name) || expr_references_var(value, name)
        }
        JsExpr::Assign { lhs, rhs } => {
            expr_references_var(lhs, name) || expr_references_var(rhs, name)
        }
    }
}

fn stmts_reference_var(stmts: &[JsStmt], name: &str) -> bool {
    stmts.iter().any(|s| stmt_references_var(s, name))
}

fn stmt_references_var(stmt: &JsStmt, name: &str) -> bool {
    match stmt {
        JsStmt::VarDecl { name: n, init, .. } => {
            n == name || init.as_ref().is_some_and(|e| expr_references_var(e, name))
        }
        JsStmt::Assign { target, value } | JsStmt::CompoundAssign { target, value, .. } => {
            expr_references_var(target, name) || expr_references_var(value, name)
        }
        JsStmt::Expr(e) | JsStmt::Return(Some(e)) | JsStmt::Throw(e) => {
            expr_references_var(e, name)
        }
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            expr_references_var(cond, name)
                || stmts_reference_var(then_body, name)
                || stmts_reference_var(else_body, name)
        }
        JsStmt::While { cond, body } => {
            expr_references_var(cond, name) || stmts_reference_var(body, name)
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            stmts_reference_var(init, name)
                || expr_references_var(cond, name)
                || stmts_reference_var(update, name)
                || stmts_reference_var(body, name)
        }
        JsStmt::Loop { body } => stmts_reference_var(body, name),
        JsStmt::ForOf { iterable, body, .. } => {
            expr_references_var(iterable, name) || stmts_reference_var(body, name)
        }
        JsStmt::Dispatch { blocks, .. } => blocks
            .iter()
            .any(|(_, stmts)| stmts_reference_var(stmts, name)),
        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            expr_references_var(value, name)
                || cases
                    .iter()
                    .any(|(_, stmts)| stmts_reference_var(stmts, name))
                || stmts_reference_var(default_body, name)
        }
        JsStmt::Return(None) | JsStmt::Break | JsStmt::Continue | JsStmt::LabeledBreak { .. } => {
            false
        }
    }
}

fn is_dead_activation_field_write(stmt: &JsStmt, act_name: &str) -> bool {
    matches!(
        stmt,
        JsStmt::Assign {
            target: JsExpr::Field { object, .. },
            ..
        } if matches!(object.as_ref(), JsExpr::Var(n) if n == act_name)
    )
}

/// Eliminate dead activation objects after closure inlining.
pub fn eliminate_dead_activations(body: &mut Vec<JsStmt>) {
    let act_names: Vec<String> = body
        .iter()
        .filter_map(|s| {
            if let JsStmt::VarDecl {
                name,
                init: Some(JsExpr::Activation),
                ..
            } = s
            {
                Some(name.clone())
            } else {
                None
            }
        })
        .collect();

    for act_name in &act_names {
        let all_dead = body.iter().all(|s| {
            if let JsStmt::VarDecl {
                name,
                init: Some(JsExpr::Activation),
                ..
            } = s
            {
                return name == act_name;
            }
            if is_dead_activation_field_write(s, act_name) {
                if let JsStmt::Assign { value, .. } = s {
                    return !expr_references_var(value, act_name);
                }
            }
            !stmt_references_var(s, act_name)
        });

        if all_dead {
            body.retain(|s| {
                if let JsStmt::VarDecl {
                    name,
                    init: Some(JsExpr::Activation),
                    ..
                } = s
                {
                    return name != act_name;
                }
                !is_dead_activation_field_write(s, act_name)
            });
        }
    }

    for stmt in body.iter_mut() {
        eliminate_dead_activations_in_stmt(stmt);
    }
}

fn eliminate_dead_activations_in_stmt(stmt: &mut JsStmt) {
    match stmt {
        JsStmt::VarDecl { init: Some(e), .. }
        | JsStmt::Expr(e)
        | JsStmt::Return(Some(e))
        | JsStmt::Throw(e) => eliminate_dead_activations_in_expr(e),
        JsStmt::Assign { target, value } | JsStmt::CompoundAssign { target, value, .. } => {
            eliminate_dead_activations_in_expr(target);
            eliminate_dead_activations_in_expr(value);
        }
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            eliminate_dead_activations_in_expr(cond);
            eliminate_dead_activations(then_body);
            eliminate_dead_activations(else_body);
        }
        JsStmt::While { cond, body } => {
            eliminate_dead_activations_in_expr(cond);
            eliminate_dead_activations(body);
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            eliminate_dead_activations(init);
            eliminate_dead_activations_in_expr(cond);
            eliminate_dead_activations(update);
            eliminate_dead_activations(body);
        }
        JsStmt::Loop { body } => eliminate_dead_activations(body),
        JsStmt::ForOf { iterable, body, .. } => {
            eliminate_dead_activations_in_expr(iterable);
            eliminate_dead_activations(body);
        }
        JsStmt::Dispatch { blocks, .. } => {
            for (_, stmts) in blocks {
                eliminate_dead_activations(stmts);
            }
        }
        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            eliminate_dead_activations_in_expr(value);
            for (_, stmts) in cases {
                eliminate_dead_activations(stmts);
            }
            eliminate_dead_activations(default_body);
        }
        JsStmt::VarDecl { init: None, .. }
        | JsStmt::Return(None)
        | JsStmt::Break
        | JsStmt::Continue
        | JsStmt::LabeledBreak { .. } => {}
    }
}

fn eliminate_dead_activations_in_expr(expr: &mut JsExpr) {
    match expr {
        JsExpr::ArrowFunction { body, .. } => eliminate_dead_activations(body),
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
            eliminate_dead_activations_in_expr(lhs);
            eliminate_dead_activations_in_expr(rhs);
        }
        JsExpr::Unary { expr: e, .. }
        | JsExpr::Cast { expr: e, .. }
        | JsExpr::TypeCheck { expr: e, .. }
        | JsExpr::Not(e)
        | JsExpr::PostIncrement(e)
        | JsExpr::Spread(e)
        | JsExpr::TypeOf(e)
        | JsExpr::GeneratorResume(e)
        | JsExpr::NonNull(e) => eliminate_dead_activations_in_expr(e),
        JsExpr::Field { object, .. } => eliminate_dead_activations_in_expr(object),
        JsExpr::Index { collection, index } => {
            eliminate_dead_activations_in_expr(collection);
            eliminate_dead_activations_in_expr(index);
        }
        JsExpr::Call { callee, args } | JsExpr::New { callee, args } => {
            eliminate_dead_activations_in_expr(callee);
            for a in args {
                eliminate_dead_activations_in_expr(a);
            }
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            eliminate_dead_activations_in_expr(cond);
            eliminate_dead_activations_in_expr(then_val);
            eliminate_dead_activations_in_expr(else_val);
        }
        JsExpr::ArrayInit(elems) | JsExpr::TupleInit(elems) | JsExpr::SuperCall(elems) => {
            for e in elems {
                eliminate_dead_activations_in_expr(e);
            }
        }
        JsExpr::ObjectInit(pairs) => {
            for (_, e) in pairs {
                eliminate_dead_activations_in_expr(e);
            }
        }
        JsExpr::SuperMethodCall { args, .. }
        | JsExpr::GeneratorCreate { args, .. }
        | JsExpr::SystemCall { args, .. } => {
            for a in args {
                eliminate_dead_activations_in_expr(a);
            }
        }
        JsExpr::SuperSet { value, .. } => eliminate_dead_activations_in_expr(value),
        JsExpr::Yield(Some(e)) => eliminate_dead_activations_in_expr(e),
        JsExpr::NullCoalesceAssign { target, value } => {
            eliminate_dead_activations_in_expr(target);
            eliminate_dead_activations_in_expr(value);
        }
        JsExpr::Assign { lhs, rhs } => {
            eliminate_dead_activations_in_expr(lhs);
            eliminate_dead_activations_in_expr(rhs);
        }
        JsExpr::Literal(_)
        | JsExpr::Var(_)
        | JsExpr::Regex(_)
        | JsExpr::This
        | JsExpr::Activation
        | JsExpr::SuperGet(_)
        | JsExpr::Yield(None) => {}
    }
}
