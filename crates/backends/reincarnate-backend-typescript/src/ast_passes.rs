//! Engine-agnostic AST passes for the TypeScript backend.
//!
//! These run after engine-specific rewrites and before printing.

use std::collections::HashMap;

use reincarnate_core::ir::inst::CmpKind;
use reincarnate_core::ir::value::Constant;
use reincarnate_core::ir::{CastKind, Type};

use crate::js_ast::{JsExpr, JsFunction, JsStmt};
use crate::types::ts_type;

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
    // Try the nested if-else-if pattern FIRST (outside-in), before recursing
    // into children. Otherwise inner if-else chains get converted to Switch
    // nodes and the outer chain no longer matches the if-else-if pattern.
    for stmt in body.iter_mut() {
        try_recover_nested_if_else(stmt);
    }

    // Then recurse into all nested bodies.
    for stmt in body.iter_mut() {
        recurse_into_stmt(stmt);
    }

    // Try the sequential-if pattern on runs of consecutive statements.
    try_recover_sequential_ifs(body);

    // Finally, recover discriminants from switch statements whose value is a
    // chained ternary comparison.  This undoes the AVM2 table-jump encoding
    // `switch((x !== A) ? ((x !== B) ? 0 : 1) : 2)` back to `switch(x)`.
    for stmt in body.iter_mut() {
        try_recover_switch_discriminant(stmt);
    }
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
        JsStmt::While { body, .. } | JsStmt::Loop { body } | JsStmt::ForOf { body, .. } => {
            recover_switch_statements(body);
        }
        JsStmt::For {
            init, body, update, ..
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
        | JsExpr::Spread(expr)
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
    let mut cases: Vec<(JsExpr, Vec<JsStmt>)> = Vec::new();
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
                cases.push((JsExpr::Literal(constant.clone()), then_body.clone()));

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

    if !all_case_labels_distinct(&cases) {
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
        let mut cases: Vec<(JsExpr, Vec<JsStmt>)> = Vec::new();
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
                    cases.push((JsExpr::Literal(constant.clone()), then_body.clone()));
                    i += 1;
                    continue;
                }
            }
            break;
        }

        // Only convert to switch when all case values are unique.
        // Duplicate case values change semantics: sequential ifs execute ALL
        // matching branches; a switch only executes the first.
        if cases.len() >= 2 && all_case_labels_distinct(&cases) {
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

// ---------------------------------------------------------------------------
// Switch discriminant recovery from chained ternary comparisons
// ---------------------------------------------------------------------------

/// Attempt to recover a clean `switch(v)` from a switch whose discriminant is
/// an AVM2 table-jump encoding of the form:
///
/// ```text
/// switch ((v !== A) ? ((v !== B) ? 0 : 1) : 2) {
///   case 0: ...
///   case 1: ...
///   case 2: ...
/// }
/// ```
///
/// If the pattern matches, the switch is rewritten in-place to:
///
/// ```text
/// switch (v) { case A: ... case B: ... case 2_fallback: ... }
/// ```
///
/// where each integer discriminant is replaced by the corresponding source
/// constant that was being compared against.
fn try_recover_switch_discriminant(stmt: &mut JsStmt) {
    let JsStmt::Switch {
        value,
        cases,
        default_body,
    } = stmt
    else {
        return;
    };

    let Some((shared_v, mapping)) = extract_ternary_chain(value) else {
        return;
    };

    // mapping: Vec<(case_expr, discriminant_int)>
    // Build a lookup: discriminant_int → case_expr.
    let disc_map: HashMap<i64, JsExpr> = mapping
        .into_iter()
        .map(|(case_expr, disc_int)| (disc_int, case_expr))
        .collect();

    // Require that the mapping is non-trivial (at least one case to recover).
    if disc_map.is_empty() {
        return;
    }

    // Replace the switch discriminant with the shared expression.
    *value = shared_v;

    // Replace each case's integer key with the recovered source expression.
    // The one integer that is NOT in disc_map is the "no-match" fallthrough
    // discriminant (the innermost `then` literal in the ternary chain).
    // If default_body is currently empty, promote that case to the default.
    let mut new_cases: Vec<(JsExpr, Vec<JsStmt>)> = Vec::with_capacity(cases.len());
    let mut taken_cases = std::mem::take(cases);
    for (case_key, case_body) in taken_cases.drain(..) {
        if let JsExpr::Literal(Constant::Int(n)) = &case_key {
            if let Some(recovered) = disc_map.get(n) {
                new_cases.push((recovered.clone(), case_body));
                continue;
            }
            // Not in disc_map → this is the "no-match" branch.
            // Promote to default if default is currently empty.
            if default_body.is_empty() {
                *default_body = case_body;
                continue;
            }
        }
        // Keep as-is.
        new_cases.push((case_key, case_body));
    }
    *cases = new_cases;
}

/// Walk a right-linear ternary chain of the form:
///
/// ```text
/// (v !== A) ? (
///   (v !== B) ? (
///     ... ? N : N-1
///   ) : N-2
/// ) : N-3
/// ```
///
/// Returns `Some((v, pairs))` where `pairs` is a list of
/// `(case_constant, discriminant_int)` in order of the outermost ternary
/// first. The `else_val` of each level is the integer discriminant for that
/// level's condition.
///
/// Returns `None` if the expression does not match the pattern.
fn extract_ternary_chain(expr: &JsExpr) -> Option<(JsExpr, Vec<(JsExpr, i64)>)> {
    match expr {
        // Base case: a bare integer literal — the innermost discriminant.
        // We return an empty pair list; the caller will attach the correct
        // case constant when it processes this level.
        JsExpr::Literal(Constant::Int(_)) => {
            // No pairs at the leaf — the leaf int is the fallthrough discriminant
            // for the innermost ternary's `then` branch.  We signal success with
            // no shared variable (None) so the parent can use its own `v`.
            Some((JsExpr::Literal(Constant::Int(0)), vec![]))
        }

        // Recursive case: (v !== case_expr) ? then_expr : disc_int
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            // Condition must be `v !== case_expr` or `case_expr !== v`.
            let JsExpr::Cmp {
                kind: CmpKind::Ne,
                lhs,
                rhs,
            } = cond.as_ref()
            else {
                return None;
            };

            // The else_val must be an integer literal (this ternary's discriminant).
            let JsExpr::Literal(Constant::Int(disc)) = else_val.as_ref() else {
                return None;
            };

            // Recurse into the then branch to get the shared variable.
            let (inner_v, mut pairs) = extract_ternary_chain(then_val)?;

            let is_leaf = matches!(inner_v, JsExpr::Literal(Constant::Int(_))) && pairs.is_empty();

            // Figure out which side is the shared variable and which is the
            // case expression. At the leaf (innermost level), we don't have a
            // known shared_v yet, so prefer literal on the case side.  At
            // non-leaf levels, match against the known shared_v from recursion.
            let (shared_v_candidate, case_expr) = if is_leaf {
                // Leaf: pick whichever side is a literal as the case constant.
                // If neither is literal, use lhs as shared_v — the parent's
                // structural equality check will validate this choice.
                if matches!(rhs.as_ref(), JsExpr::Literal(_)) {
                    (lhs.as_ref(), rhs.as_ref())
                } else if matches!(lhs.as_ref(), JsExpr::Literal(_)) {
                    (rhs.as_ref(), lhs.as_ref())
                } else {
                    // Neither side is a literal — pick lhs as shared_v.
                    (lhs.as_ref(), rhs.as_ref())
                }
            } else if exprs_structurally_equal(&inner_v, lhs.as_ref()) {
                (lhs.as_ref(), rhs.as_ref())
            } else if exprs_structurally_equal(&inner_v, rhs.as_ref()) {
                (rhs.as_ref(), lhs.as_ref())
            } else {
                return None;
            };

            // Append this level's pair: (case_expr, discriminant_int).
            pairs.push((case_expr.clone(), *disc));

            Some((shared_v_candidate.clone(), pairs))
        }

        _ => None,
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
        JsExpr::Index { collection, index } => is_stable_expr(collection) && is_stable_expr(index),
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
        (JsExpr::Unary { op: o1, expr: e1 }, JsExpr::Unary { op: o2, expr: e2 }) => {
            o1 == o2 && exprs_structurally_equal(e1, e2)
        }
        (JsExpr::Not(e1), JsExpr::Not(e2)) => exprs_structurally_equal(e1, e2),
        _ => false,
    }
}

