//! Super-call hoisting for Flash constructors.

use std::collections::HashSet;

use reincarnate_core::ir::{CastKind, Constant, Type};

use crate::js_ast::{JsExpr, JsStmt};

/// Move the first `super()` call as early as possible in a constructor body,
/// without hoisting it above statements that define variables it depends on.
///
/// When `class_name` is provided, also rewrites `this.field` references in
/// `super()` arguments to `ClassName.prototype.field`, since ES6 forbids
/// accessing `this` before `super()` in derived class constructors.
pub fn hoist_super_call(body: &mut Vec<JsStmt>, class_name: Option<&str>) {
    let pos = body
        .iter()
        .position(|s| matches!(s, JsStmt::Expr(JsExpr::SuperCall(_))));
    let Some(i) = pos else { return };
    // Collect all variable names referenced by the super call's arguments.
    let mut needed = HashSet::new();
    if let JsStmt::Expr(JsExpr::SuperCall(args)) = &body[i] {
        for arg in args {
            collect_expr_vars(arg, &mut needed);
        }
    }
    // Find the latest statement before `i` that writes to any needed variable.
    let target = if needed.is_empty() {
        0
    } else {
        let mut last_dep: Option<usize> = None;
        for (j, s) in body.iter().enumerate().take(i) {
            if stmt_writes_any(s, &needed) {
                last_dep = Some(j);
            }
        }
        match last_dep {
            Some(j) => j + 1,
            None => 0,
        }
    };
    if target < i {
        let stmt = body.remove(i);
        body.insert(target, stmt);
    }

    // Rewrite `this.field` → `ClassName.prototype.field` in super() args.
    // ES6 forbids `this` before `super()` in derived class constructors, but
    // AVM2 allows it. Method references (the common case) live on the prototype.
    if let Some(cn) = class_name {
        let pos = body
            .iter()
            .position(|s| matches!(s, JsStmt::Expr(JsExpr::SuperCall(_))));
        if let Some(idx) = pos {
            if let JsStmt::Expr(JsExpr::SuperCall(args)) = &mut body[idx] {
                for arg in args.iter_mut() {
                    rewrite_this_to_prototype(arg, cn);
                }
            }
        }
    }
}

/// Collect all `Var` name references in a JS expression.
pub(super) fn collect_expr_vars(expr: &JsExpr, out: &mut HashSet<String>) {
    match expr {
        JsExpr::Var(name) => {
            out.insert(name.clone());
        }
        JsExpr::Literal(_) | JsExpr::This | JsExpr::Activation | JsExpr::SuperGet(_) => {}
        JsExpr::Binary { lhs, rhs, .. }
        | JsExpr::Cmp { lhs, rhs, .. }
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
            collect_expr_vars(lhs, out);
            collect_expr_vars(rhs, out);
        }
        JsExpr::Unary { expr: e, .. }
        | JsExpr::Cast { expr: e, .. }
        | JsExpr::TypeCheck { expr: e, .. }
        | JsExpr::Not(e)
        | JsExpr::PostIncrement(e)
        | JsExpr::Spread(e)
        | JsExpr::TypeOf(e)
        | JsExpr::GeneratorResume(e)
        | JsExpr::NonNull(e) => collect_expr_vars(e, out),
        JsExpr::Field { object, .. } => collect_expr_vars(object, out),
        JsExpr::Index { collection, index } => {
            collect_expr_vars(collection, out);
            collect_expr_vars(index, out);
        }
        JsExpr::Call { callee, args } | JsExpr::New { callee, args } => {
            collect_expr_vars(callee, out);
            for a in args {
                collect_expr_vars(a, out);
            }
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            collect_expr_vars(cond, out);
            collect_expr_vars(then_val, out);
            collect_expr_vars(else_val, out);
        }
        JsExpr::ArrayInit(elems) | JsExpr::TupleInit(elems) | JsExpr::SuperCall(elems) => {
            for e in elems {
                collect_expr_vars(e, out);
            }
        }
        JsExpr::ObjectInit(pairs) => {
            for (_, e) in pairs {
                collect_expr_vars(e, out);
            }
        }
        JsExpr::SuperMethodCall { args, .. }
        | JsExpr::GeneratorCreate { args, .. }
        | JsExpr::SystemCall { args, .. } => {
            for a in args {
                collect_expr_vars(a, out);
            }
        }
        JsExpr::SuperSet { value, .. } => collect_expr_vars(value, out),
        JsExpr::Yield(opt) => {
            if let Some(e) = opt {
                collect_expr_vars(e, out);
            }
        }
        JsExpr::ArrowFunction { body, .. } => {
            collect_stmts_vars(body, out);
        }
        JsExpr::NullCoalesceAssign { target, value } => {
            collect_expr_vars(target, out);
            collect_expr_vars(value, out);
        }
    }
}

