// ---------------------------------------------------------------------------
// Late-bound type check rewriting + stateful call rewrites + global assignment rewriting
// ---------------------------------------------------------------------------

use std::collections::{BTreeSet, HashMap, HashSet};

use reincarnate_core::ir::{CastKind, Constant, Type, Visibility};

use crate::js_ast::{JsExpr, JsStmt};

use super::sanitize_ident;

// ---------------------------------------------------------------------------
// Late-bound type check rewriting
// ---------------------------------------------------------------------------

/// Replace `isType(x, Foo)` / `asType(x, Foo)` with late-bound variants using
/// `getDefinitionByName("qualified::Name")` for types whose static import would
/// create a circular dependency.
pub(super) fn rewrite_late_bound_types(
    body: &mut [JsStmt],
    late_bound: &HashSet<String>,
    short_to_qualified: &HashMap<String, String>,
) {
    if late_bound.is_empty() {
        return;
    }
    for stmt in body.iter_mut() {
        rewrite_late_bound_stmt(stmt, late_bound, short_to_qualified);
    }
}

fn rewrite_late_bound_stmt(
    stmt: &mut JsStmt,
    late_bound: &HashSet<String>,
    short_to_qualified: &HashMap<String, String>,
) {
    match stmt {
        JsStmt::VarDecl {
            init: Some(expr), ..
        } => {
            rewrite_late_bound_expr(expr, late_bound, short_to_qualified);
        }
        JsStmt::Assign { target, value } => {
            rewrite_late_bound_expr(target, late_bound, short_to_qualified);
            rewrite_late_bound_expr(value, late_bound, short_to_qualified);
        }
        JsStmt::CompoundAssign { target, value, .. } => {
            rewrite_late_bound_expr(target, late_bound, short_to_qualified);
            rewrite_late_bound_expr(value, late_bound, short_to_qualified);
        }
        JsStmt::Expr(expr) | JsStmt::Throw(expr) => {
            rewrite_late_bound_expr(expr, late_bound, short_to_qualified);
        }
        JsStmt::Return(Some(expr)) => {
            rewrite_late_bound_expr(expr, late_bound, short_to_qualified);
        }
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            rewrite_late_bound_expr(cond, late_bound, short_to_qualified);
            rewrite_late_bound_types(then_body, late_bound, short_to_qualified);
            rewrite_late_bound_types(else_body, late_bound, short_to_qualified);
        }
        JsStmt::While { cond, body } => {
            rewrite_late_bound_expr(cond, late_bound, short_to_qualified);
            rewrite_late_bound_types(body, late_bound, short_to_qualified);
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            rewrite_late_bound_types(init, late_bound, short_to_qualified);
            rewrite_late_bound_expr(cond, late_bound, short_to_qualified);
            rewrite_late_bound_types(update, late_bound, short_to_qualified);
            rewrite_late_bound_types(body, late_bound, short_to_qualified);
        }
        JsStmt::Loop { body } => {
            rewrite_late_bound_types(body, late_bound, short_to_qualified);
        }
        JsStmt::ForOf { iterable, body, .. } => {
            rewrite_late_bound_expr(iterable, late_bound, short_to_qualified);
            rewrite_late_bound_types(body, late_bound, short_to_qualified);
        }
        JsStmt::Dispatch { blocks, .. } => {
            for (_, stmts) in blocks {
                rewrite_late_bound_types(stmts, late_bound, short_to_qualified);
            }
        }
        JsStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            for (_, body) in cases {
                rewrite_late_bound_types(body, late_bound, short_to_qualified);
            }
            rewrite_late_bound_types(default_body, late_bound, short_to_qualified);
        }
        // Remaining leaf statements — no nested type references to rewrite.
        JsStmt::VarDecl { init: None, .. }
        | JsStmt::Return(None)
        | JsStmt::Break
        | JsStmt::Continue
        | JsStmt::LabeledBreak { .. } => {}
    }
}