/// Check that all case labels in the list are distinct (structurally).
fn all_case_labels_distinct(cases: &[(JsExpr, Vec<JsStmt>)]) -> bool {
    for i in 0..cases.len() {
        for j in (i + 1)..cases.len() {
            if exprs_structurally_equal(&cases[i].0, &cases[j].0) {
                return false;
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Redundant NullableCoerce cast elimination
// ---------------------------------------------------------------------------

/// Strip `x as T` casts where the variable `x` is already declared with type
/// `T`. These arise because AVM2 has explicit coerce/astype opcodes that
/// survive the IR `red_cast_elim` pass (which only checks IR-level types, not
/// the emitter's declaration types).
pub fn strip_redundant_casts(func: &mut JsFunction) {
    let mut var_types: HashMap<String, Type> = HashMap::new();
    // Collect param types.
    for (name, ty) in &func.params {
        if *ty != Type::Dynamic {
            var_types.insert(name.clone(), ty.clone());
        }
    }
    // Collect local variable types from declarations.
    collect_var_types(&func.body, &mut var_types);
    // Debug: count what we found
    // Strip redundant casts.
    strip_casts_in_body(&mut func.body, &var_types);
}

fn collect_var_types(body: &[JsStmt], var_types: &mut HashMap<String, Type>) {
    for stmt in body {
        match stmt {
            JsStmt::VarDecl {
                name, ty: Some(ty), ..
            } if *ty != Type::Dynamic => {
                var_types.insert(name.clone(), ty.clone());
            }
            // When ty is None but init is a Cast, the printer uses the cast
            // type as the annotation. Collect that type too.
            JsStmt::VarDecl {
                name,
                ty: None,
                init:
                    Some(JsExpr::Cast {
                        ty: cast_ty,
                        kind: CastKind::NullableCoerce,
                        ..
                    }),
                ..
            } if *cast_ty != Type::Dynamic
                && !matches!(cast_ty, Type::Struct(_) | Type::Enum(_)) =>
            {
                var_types.insert(name.clone(), cast_ty.clone());
            }
            JsStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_var_types(then_body, var_types);
                collect_var_types(else_body, var_types);
            }
            JsStmt::While { body, .. } | JsStmt::Loop { body } | JsStmt::ForOf { body, .. } => {
                collect_var_types(body, var_types);
            }
            JsStmt::For {
                init, body, update, ..
            } => {
                collect_var_types(init, var_types);
                collect_var_types(body, var_types);
                collect_var_types(update, var_types);
            }
            JsStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    collect_var_types(case_body, var_types);
                }
                collect_var_types(default_body, var_types);
            }
            JsStmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    collect_var_types(block_body, var_types);
                }
            }
            _ => {}
        }
    }
}