/// Collect variable references from a list of statements.
pub(super) fn collect_stmts_vars(stmts: &[JsStmt], out: &mut HashSet<String>) {
    for stmt in stmts {
        match stmt {
            JsStmt::VarDecl { init: Some(e), .. }
            | JsStmt::Expr(e)
            | JsStmt::Return(Some(e))
            | JsStmt::Throw(e) => {
                collect_expr_vars(e, out);
            }
            JsStmt::Assign { target, value } | JsStmt::CompoundAssign { target, value, .. } => {
                collect_expr_vars(target, out);
                collect_expr_vars(value, out);
            }
            JsStmt::If {
                cond,
                then_body,
                else_body,
            } => {
                collect_expr_vars(cond, out);
                collect_stmts_vars(then_body, out);
                collect_stmts_vars(else_body, out);
            }
            JsStmt::While { cond, body } => {
                collect_expr_vars(cond, out);
                collect_stmts_vars(body, out);
            }
            JsStmt::For {
                init,
                cond,
                update,
                body,
            } => {
                collect_stmts_vars(init, out);
                collect_expr_vars(cond, out);
                collect_stmts_vars(update, out);
                collect_stmts_vars(body, out);
            }
            JsStmt::Loop { body } => collect_stmts_vars(body, out),
            JsStmt::ForOf { iterable, body, .. } => {
                collect_expr_vars(iterable, out);
                collect_stmts_vars(body, out);
            }
            JsStmt::Dispatch { blocks, .. } => {
                for (_, stmts) in blocks {
                    collect_stmts_vars(stmts, out);
                }
            }
            JsStmt::Switch {
                value,
                cases,
                default_body,
            } => {
                collect_expr_vars(value, out);
                for (_, stmts) in cases {
                    collect_stmts_vars(stmts, out);
                }
                collect_stmts_vars(default_body, out);
            }
            JsStmt::VarDecl { init: None, .. }
            | JsStmt::Return(None)
            | JsStmt::Break
            | JsStmt::Continue
            | JsStmt::LabeledBreak { .. } => {}
        }
    }
}

/// Replace every `Var(var_name)` with `This` throughout a statement list.
///
/// Used when inlining a closure as an arrow function: the closure was compiled
/// with `self_param_name = None` (first param is the activation scope, not
/// `this`), so references to the outer method's self parameter (`v0`) remain as
/// bare `Var("v0")`.  Arrow functions inherit `this` from the enclosing scope,
/// so `Var("v0")` → `This` is semantically correct.
pub(super) fn subst_var_to_this_stmts(stmts: &mut [JsStmt], var_name: &str) {
    for stmt in stmts.iter_mut() {
        subst_var_to_this_stmt(stmt, var_name);
    }
}