pub(super) fn rewrite_late_bound_expr(
    expr: &mut JsExpr,
    late_bound: &HashSet<String>,
    short_to_qualified: &HashMap<String, String>,
) {
    // First, check if this expression itself needs rewriting.
    let needs_rewrite = match expr {
        JsExpr::TypeCheck {
            ty: Type::Struct(name) | Type::Enum(name),
            ..
        } => {
            let short = name.rsplit("::").next().unwrap_or(name);
            late_bound.contains(short)
        }
        JsExpr::Cast {
            ty: Type::Struct(name) | Type::Enum(name),
            kind: CastKind::NullableCoerce,
            ..
        } => {
            let short = name.rsplit("::").next().unwrap_or(name);
            late_bound.contains(short)
        }
        _ => false,
    };

    if needs_rewrite {
        let dummy = JsExpr::Literal(Constant::Null);
        let old = std::mem::replace(expr, dummy);
        match old {
            JsExpr::TypeCheck {
                expr: inner, ty, ..
            } => {
                let name = match &ty {
                    Type::Struct(n) | Type::Enum(n) => n,
                    _ => unreachable!(),
                };
                let short = name.rsplit("::").next().unwrap_or(name);
                let qualified = short_to_qualified
                    .get(short)
                    .map_or_else(|| name.clone(), |q| q.clone());
                *expr = JsExpr::Call {
                    callee: Box::new(JsExpr::Var("isType".into())),
                    args: vec![
                        *inner,
                        JsExpr::Call {
                            callee: Box::new(JsExpr::Var("getDefinitionByName".into())),
                            args: vec![JsExpr::Literal(Constant::String(qualified))],
                        },
                    ],
                };
            }
            JsExpr::Cast {
                expr: inner, ty, ..
            } => {
                let name = match &ty {
                    Type::Struct(n) | Type::Enum(n) => n,
                    _ => unreachable!(),
                };
                let short = name.rsplit("::").next().unwrap_or(name);
                let qualified = short_to_qualified
                    .get(short)
                    .map_or_else(|| name.clone(), |q| q.clone());
                *expr = JsExpr::Call {
                    callee: Box::new(JsExpr::Var("asType".into())),
                    args: vec![
                        *inner,
                        JsExpr::Call {
                            callee: Box::new(JsExpr::Var("getDefinitionByName".into())),
                            args: vec![JsExpr::Literal(Constant::String(qualified))],
                        },
                    ],
                };
            }
            _ => unreachable!(),
        }
        // Recurse into the newly created expression's children.
        rewrite_late_bound_expr(expr, late_bound, short_to_qualified);
        return;
    }

    // Recurse into child expressions.
    match expr {
        JsExpr::Binary { lhs, rhs, .. }
        | JsExpr::Cmp { lhs, rhs, .. }
        | JsExpr::LooseEq { lhs, rhs }
        | JsExpr::LooseNe { lhs, rhs }
        | JsExpr::LogicalOr { lhs, rhs }
        | JsExpr::LogicalAnd { lhs, rhs } => {
            rewrite_late_bound_expr(lhs, late_bound, short_to_qualified);
            rewrite_late_bound_expr(rhs, late_bound, short_to_qualified);
        }
        JsExpr::Unary { expr: inner, .. }
        | JsExpr::Not(inner)
        | JsExpr::PostIncrement(inner)
        | JsExpr::Spread(inner)
        | JsExpr::TypeOf(inner)
        | JsExpr::GeneratorResume(inner)
        | JsExpr::NonNull(inner) => {
            rewrite_late_bound_expr(inner, late_bound, short_to_qualified);
        }
        JsExpr::Cast { expr: inner, .. } | JsExpr::TypeCheck { expr: inner, .. } => {
            rewrite_late_bound_expr(inner, late_bound, short_to_qualified);
        }
        JsExpr::Field { object, .. } => {
            rewrite_late_bound_expr(object, late_bound, short_to_qualified);
        }
        JsExpr::Index { collection, index } => {
            rewrite_late_bound_expr(collection, late_bound, short_to_qualified);
            rewrite_late_bound_expr(index, late_bound, short_to_qualified);
        }
        JsExpr::Call { callee, args } | JsExpr::New { callee, args } => {
            rewrite_late_bound_expr(callee, late_bound, short_to_qualified);
            for arg in args {
                rewrite_late_bound_expr(arg, late_bound, short_to_qualified);
            }
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            rewrite_late_bound_expr(cond, late_bound, short_to_qualified);
            rewrite_late_bound_expr(then_val, late_bound, short_to_qualified);
            rewrite_late_bound_expr(else_val, late_bound, short_to_qualified);
        }
        JsExpr::ArrayInit(elems) | JsExpr::TupleInit(elems) => {
            for elem in elems {
                rewrite_late_bound_expr(elem, late_bound, short_to_qualified);
            }
        }
        JsExpr::ObjectInit(pairs) => {
            for (_, val) in pairs {
                rewrite_late_bound_expr(val, late_bound, short_to_qualified);
            }
        }
        JsExpr::In { key, object } | JsExpr::Delete { object, key } => {
            rewrite_late_bound_expr(key, late_bound, short_to_qualified);
            rewrite_late_bound_expr(object, late_bound, short_to_qualified);
        }
        JsExpr::SuperCall(args) | JsExpr::GeneratorCreate { args, .. } => {
            for arg in args {
                rewrite_late_bound_expr(arg, late_bound, short_to_qualified);
            }
        }
        JsExpr::SuperMethodCall { args, .. } => {
            for arg in args {
                rewrite_late_bound_expr(arg, late_bound, short_to_qualified);
            }
        }
        JsExpr::SuperSet { value, .. } => {
            rewrite_late_bound_expr(value, late_bound, short_to_qualified);
        }
        JsExpr::Yield(Some(inner)) => {
            rewrite_late_bound_expr(inner, late_bound, short_to_qualified);
        }
        JsExpr::SystemCall { args, .. } => {
            for arg in args {
                rewrite_late_bound_expr(arg, late_bound, short_to_qualified);
            }
        }
        JsExpr::NullCoalesceAssign { target, value } => {
            rewrite_late_bound_expr(target, late_bound, short_to_qualified);
            rewrite_late_bound_expr(value, late_bound, short_to_qualified);
        }
        JsExpr::ArrowFunction { body, .. } => {
            rewrite_late_bound_types(body, late_bound, short_to_qualified);
        }
        // Leaf nodes — nothing to recurse into.
        JsExpr::Literal(_)
        | JsExpr::Var(_)
        | JsExpr::This
        | JsExpr::Activation
        | JsExpr::SuperGet(_)
        | JsExpr::Yield(None) => {}
    }
}