/// Whether two types map to the same TypeScript type (e.g. Int(32),
/// Float(64), and Union([Int(64), Int(32)]) all map to `number`).
fn same_ts_type(a: &Type, b: &Type) -> bool {
    if a == b {
        return true;
    }
    ts_type(a) == ts_type(b)
}

fn strip_casts_in_body(body: &mut [JsStmt], var_types: &HashMap<String, Type>) {
    for stmt in body.iter_mut() {
        strip_casts_in_stmt(stmt, var_types);
    }
}

fn strip_casts_in_stmt(stmt: &mut JsStmt, var_types: &HashMap<String, Type>) {
    match stmt {
        JsStmt::VarDecl {
            init: Some(expr), ..
        }
        | JsStmt::Expr(expr)
        | JsStmt::Throw(expr) => {
            strip_casts_in_expr(expr, var_types);
        }
        JsStmt::Assign { target, value } => {
            strip_casts_in_expr(target, var_types);
            strip_casts_in_expr(value, var_types);
        }
        JsStmt::CompoundAssign { target, value, .. } => {
            strip_casts_in_expr(target, var_types);
            strip_casts_in_expr(value, var_types);
        }
        JsStmt::Return(Some(expr)) => {
            strip_casts_in_expr(expr, var_types);
        }
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            strip_casts_in_expr(cond, var_types);
            strip_casts_in_body(then_body, var_types);
            strip_casts_in_body(else_body, var_types);
        }
        JsStmt::While { cond, body } => {
            strip_casts_in_expr(cond, var_types);
            strip_casts_in_body(body, var_types);
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            strip_casts_in_body(init, var_types);
            strip_casts_in_expr(cond, var_types);
            strip_casts_in_body(update, var_types);
            strip_casts_in_body(body, var_types);
        }
        JsStmt::Loop { body } | JsStmt::ForOf { body, .. } => {
            strip_casts_in_body(body, var_types);
        }
        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            strip_casts_in_expr(value, var_types);
            for (_, case_body) in cases.iter_mut() {
                strip_casts_in_body(case_body, var_types);
            }
            strip_casts_in_body(default_body, var_types);
        }
        JsStmt::Dispatch { blocks, .. } => {
            for (_, block_body) in blocks.iter_mut() {
                strip_casts_in_body(block_body, var_types);
            }
        }
        _ => {}
    }
}

fn strip_casts_in_expr(expr: &mut JsExpr, var_types: &HashMap<String, Type>) {
    // First, check if this expr is a strippable cast.
    let should_strip = if let JsExpr::Cast {
        expr: inner,
        ty: cast_ty,
        kind,
    } = &*expr
    {
        if *kind == CastKind::NullableCoerce {
            is_cast_redundant(inner, cast_ty, var_types)
        } else {
            false
        }
    } else {
        false
    };

    if should_strip {
        // Unwrap the Cast to its inner expression.
        let inner = match std::mem::replace(expr, JsExpr::This) {
            JsExpr::Cast { expr: inner, .. } => *inner,
            _ => unreachable!(),
        };
        *expr = inner;
    }

    // Recurse into sub-expressions.
    match expr {
        JsExpr::Binary { lhs, rhs, .. }
        | JsExpr::Cmp { lhs, rhs, .. }
        | JsExpr::LogicalOr { lhs, rhs }
        | JsExpr::LogicalAnd { lhs, rhs } => {
            strip_casts_in_expr(lhs, var_types);
            strip_casts_in_expr(rhs, var_types);
        }
        JsExpr::In { key, object } | JsExpr::Delete { object, key } => {
            strip_casts_in_expr(key, var_types);
            strip_casts_in_expr(object, var_types);
        }
        JsExpr::Unary { expr: inner, .. }
        | JsExpr::Not(inner)
        | JsExpr::PostIncrement(inner)
        | JsExpr::Spread(inner)
        | JsExpr::TypeOf(inner)
        | JsExpr::GeneratorResume(inner)
        | JsExpr::Cast { expr: inner, .. }
        | JsExpr::TypeCheck { expr: inner, .. } => {
            strip_casts_in_expr(inner, var_types);
        }
        JsExpr::Field { object, .. } => {
            strip_casts_in_expr(object, var_types);
        }
        JsExpr::Index {
            collection, index, ..
        } => {
            strip_casts_in_expr(collection, var_types);
            strip_casts_in_expr(index, var_types);
        }
        JsExpr::Call { callee, args } | JsExpr::New { callee, args } => {
            strip_casts_in_expr(callee, var_types);
            for arg in args.iter_mut() {
                strip_casts_in_expr(arg, var_types);
            }
        }
        JsExpr::SystemCall { args, .. } | JsExpr::GeneratorCreate { args, .. } => {
            for arg in args.iter_mut() {
                strip_casts_in_expr(arg, var_types);
            }
        }
        JsExpr::SuperCall(args)
        | JsExpr::SuperMethodCall { args, .. }
        | JsExpr::ArrayInit(args)
        | JsExpr::TupleInit(args) => {
            for arg in args.iter_mut() {
                strip_casts_in_expr(arg, var_types);
            }
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            strip_casts_in_expr(cond, var_types);
            strip_casts_in_expr(then_val, var_types);
            strip_casts_in_expr(else_val, var_types);
        }
        JsExpr::ObjectInit(pairs) => {
            for (_, val) in pairs.iter_mut() {
                strip_casts_in_expr(val, var_types);
            }
        }
        JsExpr::SuperSet { value, .. } => {
            strip_casts_in_expr(value, var_types);
        }
        JsExpr::Yield(Some(inner)) => {
            strip_casts_in_expr(inner, var_types);
        }
        JsExpr::ArrowFunction { body, .. } => {
            strip_casts_in_body(body, var_types);
        }
        _ => {}
    }
}

