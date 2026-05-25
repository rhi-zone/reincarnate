// ---------------------------------------------------------------------------
// Late-bound type check rewriting + stateful call rewrites + global assignment rewriting
// ---------------------------------------------------------------------------

use std::collections::{HashMap, HashSet};

use reincarnate_core::ir::{CastKind, Constant, Type, Visibility};

use crate::js_ast::{JsExpr, JsStmt};

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
    // After resolve_js_function_types, named types are Instance(TypeId) —
    // use MODULE_TYPES thread-local to resolve the name for late-bound checks.
    let needs_rewrite = match expr {
        JsExpr::TypeCheck {
            ty: Type::Instance(id),
            ..
        } => {
            let short = crate::ast_printer::instance_type_short_name(*id);
            late_bound.contains(short.as_str())
        }
        JsExpr::Cast {
            ty: Type::Instance(id),
            kind: CastKind::NullableCoerce,
            ..
        } => {
            let short = crate::ast_printer::instance_type_short_name(*id);
            late_bound.contains(short.as_str())
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
                let qualified = match &ty {
                    Type::Instance(id) => {
                        let short = crate::ast_printer::instance_type_short_name(*id);
                        short_to_qualified
                            .get(short.as_str())
                            .cloned()
                            .unwrap_or(short)
                    }
                    _ => unreachable!(),
                };
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
                let qualified = match &ty {
                    Type::Instance(id) => {
                        let short = crate::ast_printer::instance_type_short_name(*id);
                        short_to_qualified
                            .get(short.as_str())
                            .cloned()
                            .unwrap_or(short)
                    }
                    _ => unreachable!(),
                };
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
        JsExpr::Assign { lhs, rhs } => {
            rewrite_late_bound_expr(lhs, late_bound, short_to_qualified);
            rewrite_late_bound_expr(rhs, late_bound, short_to_qualified);
        }
        JsExpr::ArrowFunction { body, .. } => {
            rewrite_late_bound_types(body, late_bound, short_to_qualified);
        }
        // Leaf nodes — nothing to recurse into.
        JsExpr::Literal(_)
        | JsExpr::Var(_)
        | JsExpr::Regex(_)
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
// Stateful runtime call rewriting — DELETED in Phase 3.
//
// The IR now carries `_rt: GameRuntime` as param 0 on every stateful runtime
// function and the call translator prepends `_rt_val` as arg 0 at each call
// site.  `crate::lower::lower_call` consults `LowerCtx::stateful_names` and
// emits the call as `arg0.fname(rest_args)` directly — no post-pass AST
// rewriting required.
//
// NOTE: this function also previously rewrote bare `JsExpr::Var("global")` /
// `JsExpr::Var("other")` references to `_rt.global` / `_rt.other`.  Phase 3
// does not address those — they should be handled by the engine-specific
// rewrite pass that introduces them (e.g. via @@Global@@ / @@Other@@ Op::Call
// lowering).  If you observe regressions on bare `global` / `other` access,
// re-introduce a dedicated pass instead of resurrecting this monolithic one.
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