// ---------------------------------------------------------------------------
// Global assignment rewriting (ESM compatibility)
// ---------------------------------------------------------------------------

/// Rewrite assignments to mutable globals into setter function calls.
///
/// ES modules make `import { x }` a read-only binding. To write to `x` from
/// another module, the exporting module provides `$set_x(v)`. This pass
/// rewrites `x = v` → `$set_x(v)` and `x op= v` → `$set_x(x op v)`.
///
/// Skips variables that have a local `VarDecl` in an enclosing or current scope,
/// since those shadow the global name.
pub(super) fn rewrite_global_assignments(body: &mut [JsStmt], mutable_globals: &HashSet<String>) {
    if mutable_globals.is_empty() {
        return;
    }
    rewrite_global_assignments_inner(body, mutable_globals, &HashSet::new());
}

fn rewrite_global_assignments_inner(
    body: &mut [JsStmt],
    mutable_globals: &HashSet<String>,
    parent_locals: &HashSet<String>,
) {
    // Accumulate local declarations from this scope and parent scopes.
    let mut local_decls = parent_locals.clone();
    for s in body.iter() {
        if let JsStmt::VarDecl { name, .. } = s {
            local_decls.insert(name.clone());
        }
    }

    for stmt in body.iter_mut() {
        match stmt {
            JsStmt::Assign {
                target: JsExpr::Var(name),
                ..
            } if mutable_globals.contains(name.as_str())
                && !local_decls.contains(name.as_str()) =>
            {
                // Replace: `name = value` → `$set_name(value)`
                let setter = format!("$set_{name}");
                let dummy = JsExpr::Literal(Constant::Null);
                if let JsStmt::Assign { value, .. } =
                    std::mem::replace(stmt, JsStmt::Expr(dummy.clone()))
                {
                    *stmt = JsStmt::Expr(JsExpr::Call {
                        callee: Box::new(JsExpr::Var(setter)),
                        args: vec![value],
                    });
                }
            }
            JsStmt::CompoundAssign {
                target: JsExpr::Var(name),
                ..
            } if mutable_globals.contains(name.as_str())
                && !local_decls.contains(name.as_str()) =>
            {
                // Replace: `name op= value` → `$set_name(name op value)`
                let var_name = name.clone();
                let setter = format!("$set_{var_name}");
                let dummy = JsExpr::Literal(Constant::Null);
                if let JsStmt::CompoundAssign { op, value, .. } =
                    std::mem::replace(stmt, JsStmt::Expr(dummy.clone()))
                {
                    *stmt = JsStmt::Expr(JsExpr::Call {
                        callee: Box::new(JsExpr::Var(setter)),
                        args: vec![JsExpr::Binary {
                            op,
                            lhs: Box::new(JsExpr::Var(var_name)),
                            rhs: Box::new(value),
                        }],
                    });
                }
            }
            // Recurse into nested bodies.
            JsStmt::If {
                then_body,
                else_body,
                ..
            } => {
                rewrite_global_assignments_inner(then_body, mutable_globals, &local_decls);
                rewrite_global_assignments_inner(else_body, mutable_globals, &local_decls);
            }
            JsStmt::While { body, .. } | JsStmt::Loop { body } | JsStmt::ForOf { body, .. } => {
                rewrite_global_assignments_inner(body, mutable_globals, &local_decls);
            }
            JsStmt::For {
                init, body, update, ..
            } => {
                rewrite_global_assignments_inner(init, mutable_globals, &local_decls);
                rewrite_global_assignments_inner(body, mutable_globals, &local_decls);
                rewrite_global_assignments_inner(update, mutable_globals, &local_decls);
            }
            JsStmt::Dispatch { blocks, .. } => {
                for (_, stmts) in blocks {
                    rewrite_global_assignments_inner(stmts, mutable_globals, &local_decls);
                }
            }
            JsStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, body) in cases {
                    rewrite_global_assignments_inner(body, mutable_globals, &local_decls);
                }
                rewrite_global_assignments_inner(default_body, mutable_globals, &local_decls);
            }
            // Leaf statements and non-matching forms — no nested assignments to rewrite.
            JsStmt::VarDecl { .. }
            | JsStmt::Assign { .. }
            | JsStmt::CompoundAssign { .. }
            | JsStmt::Expr(_)
            | JsStmt::Return(_)
            | JsStmt::Break
            | JsStmt::Continue
            | JsStmt::LabeledBreak { .. }
            | JsStmt::Throw(_) => {}
        }
    }
}