/// Check if a Cast is redundant because the inner expression's type already
/// matches the cast target.
fn is_cast_redundant(inner: &JsExpr, cast_ty: &Type, var_types: &HashMap<String, Type>) -> bool {
    // Only strip TS assertion forms (not runtime calls like asType, Number).
    if matches!(cast_ty, Type::Struct(_) | Type::Enum(_)) {
        return false;
    }
    if let Some(expr_ty) = infer_expr_type(inner, var_types) {
        same_ts_type(&expr_ty, cast_ty)
    } else {
        false
    }
}

/// Infer the TypeScript type of an expression from its structure.
fn infer_expr_type(expr: &JsExpr, var_types: &HashMap<String, Type>) -> Option<Type> {
    match expr {
        JsExpr::Var(name) => var_types.get(name).cloned(),
        JsExpr::Literal(c) => match c {
            Constant::Int(_) | Constant::UInt(_) | Constant::Float(_) => Some(Type::Float(64)),
            Constant::String(_) => Some(Type::String),
            Constant::Bool(_) => Some(Type::Bool),
            Constant::Null => None,
        },
        JsExpr::Binary { .. } | JsExpr::Unary { .. } => Some(Type::Float(64)),
        JsExpr::Cmp { .. } | JsExpr::Not(_) | JsExpr::TypeCheck { .. } => Some(Type::Bool),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Text call coalescing
// ---------------------------------------------------------------------------

/// Merge adjacent `SystemCall("*.Output", "text", [StringLiteral])` statements
/// into a single call with the concatenated string. Reduces output size for
/// text-heavy engines like Harlowe where the parser fragments text across
/// multiple nodes.
pub fn coalesce_text_calls(body: &mut Vec<JsStmt>) {
    // Recurse into nested bodies first
    for stmt in body.iter_mut() {
        coalesce_text_in_stmt(stmt);
    }

    // Merge adjacent text calls at this level
    let mut i = 0;
    while i < body.len() {
        if let Some((system, first_text)) = extract_text_call(&body[i]) {
            let mut merged = first_text;
            let mut j = i + 1;
            while j < body.len() {
                if let Some((sys2, next_text)) = extract_text_call(&body[j]) {
                    if sys2 == system {
                        merged.push_str(&next_text);
                        j += 1;
                        continue;
                    }
                }
                break;
            }
            if j > i + 1 {
                let replacement = JsStmt::Expr(JsExpr::SystemCall {
                    system,
                    method: "text".to_string(),
                    args: vec![JsExpr::Literal(Constant::String(merged))],
                });
                body.splice(i..j, std::iter::once(replacement));
            }
        }
        i += 1;
    }
}

fn coalesce_text_in_stmt(stmt: &mut JsStmt) {
    match stmt {
        JsStmt::If {
            then_body,
            else_body,
            ..
        } => {
            coalesce_text_calls(then_body);
            coalesce_text_calls(else_body);
        }
        JsStmt::While { body, .. } | JsStmt::Loop { body } | JsStmt::ForOf { body, .. } => {
            coalesce_text_calls(body);
        }
        JsStmt::For {
            init, body, update, ..
        } => {
            coalesce_text_calls(init);
            coalesce_text_calls(body);
            coalesce_text_calls(update);
        }
        JsStmt::Switch {
            cases,
            default_body,
            ..
        } => {
            for (_, case_body) in cases {
                coalesce_text_calls(case_body);
            }
            coalesce_text_calls(default_body);
        }
        JsStmt::Dispatch { blocks, .. } => {
            for (_, block_body) in blocks {
                coalesce_text_calls(block_body);
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Array string coalescing
// ---------------------------------------------------------------------------

/// Merge adjacent string literals inside `ArrayInit` nodes.
/// `["Hello ", "world", br(), "foo", "bar"]` → `["Hello world", br(), "foobar"]`.
///
/// Runs after engine-specific rewrites (which convert `content_array` →
/// `ArrayInit` and `text_node` → identity string) so that the merged strings
/// appear in the final output.
pub fn coalesce_array_strings(body: &mut [JsStmt]) {
    for stmt in body.iter_mut() {
        coalesce_arrays_in_stmt(stmt);
    }
}

fn coalesce_arrays_in_stmt(stmt: &mut JsStmt) {
    match stmt {
        JsStmt::VarDecl { init: Some(e), .. }
        | JsStmt::Assign { value: e, .. }
        | JsStmt::Expr(e)
        | JsStmt::Throw(e) => coalesce_arrays_in_expr(e),
        JsStmt::CompoundAssign { value, .. } => coalesce_arrays_in_expr(value),
        JsStmt::Return(Some(e)) => coalesce_arrays_in_expr(e),
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            coalesce_arrays_in_expr(cond);
            coalesce_array_strings(then_body);
            coalesce_array_strings(else_body);
        }
        JsStmt::While { cond, body } => {
            coalesce_arrays_in_expr(cond);
            coalesce_array_strings(body);
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            coalesce_array_strings(init);
            coalesce_arrays_in_expr(cond);
            coalesce_array_strings(update);
            coalesce_array_strings(body);
        }
        JsStmt::Loop { body } | JsStmt::ForOf { body, .. } => {
            coalesce_array_strings(body);
        }
        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            coalesce_arrays_in_expr(value);
            for (_, case_body) in cases {
                coalesce_array_strings(case_body);
            }
            coalesce_array_strings(default_body);
        }
        JsStmt::Dispatch { blocks, .. } => {
            for (_, block_body) in blocks {
                coalesce_array_strings(block_body);
            }
        }
        _ => {}
    }
}

fn coalesce_arrays_in_expr(expr: &mut JsExpr) {
    match expr {
        JsExpr::ArrayInit(items) => {
            // Recurse into children first.
            for item in items.iter_mut() {
                coalesce_arrays_in_expr(item);
            }
            // Then merge adjacent string literals.
            merge_adjacent_strings(items);
        }
        JsExpr::Binary { lhs, rhs, .. }
        | JsExpr::Cmp { lhs, rhs, .. }
        | JsExpr::LogicalOr { lhs, rhs }
        | JsExpr::LogicalAnd { lhs, rhs } => {
            coalesce_arrays_in_expr(lhs);
            coalesce_arrays_in_expr(rhs);
        }
        JsExpr::Unary { expr: inner, .. }
        | JsExpr::Not(inner)
        | JsExpr::PostIncrement(inner)
        | JsExpr::Spread(inner)
        | JsExpr::TypeOf(inner)
        | JsExpr::Cast { expr: inner, .. }
        | JsExpr::TypeCheck { expr: inner, .. }
        | JsExpr::GeneratorResume(inner) => {
            coalesce_arrays_in_expr(inner);
        }
        JsExpr::Field { object, .. } => coalesce_arrays_in_expr(object),
        JsExpr::Index { collection, index } => {
            coalesce_arrays_in_expr(collection);
            coalesce_arrays_in_expr(index);
        }
        JsExpr::Call { callee, args } | JsExpr::New { callee, args } => {
            coalesce_arrays_in_expr(callee);
            for arg in args.iter_mut() {
                coalesce_arrays_in_expr(arg);
            }
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            coalesce_arrays_in_expr(cond);
            coalesce_arrays_in_expr(then_val);
            coalesce_arrays_in_expr(else_val);
        }
        JsExpr::TupleInit(items) => {
            for item in items.iter_mut() {
                coalesce_arrays_in_expr(item);
            }
        }
        JsExpr::ObjectInit(fields) => {
            for (_, val) in fields.iter_mut() {
                coalesce_arrays_in_expr(val);
            }
        }
        JsExpr::SystemCall { args, .. }
        | JsExpr::SuperCall(args)
        | JsExpr::GeneratorCreate { args, .. }
        | JsExpr::SuperMethodCall { args, .. } => {
            for arg in args.iter_mut() {
                coalesce_arrays_in_expr(arg);
            }
        }
        JsExpr::In { key, object } | JsExpr::Delete { object, key } => {
            coalesce_arrays_in_expr(key);
            coalesce_arrays_in_expr(object);
        }
        JsExpr::SuperSet { value, .. } => coalesce_arrays_in_expr(value),
        JsExpr::Yield(Some(inner)) => coalesce_arrays_in_expr(inner),
        JsExpr::ArrowFunction { body, .. } => coalesce_array_strings(body),
        _ => {}
    }
}

/// Merge runs of adjacent string literals in a Vec of expressions in-place.
fn merge_adjacent_strings(items: &mut Vec<JsExpr>) {
    let mut i = 0;
    while i < items.len() {
        if let JsExpr::Literal(Constant::String(_)) = &items[i] {
            let mut j = i + 1;
            while j < items.len() {
                if matches!(&items[j], JsExpr::Literal(Constant::String(_))) {
                    j += 1;
                } else {
                    break;
                }
            }
            if j > i + 1 {
                // Merge items[i..j] into a single string.
                let mut merged = String::new();
                for item in items[i..j].iter() {
                    if let JsExpr::Literal(Constant::String(s)) = item {
                        merged.push_str(s);
                    }
                }
                items.splice(
                    i..j,
                    std::iter::once(JsExpr::Literal(Constant::String(merged))),
                );
            }
        }
        i += 1;
    }
}

/// Extract the system namespace and string literal from a text call statement.
fn extract_text_call(stmt: &JsStmt) -> Option<(String, String)> {
    if let JsStmt::Expr(JsExpr::SystemCall {
        system,
        method,
        args,
    }) = stmt
    {
        if method == "text" && system.ends_with("_Output") && args.len() == 1 {
            if let JsExpr::Literal(Constant::String(s)) = &args[0] {
                return Some((system.clone(), s.clone()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js_ast::{JsExpr, JsStmt};
    use reincarnate_core::ir::inst::CmpKind;
    use reincarnate_core::ir::value::Constant;

    fn var(name: &str) -> JsExpr {
        JsExpr::Var(name.to_string())
    }

    fn int_lit(n: i64) -> JsExpr {
        JsExpr::Literal(Constant::Int(n))
    }

    fn str_lit(s: &str) -> JsExpr {
        JsExpr::Literal(Constant::String(s.to_string()))
    }

    fn eq(lhs: JsExpr, rhs: JsExpr) -> JsExpr {
        JsExpr::Cmp {
            kind: CmpKind::Eq,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    fn ne(lhs: JsExpr, rhs: JsExpr) -> JsExpr {
        JsExpr::Cmp {
            kind: CmpKind::Ne,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        }
    }

    fn ternary(cond: JsExpr, then_val: JsExpr, else_val: JsExpr) -> JsExpr {
        JsExpr::Ternary {
            cond: Box::new(cond),
            then_val: Box::new(then_val),
            else_val: Box::new(else_val),
        }
    }

    /// Build the AVM2 ternary-chain discriminant for:
    ///   `switch(x) { case "A": ...; case "B": ... }` compiled as
    ///   `switch((x !== "A") ? ((x !== "B") ? 0 : 1) : 2) { case 0: default; case 1: B body; case 2: A body }`
    fn avm2_discriminant_2cases(v: JsExpr, a: JsExpr, b: JsExpr) -> JsExpr {
        // (v !== a) ? ((v !== b) ? 0 : 1) : 2
        ternary(
            ne(v.clone(), a),
            ternary(ne(v, b), int_lit(0), int_lit(1)),
            int_lit(2),
        )
    }

    /// Test that the AVM2 chained-ternary switch discriminant pattern is recovered
    /// back to a clean `switch(v)` with source constants as case labels.
    #[test]
    fn switch_discriminant_recovery_from_ternary_chain() {
        // Simulate: switch(x) { case "A": bodyA; case "B": bodyB; default: bodyDefault }
        // compiled by AVM2 into:
        //   switch ((x !== "A") ? ((x !== "B") ? 0 : 1) : 2) {
        //     case 0: bodyDefault;
        //     case 1: bodyB;
        //     case 2: bodyA;
        //   }
        let discriminant = avm2_discriminant_2cases(var("x"), str_lit("A"), str_lit("B"));
        let body_default = vec![JsStmt::Expr(var("bodyDefault"))];
        let body_b = vec![JsStmt::Expr(var("bodyB"))];
        let body_a = vec![JsStmt::Expr(var("bodyA"))];

        let mut body = vec![JsStmt::Switch {
            value: discriminant,
            cases: vec![
                (JsExpr::Literal(Constant::Int(0)), body_default.clone()),
                (JsExpr::Literal(Constant::Int(1)), body_b.clone()),
                (JsExpr::Literal(Constant::Int(2)), body_a.clone()),
            ],
            default_body: vec![],
        }];

        recover_switch_statements(&mut body);

        assert_eq!(body.len(), 1);
        match &body[0] {
            JsStmt::Switch {
                value,
                cases,
                default_body,
            } => {
                // Discriminant must now be `x`, not the ternary chain.
                assert!(
                    matches!(value, JsExpr::Var(name) if name == "x"),
                    "expected discriminant to be `x`, got: {value:?}"
                );
                // Case 0 should have been promoted to default_body.
                assert!(
                    !default_body.is_empty(),
                    "expected default_body to be populated from case 0"
                );
                assert_eq!(
                    cases.len(),
                    2,
                    "expected 2 recovered cases (A and B), got: {cases:?}"
                );
                // The two cases should have string constants, not integers.
                let case_keys: Vec<_> = cases.iter().map(|(k, _)| k).collect();
                assert!(
                    case_keys
                        .iter()
                        .any(|k| matches!(k, JsExpr::Literal(Constant::String(s)) if s == "B")),
                    "expected case \"B\", got: {case_keys:?}"
                );
                assert!(
                    case_keys
                        .iter()
                        .any(|k| matches!(k, JsExpr::Literal(Constant::String(s)) if s == "A")),
                    "expected case \"A\", got: {case_keys:?}"
                );
            }
            other => panic!("expected Switch, got: {other:?}"),
        }
    }

    /// Test the 3-comparison case with reversed comparison order (const on left, var on right)
    /// and with a pre-existing default body.  Mirrors the Parser.ts pattern:
    ///   `switch(("[" !== _loc8) ? (("]" !== _loc8) ? (("|" !== _loc8) ? 3 : 2) : 1) : 0)`
    #[test]
    fn switch_discriminant_recovery_3cases_const_left() {
        // Innermost: ("|" !== x) ? 3 : 2
        // Middle:    ("]" !== x) ? innermost : 1
        // Outer:     ("[" !== x) ? middle : 0
        let discriminant = ternary(
            ne(str_lit("["), var("x")), // const on LEFT
            ternary(
                ne(str_lit("]"), var("x")),
                ternary(ne(str_lit("|"), var("x")), int_lit(3), int_lit(2)),
                int_lit(1),
            ),
            int_lit(0),
        );

        let body0 = vec![JsStmt::Expr(var("body_bracket_open"))];
        let body1 = vec![JsStmt::Expr(var("body_bracket_close"))];
        let body2 = vec![JsStmt::Expr(var("body_pipe"))];
        let body3 = vec![JsStmt::Expr(var("body_other"))];
        let default_body_pre = vec![JsStmt::Expr(var("default_pre"))];

        let mut body = vec![JsStmt::Switch {
            value: discriminant,
            cases: vec![
                (JsExpr::Literal(Constant::Int(0)), body0.clone()),
                (JsExpr::Literal(Constant::Int(1)), body1.clone()),
                (JsExpr::Literal(Constant::Int(2)), body2.clone()),
                (JsExpr::Literal(Constant::Int(3)), body3.clone()),
            ],
            // Pre-existing default_body (non-empty) — should NOT block case recovery.
            default_body: default_body_pre.clone(),
        }];

        recover_switch_statements(&mut body);

        assert_eq!(body.len(), 1);
        match &body[0] {
            JsStmt::Switch {
                value,
                cases,
                default_body,
            } => {
                // Discriminant must now be `x`.
                assert!(
                    matches!(value, JsExpr::Var(name) if name == "x"),
                    "expected discriminant `x`, got: {value:?}"
                );
                // default_body should remain unchanged (was non-empty before).
                assert!(!default_body.is_empty(), "default_body should be unchanged");

                // The 4 cases should now have string constant keys.
                assert_eq!(cases.len(), 4, "expected 4 cases, got: {cases:?}");
                let case_keys: Vec<_> = cases.iter().map(|(k, _)| k).collect();
                assert!(
                    case_keys
                        .iter()
                        .any(|k| matches!(k, JsExpr::Literal(Constant::String(s)) if s == "[")),
                    "expected case \"[\", got: {case_keys:?}"
                );
                assert!(
                    case_keys
                        .iter()
                        .any(|k| matches!(k, JsExpr::Literal(Constant::String(s)) if s == "]")),
                    "expected case \"]\", got: {case_keys:?}"
                );
                assert!(
                    case_keys
                        .iter()
                        .any(|k| matches!(k, JsExpr::Literal(Constant::String(s)) if s == "|")),
                    "expected case \"|\", got: {case_keys:?}"
                );
                // case 3 (the no-match) stays as Int(3) since default_body was non-empty.
                assert!(
                    case_keys
                        .iter()
                        .any(|k| matches!(k, JsExpr::Literal(Constant::Int(3)))),
                    "expected case Int(3) to remain, got: {case_keys:?}"
                );
            }
            other => panic!("expected Switch, got: {other:?}"),
        }
    }

    /// Direct test of extract_ternary_chain with the Parser.ts pattern.
    #[test]
    fn extract_ternary_chain_direct() {
        // ("[" !== x) ? (("]" !== x) ? (("|" !== x) ? 3 : 2) : 1) : 0
        let chain = ternary(
            ne(str_lit("["), var("x")),
            ternary(
                ne(str_lit("]"), var("x")),
                ternary(ne(str_lit("|"), var("x")), int_lit(3), int_lit(2)),
                int_lit(1),
            ),
            int_lit(0),
        );

        let result = extract_ternary_chain(&chain);
        assert!(result.is_some(), "expected Some, got None");
        let (v, pairs) = result.unwrap();
        assert!(
            matches!(&v, JsExpr::Var(name) if name == "x"),
            "expected v=Var(x), got: {v:?}"
        );
        assert_eq!(pairs.len(), 3, "expected 3 pairs, got: {pairs:?}");
        // Pairs are built innermost-to-outermost: ("|", 2), ("]", 1), ("[", 0)
        assert!(
            matches!(&pairs[0].0, JsExpr::Literal(Constant::String(s)) if s == "|"),
            "got: {:?}",
            pairs[0].0
        );
        assert_eq!(pairs[0].1, 2);
        assert!(
            matches!(&pairs[1].0, JsExpr::Literal(Constant::String(s)) if s == "]"),
            "got: {:?}",
            pairs[1].0
        );
        assert_eq!(pairs[1].1, 1);
        assert!(
            matches!(&pairs[2].0, JsExpr::Literal(Constant::String(s)) if s == "["),
            "got: {:?}",
            pairs[2].0
        );
        assert_eq!(pairs[2].1, 0);
    }

    /// Test that non-literal case expressions (e.g. `Keyboard.UP`) are recovered
    /// from chained ternary patterns. This is the key new capability: case labels
    /// can be arbitrary JsExpr, not just Constant.
    #[test]
    fn switch_discriminant_recovery_non_literal_cases() {
        // Pattern: switch((x !== Keyboard.UP) ? ((x !== Keyboard.DOWN) ? 0 : 1) : 2)
        // Should recover: switch(x) { case Keyboard.UP: ...; case Keyboard.DOWN: ...; }
        let keyboard_up = JsExpr::Field {
            object: Box::new(var("Keyboard")),
            field: "UP".to_string(),
        };
        let keyboard_down = JsExpr::Field {
            object: Box::new(var("Keyboard")),
            field: "DOWN".to_string(),
        };

        let discriminant = ternary(
            ne(var("x"), keyboard_up.clone()),
            ternary(ne(var("x"), keyboard_down.clone()), int_lit(0), int_lit(1)),
            int_lit(2),
        );

        let body0 = vec![JsStmt::Expr(var("body_default"))];
        let body1 = vec![JsStmt::Expr(var("body_down"))];
        let body2 = vec![JsStmt::Expr(var("body_up"))];

        let mut body = vec![JsStmt::Switch {
            value: discriminant,
            cases: vec![
                (JsExpr::Literal(Constant::Int(0)), body0),
                (JsExpr::Literal(Constant::Int(1)), body1),
                (JsExpr::Literal(Constant::Int(2)), body2),
            ],
            default_body: vec![],
        }];

        recover_switch_statements(&mut body);

        assert_eq!(body.len(), 1);
        match &body[0] {
            JsStmt::Switch {
                value,
                cases,
                default_body,
            } => {
                // Discriminant must now be `x`.
                assert!(
                    matches!(value, JsExpr::Var(name) if name == "x"),
                    "expected discriminant `x`, got: {value:?}"
                );
                // Case 0 (no-match) should be promoted to default.
                assert!(
                    !default_body.is_empty(),
                    "expected default_body from case 0"
                );
                assert_eq!(cases.len(), 2, "expected 2 cases, got: {cases:?}");
                // One case should be Keyboard.UP, the other Keyboard.DOWN.
                let has_up = cases.iter().any(|(k, _)| {
                    matches!(k, JsExpr::Field { object, field }
                        if matches!(object.as_ref(), JsExpr::Var(n) if n == "Keyboard")
                        && field == "UP")
                });
                let has_down = cases.iter().any(|(k, _)| {
                    matches!(k, JsExpr::Field { object, field }
                        if matches!(object.as_ref(), JsExpr::Var(n) if n == "Keyboard")
                        && field == "DOWN")
                });
                assert!(has_up, "expected case Keyboard.UP");
                assert!(has_down, "expected case Keyboard.DOWN");
            }
            other => panic!("expected Switch, got: {other:?}"),
        }
    }

    #[test]
    fn switch_recovery_nested_if_else_chain() {
        // if (x === 3) { A } else if (x === 2) { B } else if (x === 1) { C }
        // Should produce a single 3-case switch, not if { } else { switch { } }.
        let mut body = vec![JsStmt::If {
            cond: eq(var("x"), int_lit(3)),
            then_body: vec![JsStmt::Expr(var("A"))],
            else_body: vec![JsStmt::If {
                cond: eq(var("x"), int_lit(2)),
                then_body: vec![JsStmt::Expr(var("B"))],
                else_body: vec![JsStmt::If {
                    cond: eq(var("x"), int_lit(1)),
                    then_body: vec![JsStmt::Expr(var("C"))],
                    else_body: vec![],
                }],
            }],
        }];

        recover_switch_statements(&mut body);

        assert_eq!(body.len(), 1, "Expected single statement, got: {body:?}");
        match &body[0] {
            JsStmt::Switch { cases, .. } => {
                assert_eq!(
                    cases.len(),
                    3,
                    "Expected 3-case switch, got {}-case: {cases:?}",
                    cases.len()
                );
            }
            other => panic!("Expected Switch, got: {other:?}"),
        }
    }

    #[test]
    fn test_coalesce_adjacent_text_calls() {
        fn text_call(s: &str) -> JsStmt {
            JsStmt::Expr(JsExpr::SystemCall {
                system: "Harlowe_Output".to_string(),
                method: "text".to_string(),
                args: vec![JsExpr::Literal(Constant::String(s.to_string()))],
            })
        }

        let mut body = vec![
            text_call("Hello "),
            text_call("world"),
            // Non-text barrier
            JsStmt::Expr(JsExpr::SystemCall {
                system: "Harlowe_Output".to_string(),
                method: "void_element".to_string(),
                args: vec![JsExpr::Literal(Constant::String("br".to_string()))],
            }),
            text_call("foo"),
            text_call("bar"),
            text_call("baz"),
        ];

        coalesce_text_calls(&mut body);

        assert_eq!(body.len(), 3, "should merge runs but not across barriers");
        // First: merged "Hello world"
        if let JsStmt::Expr(JsExpr::SystemCall { args, .. }) = &body[0] {
            assert!(matches!(&args[0], JsExpr::Literal(Constant::String(s)) if s == "Hello world"));
        } else {
            panic!("expected text call");
        }
        // Second: barrier (void_element)
        assert!(
            matches!(&body[1], JsStmt::Expr(JsExpr::SystemCall { method, .. }) if method == "void_element")
        );
        // Third: merged "foobarbaz"
        if let JsStmt::Expr(JsExpr::SystemCall { args, .. }) = &body[2] {
            assert!(matches!(&args[0], JsExpr::Literal(Constant::String(s)) if s == "foobarbaz"));
        } else {
            panic!("expected text call");
        }
    }

    #[test]
    fn test_coalesce_array_strings() {
        // ["Hello ", "world", br(), "foo", "bar"] → ["Hello world", br(), "foobar"]
        let mut body = vec![JsStmt::Return(Some(JsExpr::ArrayInit(vec![
            JsExpr::Literal(Constant::String("Hello ".into())),
            JsExpr::Literal(Constant::String("world".into())),
            JsExpr::Call {
                callee: Box::new(JsExpr::Var("br".into())),
                args: vec![],
            },
            JsExpr::Literal(Constant::String("foo".into())),
            JsExpr::Literal(Constant::String("bar".into())),
        ])))];

        coalesce_array_strings(&mut body);

        if let JsStmt::Return(Some(JsExpr::ArrayInit(items))) = &body[0] {
            assert_eq!(
                items.len(),
                3,
                "expected 3 items after coalescing, got: {items:?}"
            );
            assert!(
                matches!(&items[0], JsExpr::Literal(Constant::String(s)) if s == "Hello world")
            );
            assert!(matches!(&items[1], JsExpr::Call { .. }));
            assert!(matches!(&items[2], JsExpr::Literal(Constant::String(s)) if s == "foobar"));
        } else {
            panic!("expected Return(ArrayInit)");
        }
    }

    #[test]
    fn test_coalesce_array_strings_nested() {
        // color("red", ["a", "b"]) → color("red", ["ab"])
        let mut body = vec![JsStmt::Return(Some(JsExpr::Call {
            callee: Box::new(JsExpr::Var("color".into())),
            args: vec![
                JsExpr::Literal(Constant::String("red".into())),
                JsExpr::ArrayInit(vec![
                    JsExpr::Literal(Constant::String("a".into())),
                    JsExpr::Literal(Constant::String("b".into())),
                ]),
            ],
        }))];

        coalesce_array_strings(&mut body);

        if let JsStmt::Return(Some(JsExpr::Call { args, .. })) = &body[0] {
            if let JsExpr::ArrayInit(items) = &args[1] {
                assert_eq!(items.len(), 1);
                assert!(matches!(&items[0], JsExpr::Literal(Constant::String(s)) if s == "ab"));
            } else {
                panic!("expected ArrayInit");
            }
        } else {
            panic!("expected Return(Call)");
        }
    }
}