fn subst_var_to_this_stmt(stmt: &mut JsStmt, var_name: &str) {
    match stmt {
        JsStmt::VarDecl { init, .. } => {
            if let Some(e) = init {
                subst_var_to_this_expr(e, var_name);
            }
        }
        JsStmt::Assign { target, value } => {
            subst_var_to_this_expr(target, var_name);
            subst_var_to_this_expr(value, var_name);
        }
        JsStmt::CompoundAssign { target, value, .. } => {
            subst_var_to_this_expr(target, var_name);
            subst_var_to_this_expr(value, var_name);
        }
        JsStmt::Expr(e) => subst_var_to_this_expr(e, var_name),
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            subst_var_to_this_expr(cond, var_name);
            subst_var_to_this_stmts(then_body, var_name);
            subst_var_to_this_stmts(else_body, var_name);
        }
        JsStmt::While { cond, body } => {
            subst_var_to_this_expr(cond, var_name);
            subst_var_to_this_stmts(body, var_name);
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            subst_var_to_this_stmts(init, var_name);
            subst_var_to_this_expr(cond, var_name);
            subst_var_to_this_stmts(update, var_name);
            subst_var_to_this_stmts(body, var_name);
        }
        JsStmt::Loop { body } => subst_var_to_this_stmts(body, var_name),
        JsStmt::ForOf { iterable, body, .. } => {
            subst_var_to_this_expr(iterable, var_name);
            subst_var_to_this_stmts(body, var_name);
        }
        JsStmt::Return(Some(e)) | JsStmt::Throw(e) => subst_var_to_this_expr(e, var_name),
        JsStmt::Dispatch { blocks, .. } => {
            for (_, stmts) in blocks.iter_mut() {
                subst_var_to_this_stmts(stmts, var_name);
            }
        }
        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            subst_var_to_this_expr(value, var_name);
            for (_, stmts) in cases.iter_mut() {
                subst_var_to_this_stmts(stmts, var_name);
            }
            subst_var_to_this_stmts(default_body, var_name);
        }
        JsStmt::Return(None) | JsStmt::Break | JsStmt::Continue | JsStmt::LabeledBreak { .. } => {}
    }
}

fn subst_var_to_this_expr(expr: &mut JsExpr, var_name: &str) {
    match expr {
        JsExpr::Var(name) if name == var_name => *expr = JsExpr::This,
        JsExpr::Var(_)
        | JsExpr::Literal(_)
        | JsExpr::This
        | JsExpr::Activation
        | JsExpr::SuperGet(_) => {}
        JsExpr::Binary { lhs, rhs, .. }
        | JsExpr::Cmp { lhs, rhs, .. }
        | JsExpr::LogicalOr { lhs, rhs }
        | JsExpr::LogicalAnd { lhs, rhs }
        | JsExpr::In {
            key: lhs,
            object: rhs,
        }
        | JsExpr::Delete {
            object: lhs,
            key: rhs,
        }
        | JsExpr::NullCoalesceAssign {
            target: lhs,
            value: rhs,
        } => {
            subst_var_to_this_expr(lhs, var_name);
            subst_var_to_this_expr(rhs, var_name);
        }
        JsExpr::Unary { expr: e, .. }
        | JsExpr::Cast { expr: e, .. }
        | JsExpr::TypeCheck { expr: e, .. }
        | JsExpr::Not(e)
        | JsExpr::PostIncrement(e)
        | JsExpr::Spread(e)
        | JsExpr::TypeOf(e)
        | JsExpr::GeneratorResume(e)
        | JsExpr::NonNull(e) => subst_var_to_this_expr(e, var_name),
        JsExpr::Yield(opt) => {
            if let Some(e) = opt {
                subst_var_to_this_expr(e, var_name);
            }
        }
        JsExpr::Field { object, .. } => subst_var_to_this_expr(object, var_name),
        JsExpr::Index { collection, index } => {
            subst_var_to_this_expr(collection, var_name);
            subst_var_to_this_expr(index, var_name);
        }
        JsExpr::Call { callee, args } | JsExpr::New { callee, args } => {
            subst_var_to_this_expr(callee, var_name);
            for a in args {
                subst_var_to_this_expr(a, var_name);
            }
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            subst_var_to_this_expr(cond, var_name);
            subst_var_to_this_expr(then_val, var_name);
            subst_var_to_this_expr(else_val, var_name);
        }
        JsExpr::ArrayInit(elems) | JsExpr::TupleInit(elems) | JsExpr::SuperCall(elems) => {
            for e in elems {
                subst_var_to_this_expr(e, var_name);
            }
        }
        JsExpr::ObjectInit(pairs) => {
            for (_, e) in pairs {
                subst_var_to_this_expr(e, var_name);
            }
        }
        JsExpr::SuperMethodCall { args, .. }
        | JsExpr::GeneratorCreate { args, .. }
        | JsExpr::SystemCall { args, .. } => {
            for a in args {
                subst_var_to_this_expr(a, var_name);
            }
        }
        JsExpr::SuperSet { value, .. } => subst_var_to_this_expr(value, var_name),
        // Arrow functions inherit outer `this` — substitute inside them too.
        JsExpr::ArrowFunction { body, .. } => subst_var_to_this_stmts(body, var_name),
    }
}