// ---------------------------------------------------------------------------
// Stateful runtime call rewriting
// ---------------------------------------------------------------------------

/// Rewrite bare stateful runtime calls to qualified `_rt.foo(args)` form.
///
/// Walks the statement tree and replaces every `JsExpr::Call { callee: JsExpr::Var(name), ... }`
/// where `name` is in `stateful_names` with a qualified call:
/// - `from_class = false`: `_rt.foo(args)` (free function context, `_rt` is a parameter)
/// - `from_class = true`: `this._rt.foo(args)` (class method context)
pub(super) fn rewrite_stateful_calls(
    stmts: &mut [JsStmt],
    stateful_names: &BTreeSet<String>,
    from_class: bool,
) {
    for stmt in stmts.iter_mut() {
        rewrite_stateful_calls_stmt(stmt, stateful_names, from_class);
    }
}

fn rewrite_stateful_calls_stmt(
    stmt: &mut JsStmt,
    stateful_names: &BTreeSet<String>,
    from_class: bool,
) {
    match stmt {
        JsStmt::VarDecl { init, .. } => {
            if let Some(expr) = init {
                rewrite_stateful_calls_expr(expr, stateful_names, from_class);
            }
        }
        JsStmt::Assign { target, value } => {
            rewrite_stateful_calls_expr(target, stateful_names, from_class);
            rewrite_stateful_calls_expr(value, stateful_names, from_class);
        }
        JsStmt::CompoundAssign { target, value, .. } => {
            rewrite_stateful_calls_expr(target, stateful_names, from_class);
            rewrite_stateful_calls_expr(value, stateful_names, from_class);
        }
        JsStmt::Expr(e) => rewrite_stateful_calls_expr(e, stateful_names, from_class),
        JsStmt::Return(Some(e)) => rewrite_stateful_calls_expr(e, stateful_names, from_class),
        JsStmt::Return(None) => {}
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            rewrite_stateful_calls_expr(cond, stateful_names, from_class);
            rewrite_stateful_calls(then_body, stateful_names, from_class);
            rewrite_stateful_calls(else_body, stateful_names, from_class);
        }
        JsStmt::While { cond, body } => {
            rewrite_stateful_calls_expr(cond, stateful_names, from_class);
            rewrite_stateful_calls(body, stateful_names, from_class);
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            rewrite_stateful_calls(init, stateful_names, from_class);
            rewrite_stateful_calls_expr(cond, stateful_names, from_class);
            rewrite_stateful_calls(update, stateful_names, from_class);
            rewrite_stateful_calls(body, stateful_names, from_class);
        }
        JsStmt::Loop { body } => {
            rewrite_stateful_calls(body, stateful_names, from_class);
        }
        JsStmt::ForOf { iterable, body, .. } => {
            rewrite_stateful_calls_expr(iterable, stateful_names, from_class);
            rewrite_stateful_calls(body, stateful_names, from_class);
        }
        JsStmt::Throw(e) => rewrite_stateful_calls_expr(e, stateful_names, from_class),
        JsStmt::Dispatch { blocks, .. } => {
            for (_, stmts) in blocks {
                rewrite_stateful_calls(stmts, stateful_names, from_class);
            }
        }
        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            rewrite_stateful_calls_expr(value, stateful_names, from_class);
            for (_, stmts) in cases {
                rewrite_stateful_calls(stmts, stateful_names, from_class);
            }
            rewrite_stateful_calls(default_body, stateful_names, from_class);
        }
        JsStmt::Break | JsStmt::Continue | JsStmt::LabeledBreak { .. } => {}
    }
}

