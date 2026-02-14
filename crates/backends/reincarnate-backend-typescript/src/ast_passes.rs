//! Engine-agnostic AST passes for the TypeScript backend.
//!
//! These run after engine-specific rewrites and before printing.

use reincarnate_core::ir::inst::CmpKind;
use reincarnate_core::ir::value::Constant;

use crate::js_ast::{JsExpr, JsStmt};

/// Recover `switch` statements from if-chains where every condition compares
/// the same expression against a distinct constant.
///
/// Handles two patterns:
/// - **Sequential if**: consecutive `if (EXPR === C) { ... }` with empty else
/// - **Nested if-else-if**: `if (EXPR === C1) { ... } else if (EXPR === C2) { ... } else { ... }`
///
/// NOTE: This transformation may not be semantics-preserving if the discriminant
/// expression has side effects, since `switch` evaluates it once while the
/// original if-chain evaluates it N times. We only apply this when
/// `is_stable_expr` returns true, but that check is conservative and syntactic
/// — it cannot rule out all side effects (e.g. getters on fields).
pub fn recover_switch_statements(body: &mut Vec<JsStmt>) {
    // First, recurse into all nested bodies.
    for stmt in body.iter_mut() {
        recurse_into_stmt(stmt);
    }

    // Try the nested if-else-if pattern on individual statements.
    for stmt in body.iter_mut() {
        try_recover_nested_if_else(stmt);
    }

    // Try the sequential-if pattern on runs of consecutive statements.
    try_recover_sequential_ifs(body);
}

/// Recurse into all sub-bodies of a statement.
fn recurse_into_stmt(stmt: &mut JsStmt) {
    match stmt {
        JsStmt::If {
            then_body,
            else_body,
            ..
        } => {
            recover_switch_statements(then_body);
            recover_switch_statements(else_body);
        }
        JsStmt::While { body, .. }
        | JsStmt::Loop { body }
        | JsStmt::ForOf { body, .. } => {
            recover_switch_statements(body);
        }
        JsStmt::For {
            init,
            body,
            update,
            ..
        } => {
            recover_switch_statements(init);
            recover_switch_statements(body);
            recover_switch_statements(update);
        }
        JsStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            for (_, case_body) in cases {
                recover_switch_statements(case_body);
            }
            recover_switch_statements(default_body);
        }
        JsStmt::Dispatch { blocks, .. } => {
            for (_, block_body) in blocks {
                recover_switch_statements(block_body);
            }
        }
        // Recurse into expressions that contain statement bodies (e.g. arrow fns).
        JsStmt::VarDecl { init: Some(e), .. }
        | JsStmt::Assign { value: e, .. }
        | JsStmt::Expr(e)
        | JsStmt::Return(Some(e))
        | JsStmt::Throw(e) => {
            recurse_into_expr(e);
        }
        JsStmt::CompoundAssign { value, .. } => {
            recurse_into_expr(value);
        }
        _ => {}
    }
}