/// Replace `this.field` with `ClassName.prototype.field` inside a `super()` argument.
/// AVM2 allows `this` before `super()`, but ES6 does not; method references live on
/// the prototype and are accessible without `this`.
pub(super) fn rewrite_this_to_prototype(expr: &mut JsExpr, class_name: &str) {
    // Replace cachedBind(this, X) with a lazy arrow function:
    //   (...args: any[]): any => { return X.apply(this, args); }
    // Arrow functions capture `this` lexically but only evaluate it when called,
    // which is after super() completes — so this is safe in super() arguments.
    {
        let is_as3_bind = matches!(
            expr,
            JsExpr::Call { callee, args }
            if matches!(callee.as_ref(), JsExpr::Var(n) if n == "cachedBind")
                && args.len() == 2
                && matches!(&args[0], JsExpr::This)
        );
        if is_as3_bind {
            let dummy = JsExpr::Literal(Constant::Null);
            let old = std::mem::replace(expr, dummy);
            if let JsExpr::Call { mut args, .. } = old {
                let method_ref = args.swap_remove(1);
                // Extract method name for the type assertion before rewriting.
                let cast_as = if let JsExpr::Field { field, .. } = &method_ref {
                    Some(format!("{class_name}['{field}']"))
                } else {
                    None
                };
                let mut method_ref = method_ref;
                rewrite_this_to_prototype(&mut method_ref, class_name);
                *expr = JsExpr::ArrowFunction {
                    params: vec![("args".to_string(), Type::Array(Box::new(Type::Dynamic)))],
                    return_ty: Type::Unknown,
                    body: vec![JsStmt::Return(Some(JsExpr::Call {
                        callee: Box::new(JsExpr::Field {
                            object: Box::new(method_ref),
                            field: "apply".to_string(),
                        }),
                        args: vec![
                            JsExpr::This,
                            JsExpr::Cast {
                                expr: Box::new(JsExpr::Var("args".to_string())),
                                ty: Type::Dynamic,
                                kind: CastKind::NullableCoerce,
                            },
                        ],
                    }))],
                    has_rest_param: true,
                    cast_as,
                    infer_param_types: false,
                };
            }
            return;
        }
    }

    match expr {
        JsExpr::Field { object, .. } if matches!(object.as_ref(), JsExpr::This) => {
            *object = Box::new(JsExpr::Field {
                object: Box::new(JsExpr::Var(class_name.to_string())),
                field: "prototype".to_string(),
            });
        }
        // Recurse into subexpressions.
        JsExpr::Field { object, .. } => rewrite_this_to_prototype(object, class_name),
        JsExpr::Binary { lhs, rhs, .. }
        | JsExpr::Cmp { lhs, rhs, .. }
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
            rewrite_this_to_prototype(lhs, class_name);
            rewrite_this_to_prototype(rhs, class_name);
        }
        JsExpr::Unary { expr: e, .. }
        | JsExpr::Cast { expr: e, .. }
        | JsExpr::TypeCheck { expr: e, .. }
        | JsExpr::Not(e)
        | JsExpr::PostIncrement(e)
        | JsExpr::Spread(e)
        | JsExpr::TypeOf(e)
        | JsExpr::GeneratorResume(e)
        | JsExpr::NonNull(e) => rewrite_this_to_prototype(e, class_name),
        JsExpr::Index { collection, index } => {
            rewrite_this_to_prototype(collection, class_name);
            rewrite_this_to_prototype(index, class_name);
        }
        JsExpr::Call { callee, args } | JsExpr::New { callee, args } => {
            rewrite_this_to_prototype(callee, class_name);
            for a in args {
                rewrite_this_to_prototype(a, class_name);
            }
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            rewrite_this_to_prototype(cond, class_name);
            rewrite_this_to_prototype(then_val, class_name);
            rewrite_this_to_prototype(else_val, class_name);
        }
        JsExpr::ArrayInit(elems) | JsExpr::TupleInit(elems) | JsExpr::SuperCall(elems) => {
            for e in elems {
                rewrite_this_to_prototype(e, class_name);
            }
        }
        JsExpr::ObjectInit(pairs) => {
            for (_, e) in pairs {
                rewrite_this_to_prototype(e, class_name);
            }
        }
        JsExpr::SuperMethodCall { args, .. }
        | JsExpr::GeneratorCreate { args, .. }
        | JsExpr::SystemCall { args, .. } => {
            for a in args {
                rewrite_this_to_prototype(a, class_name);
            }
        }
        JsExpr::SuperSet { value, .. } => rewrite_this_to_prototype(value, class_name),
        JsExpr::Yield(opt) => {
            if let Some(e) = opt {
                rewrite_this_to_prototype(e, class_name);
            }
        }
        // Arrow functions in super() args are extremely rare; skip recursion.
        JsExpr::ArrowFunction { .. } => {}
        JsExpr::NullCoalesceAssign { target, value } => {
            rewrite_this_to_prototype(target, class_name);
            rewrite_this_to_prototype(value, class_name);
        }
        // Leaves: no recursion needed.
        JsExpr::Literal(_)
        | JsExpr::Var(_)
        | JsExpr::This
        | JsExpr::Activation
        | JsExpr::SuperGet(_) => {}
    }
}