fn rewrite_stateful_calls_expr(
    expr: &mut JsExpr,
    stateful_names: &BTreeSet<String>,
    from_class: bool,
) {
    match expr {
        JsExpr::Binary { lhs, rhs, .. }
        | JsExpr::Cmp { lhs, rhs, .. }
        | JsExpr::LooseEq { lhs, rhs }
        | JsExpr::LooseNe { lhs, rhs } => {
            rewrite_stateful_calls_expr(lhs, stateful_names, from_class);
            rewrite_stateful_calls_expr(rhs, stateful_names, from_class);
        }
        JsExpr::LogicalOr { lhs, rhs } | JsExpr::LogicalAnd { lhs, rhs } => {
            rewrite_stateful_calls_expr(lhs, stateful_names, from_class);
            rewrite_stateful_calls_expr(rhs, stateful_names, from_class);
        }
        JsExpr::Unary { expr: inner, .. }
        | JsExpr::Not(inner)
        | JsExpr::PostIncrement(inner)
        | JsExpr::Spread(inner)
        | JsExpr::TypeOf(inner)
        | JsExpr::NonNull(inner)
        | JsExpr::Cast { expr: inner, .. }
        | JsExpr::TypeCheck { expr: inner, .. } => {
            rewrite_stateful_calls_expr(inner, stateful_names, from_class);
        }
        JsExpr::Field { object, .. } => {
            rewrite_stateful_calls_expr(object, stateful_names, from_class)
        }
        JsExpr::Index { collection, index } => {
            rewrite_stateful_calls_expr(collection, stateful_names, from_class);
            rewrite_stateful_calls_expr(index, stateful_names, from_class);
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            rewrite_stateful_calls_expr(cond, stateful_names, from_class);
            rewrite_stateful_calls_expr(then_val, stateful_names, from_class);
            rewrite_stateful_calls_expr(else_val, stateful_names, from_class);
        }
        JsExpr::ArrayInit(items) | JsExpr::TupleInit(items) => {
            for item in items.iter_mut() {
                rewrite_stateful_calls_expr(item, stateful_names, from_class);
            }
        }
        JsExpr::ObjectInit(fields) => {
            for (_, val) in fields.iter_mut() {
                rewrite_stateful_calls_expr(val, stateful_names, from_class);
            }
        }
        JsExpr::New { callee, args } => {
            rewrite_stateful_calls_expr(callee, stateful_names, from_class);
            for arg in args.iter_mut() {
                rewrite_stateful_calls_expr(arg, stateful_names, from_class);
            }
        }
        JsExpr::In { key, object } => {
            rewrite_stateful_calls_expr(key, stateful_names, from_class);
            rewrite_stateful_calls_expr(object, stateful_names, from_class);
        }
        JsExpr::Delete { object, key } => {
            rewrite_stateful_calls_expr(object, stateful_names, from_class);
            rewrite_stateful_calls_expr(key, stateful_names, from_class);
        }
        JsExpr::ArrowFunction { body, .. } => {
            rewrite_stateful_calls(body, stateful_names, from_class);
        }
        JsExpr::SuperCall(args) | JsExpr::SuperMethodCall { args, .. } => {
            for arg in args.iter_mut() {
                rewrite_stateful_calls_expr(arg, stateful_names, from_class);
            }
        }
        JsExpr::SuperSet { value, .. } => {
            rewrite_stateful_calls_expr(value, stateful_names, from_class)
        }
        JsExpr::NullCoalesceAssign { target, value } => {
            rewrite_stateful_calls_expr(target, stateful_names, from_class);
            rewrite_stateful_calls_expr(value, stateful_names, from_class);
        }
        JsExpr::GeneratorCreate { args, .. } => {
            for arg in args.iter_mut() {
                rewrite_stateful_calls_expr(arg, stateful_names, from_class);
            }
        }
        JsExpr::GeneratorResume(inner) => {
            rewrite_stateful_calls_expr(inner, stateful_names, from_class)
        }
        JsExpr::Yield(inner) => {
            if let Some(e) = inner {
                rewrite_stateful_calls_expr(e, stateful_names, from_class);
            }
        }
        JsExpr::SystemCall { args, .. } => {
            for arg in args.iter_mut() {
                rewrite_stateful_calls_expr(arg, stateful_names, from_class);
            }
        }
        JsExpr::Call { callee, args } => {
            // If the callee is a bare Var that names a stateful runtime function,
            // replace it with a qualified method call: `_rt.foo(args)` or `this._rt.foo(args)`.
            // The Var name may be unsanitized (e.g. `@@SetStatic@@`) while `stateful_names`
            // holds sanitized names (`__SetStatic__`) — sanitize before the lookup.
            if let JsExpr::Var(name) = callee.as_ref() {
                let sanitized = sanitize_ident(name);
                if stateful_names.contains(&sanitized) {
                    let rt_expr = if from_class {
                        JsExpr::Field {
                            object: Box::new(JsExpr::This),
                            field: "_rt".into(),
                        }
                    } else {
                        JsExpr::Var("_rt".into())
                    };
                    *callee = Box::new(JsExpr::Field {
                        object: Box::new(rt_expr),
                        field: sanitized,
                    });
                }
            }
            // Recurse into callee and args (callee may have been replaced, recurse anyway).
            rewrite_stateful_calls_expr(callee, stateful_names, from_class);
            for arg in args.iter_mut() {
                rewrite_stateful_calls_expr(arg, stateful_names, from_class);
            }
        }
        // `global` and `other` are properties on the runtime, not local variables.
        // Rewrite bare `global` → `_rt.global` / `this._rt.global`
        //          and `other`  → `_rt.other`  / `this._rt.other`.
        JsExpr::Var(name) if name == "global" || name == "other" => {
            let rt_expr = if from_class {
                JsExpr::Field {
                    object: Box::new(JsExpr::This),
                    field: "_rt".into(),
                }
            } else {
                JsExpr::Var("_rt".into())
            };
            *expr = JsExpr::Field {
                object: Box::new(rt_expr),
                field: "global".into(),
            };
        }
        // Leaf nodes — nothing to recurse into.
        JsExpr::Literal(_)
        | JsExpr::Var(_)
        | JsExpr::This
        | JsExpr::Activation
        | JsExpr::SuperGet(_) => {}
    }
}