/// Recurse into an expression to find nested statement bodies (arrow functions).
fn recurse_into_expr(expr: &mut JsExpr) {
    match expr {
        JsExpr::ArrowFunction { body, .. } => {
            recover_switch_statements(body);
        }
        JsExpr::Binary { lhs, rhs, .. }
        | JsExpr::Cmp { lhs, rhs, .. }
        | JsExpr::LogicalOr { lhs, rhs }
        | JsExpr::LogicalAnd { lhs, rhs } => {
            recurse_into_expr(lhs);
            recurse_into_expr(rhs);
        }
        JsExpr::Field { object, .. } => recurse_into_expr(object),
        JsExpr::Index { collection, index } => {
            recurse_into_expr(collection);
            recurse_into_expr(index);
        }
        JsExpr::Call { callee, args } => {
            recurse_into_expr(callee);
            for arg in args {
                recurse_into_expr(arg);
            }
        }
        JsExpr::New { callee, args } => {
            recurse_into_expr(callee);
            for arg in args {
                recurse_into_expr(arg);
            }
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            recurse_into_expr(cond);
            recurse_into_expr(then_val);
            recurse_into_expr(else_val);
        }
        JsExpr::Unary { expr, .. }
        | JsExpr::Cast { expr, .. }
        | JsExpr::TypeCheck { expr, .. }
        | JsExpr::Not(expr)
        | JsExpr::PostIncrement(expr)
        | JsExpr::TypeOf(expr)
        | JsExpr::GeneratorResume(expr) => {
            recurse_into_expr(expr);
        }
        JsExpr::Yield(Some(expr)) => recurse_into_expr(expr),
        JsExpr::ArrayInit(elems) | JsExpr::TupleInit(elems) => {
            for e in elems {
                recurse_into_expr(e);
            }
        }
        JsExpr::ObjectInit(fields) => {
            for (_, e) in fields {
                recurse_into_expr(e);
            }
        }
        JsExpr::SystemCall { args, .. }
        | JsExpr::SuperCall(args)
        | JsExpr::GeneratorCreate { args, .. } => {
            for arg in args {
                recurse_into_expr(arg);
            }
        }
        JsExpr::SuperMethodCall { args, .. } => {
            for arg in args {
                recurse_into_expr(arg);
            }
        }
        JsExpr::In { key, object } => {
            recurse_into_expr(key);
            recurse_into_expr(object);
        }
        JsExpr::Delete { object, key } => {
            recurse_into_expr(object);
            recurse_into_expr(key);
        }
        JsExpr::SuperSet { value, .. } => recurse_into_expr(value),
        _ => {}
    }
}

/// Try to recover a switch from a nested if-else-if chain rooted at `stmt`.
///
/// Pattern: `if (EXPR === C1) { body1 } else if (EXPR === C2) { body2 } else { default }`
fn try_recover_nested_if_else(stmt: &mut JsStmt) {
    let mut cases: Vec<(Constant, Vec<JsStmt>)> = Vec::new();
    let mut discriminant: Option<&JsExpr> = None;

    // Walk the if-else-if chain without consuming the statement.
    let mut current = &*stmt;
    let default_body;
    loop {
        if let JsStmt::If {
            cond,
            then_body,
            else_body,
        } = current
        {
            if let Some((disc, constant)) = extract_eq_constant(cond) {
                match &discriminant {
                    None => {
                        if !is_stable_expr(disc) {
                            return;
                        }
                        discriminant = Some(disc);
                    }
                    Some(prev) => {
                        if !exprs_structurally_equal(prev, disc) {
                            return;
                        }
                    }
                }
                cases.push((constant.clone(), then_body.clone()));

                // Continue down the else chain.
                if else_body.len() == 1 {
                    current = &else_body[0];
                    continue;
                } else {
                    // else_body is the default (possibly empty).
                    default_body = else_body.clone();
                    break;
                }
            } else {
                return;
            }
        } else {
            return;
        }
    }

    if cases.len() < 2 {
        return;
    }

    if !all_constants_distinct(&cases) {
        return;
    }

    let disc = discriminant.unwrap().clone();
    *stmt = JsStmt::Switch {
        value: disc,
        cases,
        default_body,
    };
}

/// Try to recover switches from runs of consecutive `if` statements with empty
/// else bodies that all compare the same expression against distinct constants.
fn try_recover_sequential_ifs(body: &mut Vec<JsStmt>) {
    let mut i = 0;
    while i < body.len() {
        // Find the start of a potential run.
        let run_start = i;
        let mut cases: Vec<(Constant, Vec<JsStmt>)> = Vec::new();
        let mut discriminant: Option<&JsExpr> = None;

        while i < body.len() {
            if let JsStmt::If {
                cond,
                then_body,
                else_body,
            } = &body[i]
            {
                if !else_body.is_empty() {
                    break;
                }
                if let Some((disc, constant)) = extract_eq_constant(cond) {
                    match &discriminant {
                        None => {
                            if !is_stable_expr(disc) {
                                break;
                            }
                            discriminant = Some(disc);
                        }
                        Some(prev) => {
                            if !exprs_structurally_equal(prev, disc) {
                                break;
                            }
                        }
                    }
                    cases.push((constant.clone(), then_body.clone()));
                    i += 1;
                    continue;
                }
            }
            break;
        }

        if cases.len() >= 2 {
            let disc = discriminant.unwrap().clone();
            let switch_stmt = JsStmt::Switch {
                value: disc,
                cases,
                default_body: vec![],
            };
            // Replace the run [run_start..i) with the single switch.
            body.splice(run_start..i, std::iter::once(switch_stmt));
            // After splice, the switch is at run_start; advance past it.
            i = run_start + 1;
        } else {
            // No run found starting at run_start; advance.
            i = if i == run_start { run_start + 1 } else { i };
        }
    }
}