/// Check whether a statement declares or assigns any variable in `vars`.
/// Recurses into nested bodies (if/else, loops) to find assignments.
pub(super) fn stmt_writes_any(stmt: &JsStmt, vars: &HashSet<String>) -> bool {
    match stmt {
        JsStmt::VarDecl { name, .. } => vars.contains(name.as_str()),
        JsStmt::Assign {
            target: JsExpr::Var(name),
            ..
        }
        | JsStmt::CompoundAssign {
            target: JsExpr::Var(name),
            ..
        } => vars.contains(name.as_str()),
        JsStmt::If {
            then_body,
            else_body,
            ..
        } => {
            then_body.iter().any(|s| stmt_writes_any(s, vars))
                || else_body.iter().any(|s| stmt_writes_any(s, vars))
        }
        JsStmt::While { body, .. } | JsStmt::Loop { body } | JsStmt::ForOf { body, .. } => {
            body.iter().any(|s| stmt_writes_any(s, vars))
        }
        JsStmt::For {
            init, body, update, ..
        } => {
            init.iter().any(|s| stmt_writes_any(s, vars))
                || body.iter().any(|s| stmt_writes_any(s, vars))
                || update.iter().any(|s| stmt_writes_any(s, vars))
        }
        JsStmt::Dispatch { blocks, .. } => blocks
            .iter()
            .any(|(_, stmts)| stmts.iter().any(|s| stmt_writes_any(s, vars))),
        JsStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            cases
                .iter()
                .any(|(_, stmts)| stmts.iter().any(|s| stmt_writes_any(s, vars)))
                || default_body.iter().any(|s| stmt_writes_any(s, vars))
        }
        _ => false,
    }
}