// ---------------------------------------------------------------------------
// Prepend _rt argument to free function calls
// ---------------------------------------------------------------------------

/// Prepend a runtime argument to calls to free functions.
///
/// When `from_class` is true, prepends `this._rt`; when false, prepends `_rt`.
pub(super) fn prepend_rt_arg_to_free_calls(
    stmts: &mut [JsStmt],
    free_func_names: &HashSet<String>,
    from_class: bool,
) {
    for stmt in stmts.iter_mut() {
        prepend_rt_arg_stmt(stmt, free_func_names, from_class);
    }
}

fn prepend_rt_arg_stmt(stmt: &mut JsStmt, free_func_names: &HashSet<String>, from_class: bool) {
    match stmt {
        JsStmt::VarDecl { init, .. } => {
            if let Some(expr) = init {
                prepend_rt_arg_expr(expr, free_func_names, from_class);
            }
        }
        JsStmt::Assign { target, value } => {
            prepend_rt_arg_expr(target, free_func_names, from_class);
            prepend_rt_arg_expr(value, free_func_names, from_class);
        }
        JsStmt::CompoundAssign { target, value, .. } => {
            prepend_rt_arg_expr(target, free_func_names, from_class);
            prepend_rt_arg_expr(value, free_func_names, from_class);
        }
        JsStmt::Expr(e) => prepend_rt_arg_expr(e, free_func_names, from_class),
        JsStmt::Return(Some(e)) => prepend_rt_arg_expr(e, free_func_names, from_class),
        JsStmt::Return(None) => {}
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            prepend_rt_arg_expr(cond, free_func_names, from_class);
            prepend_rt_arg_to_free_calls(then_body, free_func_names, from_class);
            prepend_rt_arg_to_free_calls(else_body, free_func_names, from_class);
        }
        JsStmt::While { cond, body } => {
            prepend_rt_arg_expr(cond, free_func_names, from_class);
            prepend_rt_arg_to_free_calls(body, free_func_names, from_class);
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            prepend_rt_arg_to_free_calls(init, free_func_names, from_class);
            prepend_rt_arg_expr(cond, free_func_names, from_class);
            prepend_rt_arg_to_free_calls(update, free_func_names, from_class);
            prepend_rt_arg_to_free_calls(body, free_func_names, from_class);
        }
        JsStmt::Loop { body } => {
            prepend_rt_arg_to_free_calls(body, free_func_names, from_class);
        }
        JsStmt::ForOf { iterable, body, .. } => {
            prepend_rt_arg_expr(iterable, free_func_names, from_class);
            prepend_rt_arg_to_free_calls(body, free_func_names, from_class);
        }
        JsStmt::Throw(e) => prepend_rt_arg_expr(e, free_func_names, from_class),
        JsStmt::Dispatch { blocks, .. } => {
            for (_, stmts) in blocks {
                prepend_rt_arg_to_free_calls(stmts, free_func_names, from_class);
            }
        }
        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            prepend_rt_arg_expr(value, free_func_names, from_class);
            for (_, stmts) in cases {
                prepend_rt_arg_to_free_calls(stmts, free_func_names, from_class);
            }
            prepend_rt_arg_to_free_calls(default_body, free_func_names, from_class);
        }
        JsStmt::Break | JsStmt::Continue | JsStmt::LabeledBreak { .. } => {}
    }
}