/// If `cond` is `EXPR === CONST` (or `CONST === EXPR`), return `(expr, constant)`.
fn extract_eq_constant(cond: &JsExpr) -> Option<(&JsExpr, &Constant)> {
    if let JsExpr::Cmp {
        kind: CmpKind::Eq,
        lhs,
        rhs,
    } = cond
    {
        if let JsExpr::Literal(c) = rhs.as_ref() {
            return Some((lhs.as_ref(), c));
        }
        if let JsExpr::Literal(c) = lhs.as_ref() {
            return Some((rhs.as_ref(), c));
        }
    }
    None
}

/// Conservative check: returns true only for expressions that are clearly
/// free of side effects. This is a syntactic check and cannot rule out all
/// side effects (e.g. property getters).
fn is_stable_expr(expr: &JsExpr) -> bool {
    match expr {
        JsExpr::Var(_) | JsExpr::This | JsExpr::Literal(_) => true,
        JsExpr::Field { object, .. } => is_stable_expr(object),
        JsExpr::Index { collection, index } => {
            is_stable_expr(collection) && is_stable_expr(index)
        }
        _ => false,
    }
}

/// Recursive structural equality for `JsExpr`.
///
/// Conservative: returns false for any variant pair we don't explicitly handle,
/// which prevents incorrect switch recovery rather than risking a wrong match.
fn exprs_structurally_equal(a: &JsExpr, b: &JsExpr) -> bool {
    match (a, b) {
        (JsExpr::Var(x), JsExpr::Var(y)) => x == y,
        (JsExpr::This, JsExpr::This) => true,
        (JsExpr::Literal(x), JsExpr::Literal(y)) => x == y,
        (
            JsExpr::Field {
                object: o1,
                field: f1,
            },
            JsExpr::Field {
                object: o2,
                field: f2,
            },
        ) => f1 == f2 && exprs_structurally_equal(o1, o2),
        (
            JsExpr::Index {
                collection: c1,
                index: i1,
            },
            JsExpr::Index {
                collection: c2,
                index: i2,
            },
        ) => exprs_structurally_equal(c1, c2) && exprs_structurally_equal(i1, i2),
        (
            JsExpr::Cmp {
                kind: k1,
                lhs: l1,
                rhs: r1,
            },
            JsExpr::Cmp {
                kind: k2,
                lhs: l2,
                rhs: r2,
            },
        ) => k1 == k2 && exprs_structurally_equal(l1, l2) && exprs_structurally_equal(r1, r2),
        (
            JsExpr::Binary {
                op: o1,
                lhs: l1,
                rhs: r1,
            },
            JsExpr::Binary {
                op: o2,
                lhs: l2,
                rhs: r2,
            },
        ) => o1 == o2 && exprs_structurally_equal(l1, l2) && exprs_structurally_equal(r1, r2),
        (
            JsExpr::Unary { op: o1, expr: e1 },
            JsExpr::Unary { op: o2, expr: e2 },
        ) => o1 == o2 && exprs_structurally_equal(e1, e2),
        (JsExpr::Not(e1), JsExpr::Not(e2)) => exprs_structurally_equal(e1, e2),
        _ => false,
    }
}

/// Check that all case constants in the list are distinct.
fn all_constants_distinct(cases: &[(Constant, Vec<JsStmt>)]) -> bool {
    for i in 0..cases.len() {
        for j in (i + 1)..cases.len() {
            if cases[i].0 == cases[j].0 {
                return false;
            }
        }
    }
    true
}