fn prepend_rt_arg_expr(expr: &mut JsExpr, free_func_names: &HashSet<String>, from_class: bool) {
    // Recurse into children first.
    match expr {
        JsExpr::Binary { lhs, rhs, .. }
        | JsExpr::Cmp { lhs, rhs, .. }
        | JsExpr::LooseEq { lhs, rhs }
        | JsExpr::LooseNe { lhs, rhs } => {
            prepend_rt_arg_expr(lhs, free_func_names, from_class);
            prepend_rt_arg_expr(rhs, free_func_names, from_class);
        }
        JsExpr::LogicalOr { lhs, rhs } | JsExpr::LogicalAnd { lhs, rhs } => {
            prepend_rt_arg_expr(lhs, free_func_names, from_class);
            prepend_rt_arg_expr(rhs, free_func_names, from_class);
        }
        JsExpr::Unary { expr: inner, .. }
        | JsExpr::Not(inner)
        | JsExpr::PostIncrement(inner)
        | JsExpr::Spread(inner)
        | JsExpr::TypeOf(inner)
        | JsExpr::NonNull(inner)
        | JsExpr::Cast { expr: inner, .. }
        | JsExpr::TypeCheck { expr: inner, .. } => {
            prepend_rt_arg_expr(inner, free_func_names, from_class);
        }
        JsExpr::Field { object, .. } => prepend_rt_arg_expr(object, free_func_names, from_class),
        JsExpr::Index { collection, index } => {
            prepend_rt_arg_expr(collection, free_func_names, from_class);
            prepend_rt_arg_expr(index, free_func_names, from_class);
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            prepend_rt_arg_expr(cond, free_func_names, from_class);
            prepend_rt_arg_expr(then_val, free_func_names, from_class);
            prepend_rt_arg_expr(else_val, free_func_names, from_class);
        }
        JsExpr::ArrayInit(items) | JsExpr::TupleInit(items) => {
            for item in items.iter_mut() {
                prepend_rt_arg_expr(item, free_func_names, from_class);
            }
        }
        JsExpr::ObjectInit(fields) => {
            for (_, val) in fields.iter_mut() {
                prepend_rt_arg_expr(val, free_func_names, from_class);
            }
        }
        JsExpr::New { callee, args } => {
            prepend_rt_arg_expr(callee, free_func_names, from_class);
            for arg in args.iter_mut() {
                prepend_rt_arg_expr(arg, free_func_names, from_class);
            }
        }
        JsExpr::In { key, object } => {
            prepend_rt_arg_expr(key, free_func_names, from_class);
            prepend_rt_arg_expr(object, free_func_names, from_class);
        }
        JsExpr::Delete { object, key } => {
            prepend_rt_arg_expr(object, free_func_names, from_class);
            prepend_rt_arg_expr(key, free_func_names, from_class);
        }
        JsExpr::ArrowFunction { body, .. } => {
            prepend_rt_arg_to_free_calls(body, free_func_names, from_class);
        }
        JsExpr::SuperCall(args) | JsExpr::SuperMethodCall { args, .. } => {
            for arg in args.iter_mut() {
                prepend_rt_arg_expr(arg, free_func_names, from_class);
            }
        }
        JsExpr::SuperSet { value, .. } => prepend_rt_arg_expr(value, free_func_names, from_class),
        JsExpr::NullCoalesceAssign { target, value } => {
            prepend_rt_arg_expr(target, free_func_names, from_class);
            prepend_rt_arg_expr(value, free_func_names, from_class);
        }
        JsExpr::GeneratorCreate { args, .. } => {
            for arg in args.iter_mut() {
                prepend_rt_arg_expr(arg, free_func_names, from_class);
            }
        }
        JsExpr::GeneratorResume(inner) => prepend_rt_arg_expr(inner, free_func_names, from_class),
        JsExpr::Yield(inner) => {
            if let Some(e) = inner {
                prepend_rt_arg_expr(e, free_func_names, from_class);
            }
        }
        JsExpr::SystemCall { args, .. } => {
            for arg in args.iter_mut() {
                prepend_rt_arg_expr(arg, free_func_names, from_class);
            }
        }
        JsExpr::Call { callee, args } => {
            // Check if this is a call to a free function.
            if let JsExpr::Var(name) = callee.as_ref() {
                if free_func_names.contains(name) {
                    let rt_arg = if from_class {
                        JsExpr::Field {
                            object: Box::new(JsExpr::This),
                            field: "_rt".into(),
                        }
                    } else {
                        JsExpr::Var("_rt".into())
                    };
                    args.insert(0, rt_arg);
                }
            }
            // Recurse into callee and args.
            prepend_rt_arg_expr(callee, free_func_names, from_class);
            for arg in args.iter_mut() {
                prepend_rt_arg_expr(arg, free_func_names, from_class);
            }
        }
        // Leaf nodes — nothing to recurse into.
        JsExpr::Literal(_)
        | JsExpr::Var(_)
        | JsExpr::This
        | JsExpr::Activation
        | JsExpr::SuperGet(_) => {}
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Whether a cinit statement is a redundant assignment to a field that already
/// has a `static readonly` default value on the class.
pub(super) fn is_redundant_static_assign(stmt: &JsStmt, const_fields: &HashSet<String>) -> bool {
    if let JsStmt::Assign {
        target: JsExpr::Field { object, field },
        ..
    } = stmt
    {
        matches!(**object, JsExpr::This) && const_fields.contains(field)
    } else {
        false
    }
}

pub(super) fn visibility_prefix(vis: Visibility) -> &'static str {
    match vis {
        Visibility::Public => "export ",
        Visibility::Private | Visibility::Protected => "",
    }
}
