//! Control-flow rewrite passes: ternary, min/max, compound assign, for-each,
//! for promotion, post-increment, logical operator simplification, and
//! Harlowe output node lowering.

use std::collections::HashSet;

use super::super::ast::{BinOp, Expr, Stmt};
use super::super::inst::CmpKind;
use super::super::ty::Type;
use super::super::value::Constant;
use super::{count_var_reads_in_stmt, recurse_into_stmt, strip_as_type, substitute_var_in_stmt};

// ---------------------------------------------------------------------------
// Lower synthetic output SystemCalls to native AST nodes
// ---------------------------------------------------------------------------

/// Rewrite `SystemCall(system, method, args)` to `MethodCall(receiver_var, method, args)`.
///
/// `system` and `receiver_var` come from `LoweringConfig::output_node_system`.
/// Doing this early lets optimization passes see them as regular method calls.
pub fn lower_output_nodes(body: &mut [Stmt], system: &str, receiver_var: &str) {
    for stmt in body.iter_mut() {
        lower_output_nodes_in_stmt(stmt, system, receiver_var);
    }
}

fn lower_output_nodes_in_stmt(stmt: &mut Stmt, system: &str, receiver_var: &str) {
    match stmt {
        Stmt::VarDecl { init: Some(e), .. } => {
            lower_output_nodes_in_expr(e, system, receiver_var);
        }
        Stmt::Assign { target, value } => {
            lower_output_nodes_in_expr(target, system, receiver_var);
            lower_output_nodes_in_expr(value, system, receiver_var);
        }
        Stmt::CompoundAssign { target, value, .. } => {
            lower_output_nodes_in_expr(target, system, receiver_var);
            lower_output_nodes_in_expr(value, system, receiver_var);
        }
        Stmt::Expr(e) => lower_output_nodes_in_expr(e, system, receiver_var),
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            lower_output_nodes_in_expr(cond, system, receiver_var);
            lower_output_nodes(then_body, system, receiver_var);
            lower_output_nodes(else_body, system, receiver_var);
        }
        Stmt::While { cond, body } => {
            lower_output_nodes_in_expr(cond, system, receiver_var);
            lower_output_nodes(body, system, receiver_var);
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            lower_output_nodes(init, system, receiver_var);
            lower_output_nodes_in_expr(cond, system, receiver_var);
            lower_output_nodes(update, system, receiver_var);
            lower_output_nodes(body, system, receiver_var);
        }
        Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
            lower_output_nodes(body, system, receiver_var);
        }
        Stmt::Return(Some(e)) => lower_output_nodes_in_expr(e, system, receiver_var),
        Stmt::Switch {
            value,
            cases,
            default_body,
        } => {
            lower_output_nodes_in_expr(value, system, receiver_var);
            for (_, case_body) in cases {
                lower_output_nodes(case_body, system, receiver_var);
            }
            lower_output_nodes(default_body, system, receiver_var);
        }
        Stmt::Dispatch { blocks, .. } => {
            for (_, block_body) in blocks {
                lower_output_nodes(block_body, system, receiver_var);
            }
        }
        _ => {}
    }
}

fn lower_output_nodes_in_expr(expr: &mut Expr, system: &str, receiver_var: &str) {
    // Post-order: recurse first, then try to rewrite this node.
    match expr {
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            lower_output_nodes_in_expr(lhs, system, receiver_var);
            lower_output_nodes_in_expr(rhs, system, receiver_var);
        }
        Expr::LogicalOr { lhs, rhs } | Expr::LogicalAnd { lhs, rhs } => {
            lower_output_nodes_in_expr(lhs, system, receiver_var);
            lower_output_nodes_in_expr(rhs, system, receiver_var);
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::Spread(inner) => {
            lower_output_nodes_in_expr(inner, system, receiver_var);
        }
        Expr::Field { object, .. } => lower_output_nodes_in_expr(object, system, receiver_var),
        Expr::Index { collection, index } => {
            lower_output_nodes_in_expr(collection, system, receiver_var);
            lower_output_nodes_in_expr(index, system, receiver_var);
        }
        Expr::Call { args, .. }
        | Expr::CoroutineCreate { args, .. }
        | Expr::SystemCall { args, .. } => {
            for a in args {
                lower_output_nodes_in_expr(a, system, receiver_var);
            }
        }
        Expr::CallIndirect { callee, args } => {
            lower_output_nodes_in_expr(callee, system, receiver_var);
            for a in args {
                lower_output_nodes_in_expr(a, system, receiver_var);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            lower_output_nodes_in_expr(receiver, system, receiver_var);
            for a in args {
                lower_output_nodes_in_expr(a, system, receiver_var);
            }
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            lower_output_nodes_in_expr(cond, system, receiver_var);
            lower_output_nodes_in_expr(then_val, system, receiver_var);
            lower_output_nodes_in_expr(else_val, system, receiver_var);
        }
        Expr::ArrayInit(elems) | Expr::TupleInit(elems) => {
            for e in elems {
                lower_output_nodes_in_expr(e, system, receiver_var);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, v) in fields {
                lower_output_nodes_in_expr(v, system, receiver_var);
            }
        }
        Expr::PostIncrement(inner) | Expr::CoroutineResume(inner) => {
            lower_output_nodes_in_expr(inner, system, receiver_var);
        }
        Expr::Yield(Some(e)) => {
            lower_output_nodes_in_expr(e, system, receiver_var);
        }
        _ => {} // Literal, Var, GlobalRef -- no children
    }

    // Rewrite SystemCall(system, method, args) -> MethodCall(receiver_var, method, args).
    if let Expr::SystemCall {
        system: s,
        method,
        args,
    } = expr
    {
        if s == system {
            *expr = Expr::MethodCall {
                receiver: Box::new(Expr::Var(receiver_var.into())),
                method: std::mem::take(method),
                args: std::mem::take(args),
            };
        }
    }
}

// ---------------------------------------------------------------------------
// Ternary rewrite
// ---------------------------------------------------------------------------

/// Rewrite single-assign if/else to ternary expressions.
///
/// Matches:
/// ```text
/// if (cond) { x = a; } else { x = b; }
/// ```
/// and rewrites to:
/// ```text
/// x = cond ? a : b;
/// ```
///
/// Recurses into all nested statement bodies.
pub fn rewrite_ternary(body: &mut [Stmt]) {
    for stmt in body.iter_mut() {
        // First recurse into nested bodies.
        recurse_into_stmt(stmt, rewrite_ternary);

        // Then try to rewrite this statement.
        let replacement = match stmt {
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => match_ternary(cond, then_body, else_body),
            _ => None,
        };

        if let Some(new_stmt) = replacement {
            *stmt = new_stmt;
        }
    }
}

/// Check whether an if/else matches the single-assign ternary pattern.
fn match_ternary(cond: &Expr, then_body: &[Stmt], else_body: &[Stmt]) -> Option<Stmt> {
    if then_body.len() != 1 || else_body.len() != 1 {
        return None;
    }

    let (then_target, then_value) = match &then_body[0] {
        Stmt::Assign { target, value } => (target, value),
        _ => return None,
    };
    let (else_target, else_value) = match &else_body[0] {
        Stmt::Assign { target, value } => (target, value),
        _ => return None,
    };

    if then_target != else_target {
        return None;
    }

    // Skip identity-branch ternaries where one side is just the target variable
    // (e.g. `x = cond ? x : y` -> better as `if (!cond) { x = y; }`).
    if then_value == then_target || else_value == then_target {
        return None;
    }

    Some(Stmt::Assign {
        target: then_target.clone(),
        value: Expr::Ternary {
            cond: Box::new(cond.clone()),
            then_val: Box::new(then_value.clone()),
            else_val: Box::new(else_value.clone()),
        },
    })
}

// ---------------------------------------------------------------------------
// Math.max / Math.min rewrite
// ---------------------------------------------------------------------------

/// Rewrite comparison+ternary patterns to `Math.max` / `Math.min`.
///
/// Must run **after** `rewrite_ternary`. Recurses into all nested statement
/// bodies.
pub fn rewrite_minmax(body: &mut [Stmt]) {
    for stmt in body.iter_mut() {
        recurse_into_stmt(stmt, rewrite_minmax);

        let replacement = match stmt {
            Stmt::Assign { target, value } => match_minmax(target, value),
            _ => None,
        };

        if let Some(new_stmt) = replacement {
            *stmt = new_stmt;
        }
    }
}

/// Check whether an assign of a ternary matches a Math.max/min pattern.
fn match_minmax(target: &Expr, value: &Expr) -> Option<Stmt> {
    let (cond, then_val, else_val) = match value {
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => (cond.as_ref(), then_val.as_ref(), else_val.as_ref()),
        _ => return None,
    };

    let (kind, cmp_lhs, cmp_rhs) = match cond {
        Expr::Cmp { kind, lhs, rhs } => (*kind, lhs.as_ref(), rhs.as_ref()),
        _ => return None,
    };

    let func_name = match kind {
        CmpKind::Ge | CmpKind::Gt => {
            if then_val == cmp_lhs && else_val == cmp_rhs {
                "Math.max"
            } else if then_val == cmp_rhs && else_val == cmp_lhs {
                "Math.min"
            } else {
                return None;
            }
        }
        CmpKind::Le | CmpKind::Lt => {
            if then_val == cmp_lhs && else_val == cmp_rhs {
                "Math.min"
            } else if then_val == cmp_rhs && else_val == cmp_lhs {
                "Math.max"
            } else {
                return None;
            }
        }
        _ => return None,
    };

    Some(Stmt::Assign {
        target: target.clone(),
        value: Expr::Call {
            func: func_name.to_string(),
            args: vec![then_val.clone(), else_val.clone()],
        },
    })
}

// ---------------------------------------------------------------------------
// Simplify ternary to logical operators
// ---------------------------------------------------------------------------

/// Simplify `cond ? then_val : cond` -> `cond && then_val`,
/// and `cond ? cond : else_val` -> `cond || else_val`.
///
/// Recurses bottom-up into all sub-expressions and then into nested
/// statement bodies.
pub fn simplify_ternary_to_logical(body: &mut [Stmt]) {
    for stmt in body.iter_mut() {
        simplify_ternary_in_stmt(stmt);
    }
}

fn simplify_ternary_in_stmt(stmt: &mut Stmt) {
    match stmt {
        Stmt::VarDecl { init, .. } => {
            if let Some(e) = init {
                simplify_ternary_in_expr(e);
            }
        }
        Stmt::Assign { target, value } => {
            simplify_ternary_in_expr(target);
            simplify_ternary_in_expr(value);
        }
        Stmt::CompoundAssign { target, value, .. } => {
            simplify_ternary_in_expr(target);
            simplify_ternary_in_expr(value);
        }
        Stmt::Expr(e) => {
            simplify_ternary_in_expr(e);
        }
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            simplify_ternary_in_expr(cond);
            simplify_ternary_to_logical(then_body);
            simplify_ternary_to_logical(else_body);
        }
        Stmt::While { cond, body } => {
            simplify_ternary_in_expr(cond);
            simplify_ternary_to_logical(body);
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            simplify_ternary_to_logical(init);
            simplify_ternary_in_expr(cond);
            simplify_ternary_to_logical(update);
            simplify_ternary_to_logical(body);
        }
        Stmt::Loop { body } => {
            simplify_ternary_to_logical(body);
        }
        Stmt::ForOf { iterable, body, .. } => {
            simplify_ternary_in_expr(iterable);
            simplify_ternary_to_logical(body);
        }
        Stmt::Return(Some(e)) => {
            simplify_ternary_in_expr(e);
        }
        Stmt::Dispatch { blocks, .. } => {
            for (_, block_body) in blocks {
                simplify_ternary_to_logical(block_body);
            }
        }
        Stmt::Switch {
            value,
            cases,
            default_body,
        } => {
            simplify_ternary_in_expr(value);
            for (_, case_body) in cases {
                simplify_ternary_to_logical(case_body);
            }
            simplify_ternary_to_logical(default_body);
        }
        Stmt::Return(None) | Stmt::Break | Stmt::Continue | Stmt::LabeledBreak { .. } => {}
    }
}

fn simplify_ternary_in_expr(expr: &mut Expr) {
    // Recurse into sub-expressions first (bottom-up).
    match expr {
        Expr::Literal(_) | Expr::Var(_) | Expr::GlobalRef(_) => {}
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            simplify_ternary_in_expr(lhs);
            simplify_ternary_in_expr(rhs);
        }
        Expr::LogicalOr { lhs, rhs } | Expr::LogicalAnd { lhs, rhs } => {
            simplify_ternary_in_expr(lhs);
            simplify_ternary_in_expr(rhs);
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::CoroutineResume(inner)
        | Expr::PostIncrement(inner)
        | Expr::Spread(inner) => {
            simplify_ternary_in_expr(inner);
        }
        Expr::Field { object, .. } => {
            simplify_ternary_in_expr(object);
        }
        Expr::Index { collection, index } => {
            simplify_ternary_in_expr(collection);
            simplify_ternary_in_expr(index);
        }
        Expr::Call { args, .. } | Expr::CoroutineCreate { args, .. } => {
            for a in args {
                simplify_ternary_in_expr(a);
            }
        }
        Expr::CallIndirect { callee, args } => {
            simplify_ternary_in_expr(callee);
            for a in args {
                simplify_ternary_in_expr(a);
            }
        }
        Expr::SystemCall { args, .. } => {
            for a in args {
                simplify_ternary_in_expr(a);
            }
        }
        Expr::MethodCall { receiver, args, .. } => {
            simplify_ternary_in_expr(receiver);
            for a in args {
                simplify_ternary_in_expr(a);
            }
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            simplify_ternary_in_expr(cond);
            simplify_ternary_in_expr(then_val);
            simplify_ternary_in_expr(else_val);
        }
        Expr::ArrayInit(elems) | Expr::TupleInit(elems) => {
            for e in elems {
                simplify_ternary_in_expr(e);
            }
        }
        Expr::StructInit { fields, .. } => {
            for (_, v) in fields {
                simplify_ternary_in_expr(v);
            }
        }
        Expr::Yield(v) => {
            if let Some(e) = v {
                simplify_ternary_in_expr(e);
            }
        }
        Expr::MakeClosure { captures, .. } => {
            for c in captures {
                simplify_ternary_in_expr(c);
            }
        }
    }

    // After recursion, check if this is a simplifiable ternary.
    if let Expr::Ternary {
        cond,
        then_val,
        else_val,
    } = expr
    {
        if **cond == **else_val {
            // cond ? then_val : cond -> cond && then_val
            let dummy = Expr::Literal(Constant::Null);
            let old = std::mem::replace(expr, dummy);
            if let Expr::Ternary { cond, then_val, .. } = old {
                *expr = Expr::LogicalAnd {
                    lhs: cond,
                    rhs: then_val,
                };
            }
        } else if **cond == **then_val {
            // cond ? cond : else_val -> cond || else_val
            let dummy = Expr::Literal(Constant::Null);
            let old = std::mem::replace(expr, dummy);
            if let Expr::Ternary { cond, else_val, .. } = old {
                *expr = Expr::LogicalOr {
                    lhs: cond,
                    rhs: else_val,
                };
            }
        }
    }
}

// ---------------------------------------------------------------------------
// For-each (hasNext2) -> for-of rewrite
// ---------------------------------------------------------------------------

/// Rewrite `while (true) { hasNext2 boilerplate ... }` loops into `for (const x of ...)`.
///
/// Recurses into all nested statement bodies.
pub fn rewrite_foreach_loops(body: &mut [Stmt], iterator_system: &str) {
    let mut i = 0;
    while i < body.len() {
        if let Some(for_of) = try_rewrite_foreach(&body[i], iterator_system) {
            body[i] = for_of;
        }
        i += 1;
    }
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                rewrite_foreach_loops(then_body, iterator_system);
                rewrite_foreach_loops(else_body, iterator_system);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                rewrite_foreach_loops(body, iterator_system);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                rewrite_foreach_loops(init, iterator_system);
                rewrite_foreach_loops(update, iterator_system);
                rewrite_foreach_loops(body, iterator_system);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    rewrite_foreach_loops(block_body, iterator_system);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    rewrite_foreach_loops(case_body, iterator_system);
                }
                rewrite_foreach_loops(default_body, iterator_system);
            }
            _ => {}
        }
    }
}

fn try_rewrite_foreach(stmt: &Stmt, iterator_system: &str) -> Option<Stmt> {
    let loop_body = match stmt {
        Stmt::Loop { body } => body,
        _ => return None,
    };

    let hn2_idx = loop_body.iter().position(|s| {
        matches!(
            s,
            Stmt::VarDecl {
                init: Some(Expr::SystemCall { system, method, .. }),
                ..
            } if system == iterator_system && method == "hasNext2"
        )
    })?;

    if loop_body.len() < hn2_idx + 5 {
        return None;
    }

    let (tmp_name, obj_expr) = match &loop_body[hn2_idx] {
        Stmt::VarDecl {
            name,
            init: Some(Expr::SystemCall { args, .. }),
            ..
        } if args.len() == 2 => (name.as_str(), &args[0]),
        _ => return None,
    };

    let obj_name = match_index_assign(&loop_body[hn2_idx + 1], tmp_name, 0)?;
    let idx_name = match_index_assign(&loop_body[hn2_idx + 2], tmp_name, 1)?;

    if !matches!(
        &loop_body[hn2_idx + 3],
        Stmt::If {
            cond: Expr::Not(inner),
            then_body,
            else_body,
        } if else_body.is_empty()
            && then_body.len() == 1
            && matches!(&then_body[0], Stmt::Break)
            && matches!(
                inner.as_ref(),
                Expr::Index { collection, index }
                    if matches!(collection.as_ref(), Expr::Var(v) if v == tmp_name)
                    && matches!(index.as_ref(), Expr::Literal(Constant::Int(2)))
            )
    ) {
        return None;
    }

    let remaining = &loop_body[hn2_idx + 4..];
    let (next_method, next_stmt_idx) =
        find_next_call(remaining, &obj_name, &idx_name, iterator_system)?;

    let wrapper = match next_method {
        "nextValue" => "Object.values",
        "nextName" => "Object.keys",
        _ => return None,
    };

    let iterable = Expr::Call {
        func: wrapper.to_string(),
        args: vec![obj_expr.clone()],
    };

    let mut new_body: Vec<Stmt> = remaining.to_vec();
    let (binding, declare) = extract_binding_and_replace(
        &mut new_body,
        next_stmt_idx,
        &obj_name,
        &idx_name,
        iterator_system,
    )?;

    Some(Stmt::ForOf {
        binding,
        declare,
        binding_ty: Some(Type::Dynamic),
        iterable,
        body: new_body,
    })
}

fn match_index_assign(stmt: &Stmt, tmp_name: &str, index_val: i64) -> Option<String> {
    match stmt {
        Stmt::Assign {
            target: Expr::Var(name),
            value: Expr::Index { collection, index },
        } => {
            if !matches!(collection.as_ref(), Expr::Var(v) if v == tmp_name) {
                return None;
            }
            if !matches!(index.as_ref(), Expr::Literal(Constant::Int(i)) if *i == index_val) {
                return None;
            }
            Some(name.clone())
        }
        _ => None,
    }
}

fn find_next_call<'a>(
    body: &'a [Stmt],
    obj_name: &str,
    idx_name: &str,
    iterator_system: &str,
) -> Option<(&'a str, usize)> {
    for (i, stmt) in body.iter().enumerate() {
        if let Some(method) = stmt_contains_next_call(stmt, obj_name, idx_name, iterator_system) {
            return Some((method, i));
        }
    }
    None
}

fn stmt_contains_next_call<'a>(
    stmt: &'a Stmt,
    obj_name: &str,
    idx_name: &str,
    iterator_system: &str,
) -> Option<&'a str> {
    match stmt {
        Stmt::VarDecl { init: Some(e), .. } => {
            expr_contains_next_call(e, obj_name, idx_name, iterator_system)
        }
        Stmt::Assign { value, .. } => {
            expr_contains_next_call(value, obj_name, idx_name, iterator_system)
        }
        Stmt::Expr(e) => expr_contains_next_call(e, obj_name, idx_name, iterator_system),
        Stmt::If { cond, .. } => expr_contains_next_call(cond, obj_name, idx_name, iterator_system),
        _ => None,
    }
}

fn expr_contains_next_call<'a>(
    expr: &'a Expr,
    obj_name: &str,
    idx_name: &str,
    iterator_system: &str,
) -> Option<&'a str> {
    match expr {
        Expr::SystemCall {
            system,
            method,
            args,
        } if system == iterator_system
            && (method == "nextValue" || method == "nextName")
            && args.len() == 2
            && matches!(&args[0], Expr::Var(v) if v == obj_name)
            && matches!(&args[1], Expr::Var(v) if v == idx_name) =>
        {
            Some(method.as_str())
        }
        Expr::Cast { expr, .. } => {
            expr_contains_next_call(expr, obj_name, idx_name, iterator_system)
        }
        Expr::Call { args, .. } | Expr::SystemCall { args, .. } => args
            .iter()
            .find_map(|a| expr_contains_next_call(a, obj_name, idx_name, iterator_system)),
        Expr::CallIndirect { callee, args } => {
            expr_contains_next_call(callee, obj_name, idx_name, iterator_system).or_else(|| {
                args.iter()
                    .find_map(|a| expr_contains_next_call(a, obj_name, idx_name, iterator_system))
            })
        }
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            expr_contains_next_call(lhs, obj_name, idx_name, iterator_system)
                .or_else(|| expr_contains_next_call(rhs, obj_name, idx_name, iterator_system))
        }
        Expr::Unary { expr, .. } | Expr::Not(expr) => {
            expr_contains_next_call(expr, obj_name, idx_name, iterator_system)
        }
        Expr::Index { collection, index } => {
            expr_contains_next_call(collection, obj_name, idx_name, iterator_system)
                .or_else(|| expr_contains_next_call(index, obj_name, idx_name, iterator_system))
        }
        Expr::Field { object, .. } => {
            expr_contains_next_call(object, obj_name, idx_name, iterator_system)
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => expr_contains_next_call(cond, obj_name, idx_name, iterator_system)
            .or_else(|| expr_contains_next_call(then_val, obj_name, idx_name, iterator_system))
            .or_else(|| expr_contains_next_call(else_val, obj_name, idx_name, iterator_system)),
        Expr::ArrayInit(elems) => elems
            .iter()
            .find_map(|e| expr_contains_next_call(e, obj_name, idx_name, iterator_system)),
        Expr::TypeCheck { expr, .. } => {
            expr_contains_next_call(expr, obj_name, idx_name, iterator_system)
        }
        _ => None,
    }
}

fn extract_binding_and_replace(
    body: &mut Vec<Stmt>,
    idx: usize,
    obj_name: &str,
    idx_name: &str,
    iterator_system: &str,
) -> Option<(String, bool)> {
    if let Stmt::VarDecl {
        name,
        init: Some(init),
        ..
    } = &body[idx]
    {
        let is_direct = is_next_call(init, obj_name, idx_name, iterator_system);
        let is_cast_wrapped = matches!(
            init,
            Expr::Cast { expr, .. } if is_next_call(expr, obj_name, idx_name, iterator_system)
        );
        if is_direct || is_cast_wrapped {
            let binding = name.clone();
            body.remove(idx);
            return Some((binding, true));
        }
    }

    if let Stmt::Assign {
        target: Expr::Var(name),
        value,
    } = &body[idx]
    {
        let is_direct = is_next_call(value, obj_name, idx_name, iterator_system);
        let is_cast_wrapped = matches!(
            value,
            Expr::Cast { expr, .. } if is_next_call(expr, obj_name, idx_name, iterator_system)
        );
        if is_direct || is_cast_wrapped {
            let binding = name.clone();
            body.remove(idx);
            return Some((binding, false));
        }
    }

    let binding = "$item".to_string();
    let replaced = replace_next_call_in_stmt(
        &mut body[idx],
        obj_name,
        idx_name,
        &binding,
        iterator_system,
    );
    if replaced {
        Some((binding, true))
    } else {
        None
    }
}

fn is_next_call(expr: &Expr, obj_name: &str, idx_name: &str, iterator_system: &str) -> bool {
    matches!(
        expr,
        Expr::SystemCall { system, method, args }
            if system == iterator_system
            && (method == "nextValue" || method == "nextName")
            && args.len() == 2
            && matches!(&args[0], Expr::Var(v) if v == obj_name)
            && matches!(&args[1], Expr::Var(v) if v == idx_name)
    )
}

fn replace_next_call_in_stmt(
    stmt: &mut Stmt,
    obj_name: &str,
    idx_name: &str,
    binding: &str,
    iterator_system: &str,
) -> bool {
    match stmt {
        Stmt::VarDecl { init: Some(e), .. } => {
            replace_next_call_in_expr(e, obj_name, idx_name, binding, iterator_system)
        }
        Stmt::Assign { value, .. } => {
            replace_next_call_in_expr(value, obj_name, idx_name, binding, iterator_system)
        }
        Stmt::Expr(e) => replace_next_call_in_expr(e, obj_name, idx_name, binding, iterator_system),
        Stmt::CompoundAssign { value, .. } => {
            replace_next_call_in_expr(value, obj_name, idx_name, binding, iterator_system)
        }
        Stmt::If { cond, .. } => {
            replace_next_call_in_expr(cond, obj_name, idx_name, binding, iterator_system)
        }
        _ => false,
    }
}

fn replace_next_call_in_expr(
    expr: &mut Expr,
    obj_name: &str,
    idx_name: &str,
    binding: &str,
    iterator_system: &str,
) -> bool {
    if is_next_call(expr, obj_name, idx_name, iterator_system) {
        *expr = Expr::Var(binding.to_string());
        return true;
    }
    if let Expr::Cast { expr: inner, .. } = expr {
        if is_next_call(inner, obj_name, idx_name, iterator_system) {
            *expr = Expr::Var(binding.to_string());
            return true;
        }
    }

    match expr {
        Expr::Cast { expr, .. } => {
            replace_next_call_in_expr(expr, obj_name, idx_name, binding, iterator_system)
        }
        Expr::Call { args, .. } | Expr::SystemCall { args, .. } => args
            .iter_mut()
            .any(|a| replace_next_call_in_expr(a, obj_name, idx_name, binding, iterator_system)),
        Expr::CallIndirect { callee, args } => {
            replace_next_call_in_expr(callee, obj_name, idx_name, binding, iterator_system)
                || args.iter_mut().any(|a| {
                    replace_next_call_in_expr(a, obj_name, idx_name, binding, iterator_system)
                })
        }
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            replace_next_call_in_expr(lhs, obj_name, idx_name, binding, iterator_system)
                || replace_next_call_in_expr(rhs, obj_name, idx_name, binding, iterator_system)
        }
        Expr::Unary { expr, .. } | Expr::Not(expr) => {
            replace_next_call_in_expr(expr, obj_name, idx_name, binding, iterator_system)
        }
        Expr::Index { collection, index } => {
            replace_next_call_in_expr(collection, obj_name, idx_name, binding, iterator_system)
                || replace_next_call_in_expr(index, obj_name, idx_name, binding, iterator_system)
        }
        Expr::Field { object, .. } => {
            replace_next_call_in_expr(object, obj_name, idx_name, binding, iterator_system)
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            replace_next_call_in_expr(cond, obj_name, idx_name, binding, iterator_system)
                || replace_next_call_in_expr(then_val, obj_name, idx_name, binding, iterator_system)
                || replace_next_call_in_expr(else_val, obj_name, idx_name, binding, iterator_system)
        }
        Expr::ArrayInit(elems) => elems
            .iter_mut()
            .any(|e| replace_next_call_in_expr(e, obj_name, idx_name, binding, iterator_system)),
        Expr::TypeCheck { expr, .. } => {
            replace_next_call_in_expr(expr, obj_name, idx_name, binding, iterator_system)
        }
        Expr::LogicalOr { lhs, rhs } | Expr::LogicalAnd { lhs, rhs } => {
            replace_next_call_in_expr(lhs, obj_name, idx_name, binding, iterator_system)
                || replace_next_call_in_expr(rhs, obj_name, idx_name, binding, iterator_system)
        }
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Compound assignment rewrite
// ---------------------------------------------------------------------------

/// Rewrite `target = target op value` to `target op= value`.
///
/// Recurses into all nested statement bodies.
pub fn rewrite_compound_assign(body: &mut [Stmt]) {
    for stmt in body.iter_mut() {
        recurse_into_stmt(stmt, rewrite_compound_assign);

        let replacement = match stmt {
            Stmt::Assign { target, value } => match_compound_assign(target, value),
            _ => None,
        };

        if let Some(new_stmt) = replacement {
            *stmt = new_stmt;
        }
    }
}

fn match_compound_assign(target: &Expr, value: &Expr) -> Option<Stmt> {
    let (op, lhs, rhs) = match value {
        Expr::Binary { op, lhs, rhs } => (*op, lhs.as_ref(), rhs.as_ref()),
        _ => return None,
    };

    if strip_as_type(lhs) != target {
        return None;
    }

    Some(Stmt::CompoundAssign {
        target: target.clone(),
        op,
        value: rhs.clone(),
    })
}

// ---------------------------------------------------------------------------
// Post-increment rewrite
// ---------------------------------------------------------------------------

/// Rewrite read-modify-write patterns to post-increment.
///
/// Recurses into all nested statement bodies.
pub fn rewrite_post_increment(body: &mut Vec<Stmt>) {
    loop {
        if !try_rewrite_one_post_increment(body) {
            break;
        }
    }
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                rewrite_post_increment(then_body);
                rewrite_post_increment(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                rewrite_post_increment(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                rewrite_post_increment(init);
                rewrite_post_increment(update);
                rewrite_post_increment(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    rewrite_post_increment(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    rewrite_post_increment(case_body);
                }
                rewrite_post_increment(default_body);
            }
            _ => {}
        }
    }
}

fn try_rewrite_one_post_increment(body: &mut Vec<Stmt>) -> bool {
    for i in 0..body.len().saturating_sub(1) {
        let (var_name, target) = match &body[i] {
            Stmt::VarDecl {
                name,
                init: Some(init),
                mutable: false,
                ..
            } => (name.clone(), init.clone()),
            _ => continue,
        };

        let is_increment = match &body[i + 1] {
            Stmt::Assign { target: tgt, value } => {
                tgt == &target && is_var_plus_one(value, &var_name)
            }
            _ => false,
        };
        if !is_increment {
            continue;
        }

        let remaining_refs: usize = body[i + 2..]
            .iter()
            .map(|s| count_var_reads_in_stmt(s, &var_name))
            .sum();

        if remaining_refs == 1 {
            let inc_expr = Expr::PostIncrement(Box::new(target));
            let mut replacement = Some(inc_expr);
            for s in &mut body[i + 2..] {
                if substitute_var_in_stmt(s, &var_name, &mut replacement) {
                    break;
                }
            }
            body.remove(i + 1);
            body.remove(i);
            return true;
        } else if remaining_refs == 0 {
            let inc_expr = Expr::PostIncrement(Box::new(target));
            body.remove(i + 1);
            body[i] = Stmt::Expr(inc_expr);
            return true;
        }
    }
    false
}

fn is_var_plus_one(expr: &Expr, var_name: &str) -> bool {
    if let Expr::Binary {
        op: BinOp::Add,
        lhs,
        rhs,
    } = expr
    {
        let is_one = match rhs.as_ref() {
            Expr::Literal(Constant::Int(1)) => true,
            Expr::Literal(Constant::Float(f)) => *f == 1.0,
            _ => false,
        };
        if !is_one {
            return false;
        }
        match lhs.as_ref() {
            Expr::Var(n) => n == var_name,
            Expr::Cast { expr: inner, .. } => {
                matches!(inner.as_ref(), Expr::Var(n) if n == var_name)
            }
            _ => false,
        }
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// While -> For loop promotion
// ---------------------------------------------------------------------------

/// Promote `let i = init; while (cond) { body; i += step; }` to
/// `for (let i = init; cond; i += step) { body }`.
pub fn promote_while_to_for(body: &mut Vec<Stmt>) {
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                promote_while_to_for(then_body);
                promote_while_to_for(else_body);
            }
            Stmt::While { body: wb, .. }
            | Stmt::Loop { body: wb }
            | Stmt::For { body: wb, .. }
            | Stmt::ForOf { body: wb, .. } => {
                promote_while_to_for(wb);
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    promote_while_to_for(case_body);
                }
                promote_while_to_for(default_body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    promote_while_to_for(block_body);
                }
            }
            _ => {}
        }
    }

    let mut i = 0;
    while i < body.len() {
        if !matches!(&body[i], Stmt::While { .. }) {
            i += 1;
            continue;
        }

        let while_cond_var = if let Stmt::While { cond, .. } = &body[i] {
            extract_cmp_var(cond)
        } else {
            None
        };

        let Some(var_name) = while_cond_var else {
            i += 1;
            continue;
        };

        let mut init_idx = None;
        if i > 0 {
            for j in (0..i).rev() {
                let is_init = match &body[j] {
                    Stmt::VarDecl {
                        name,
                        init: Some(_),
                        mutable: true,
                        ..
                    } => name == &var_name,
                    Stmt::Assign {
                        target: Expr::Var(name),
                        ..
                    } => name == &var_name,
                    _ => false,
                };
                if is_init {
                    init_idx = Some(j);
                    break;
                }
                if stmt_references_for_promote(&body[j], &var_name) {
                    break;
                }
            }
        }

        let Some(init_j) = init_idx else {
            i += 1;
            continue;
        };

        let mut init_stmt = body.remove(init_j);
        let while_idx = i - 1;

        let has_post_loop_refs = matches!(
            &init_stmt,
            Stmt::VarDecl {
                mutable: true,
                init: Some(_),
                ..
            }
        ) && body[(while_idx + 1)..]
            .iter()
            .any(|s| stmt_references_for_promote(s, &var_name));

        if has_post_loop_refs {
            let Stmt::VarDecl {
                name,
                ty,
                init: Some(val),
                ..
            } = init_stmt
            else {
                unreachable!()
            };
            let scope_decl = Stmt::VarDecl {
                name: name.clone(),
                ty: ty.clone(),
                init: None,
                mutable: true,
            };
            body.insert(init_j, scope_decl);
            let adj_while_idx = while_idx + 1;
            let mut assign_init = Stmt::Assign {
                target: Expr::Var(name),
                value: val,
            };
            if let Some(promoted) =
                try_promote_while(&var_name, &mut assign_init, &mut body[adj_while_idx])
            {
                body[adj_while_idx] = promoted;
                i = adj_while_idx;
                continue;
            }
            body.insert(adj_while_idx, assign_init);
        } else if let Some(promoted) =
            try_promote_while(&var_name, &mut init_stmt, &mut body[while_idx])
        {
            body[while_idx] = promoted;
            i = while_idx;
            continue;
        } else {
            body.insert(init_j, init_stmt);
        }
        i += 1;
    }
}

fn extract_cmp_var(expr: &Expr) -> Option<String> {
    if let Expr::Cmp { lhs, rhs, .. } = expr {
        if let Expr::Var(name) = lhs.as_ref() {
            return Some(name.clone());
        }
        if let Expr::Var(name) = rhs.as_ref() {
            return Some(name.clone());
        }
    }
    None
}

/// Check if a statement references the named variable (reads or writes).
/// Used by promote_while_to_for.
fn stmt_references_for_promote(stmt: &Stmt, name: &str) -> bool {
    match stmt {
        Stmt::VarDecl {
            name: decl_name,
            init,
            ..
        } => {
            decl_name == name
                || init
                    .as_ref()
                    .is_some_and(|e| expr_references_for_promote(e, name))
        }
        Stmt::Assign { target, value } => {
            expr_references_for_promote(target, name) || expr_references_for_promote(value, name)
        }
        Stmt::CompoundAssign { target, value, .. } => {
            expr_references_for_promote(target, name) || expr_references_for_promote(value, name)
        }
        Stmt::Expr(e) => expr_references_for_promote(e, name),
        _ => true, // Conservatively assume anything else references the var.
    }
}

fn expr_references_for_promote(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Var(n) => n == name,
        Expr::Literal(_) | Expr::GlobalRef(_) => false,
        Expr::Binary { lhs, rhs, .. }
        | Expr::Cmp { lhs, rhs, .. }
        | Expr::LogicalOr { lhs, rhs }
        | Expr::LogicalAnd { lhs, rhs } => {
            expr_references_for_promote(lhs, name) || expr_references_for_promote(rhs, name)
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::CoroutineResume(inner)
        | Expr::PostIncrement(inner)
        | Expr::Spread(inner) => expr_references_for_promote(inner, name),
        Expr::Field { object, .. } => expr_references_for_promote(object, name),
        Expr::Index { collection, index } => {
            expr_references_for_promote(collection, name)
                || expr_references_for_promote(index, name)
        }
        Expr::Call { args, .. }
        | Expr::CoroutineCreate { args, .. }
        | Expr::SystemCall { args, .. } => {
            args.iter().any(|a| expr_references_for_promote(a, name))
        }
        Expr::CallIndirect { callee, args } => {
            expr_references_for_promote(callee, name)
                || args.iter().any(|a| expr_references_for_promote(a, name))
        }
        Expr::MethodCall { receiver, args, .. } => {
            expr_references_for_promote(receiver, name)
                || args.iter().any(|a| expr_references_for_promote(a, name))
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            expr_references_for_promote(cond, name)
                || expr_references_for_promote(then_val, name)
                || expr_references_for_promote(else_val, name)
        }
        Expr::ArrayInit(elems) | Expr::TupleInit(elems) => {
            elems.iter().any(|e| expr_references_for_promote(e, name))
        }
        Expr::StructInit { fields, .. } => fields
            .iter()
            .any(|(_, v)| expr_references_for_promote(v, name)),
        Expr::Yield(v) => v
            .as_ref()
            .is_some_and(|e| expr_references_for_promote(e, name)),
        Expr::MakeClosure { captures, .. } => captures
            .iter()
            .any(|c| expr_references_for_promote(c, name)),
    }
}

/// Collect variable names declared at the top level of a statement list.
fn collect_top_level_decls(stmts: &[Stmt]) -> HashSet<&str> {
    stmts
        .iter()
        .filter_map(|s| {
            if let Stmt::VarDecl { name, .. } = s {
                Some(name.as_str())
            } else {
                None
            }
        })
        .collect()
}

/// Check if a statement references any variable in the given set.
fn stmt_references_any(stmt: &Stmt, names: &HashSet<&str>) -> bool {
    names.iter().any(|n| stmt_references_for_promote(stmt, n))
}

fn is_var_update(stmt: &Stmt, name: &str) -> bool {
    match stmt {
        Stmt::CompoundAssign { target, .. } => matches!(target, Expr::Var(n) if n == name),
        Stmt::Assign { target, value } => {
            matches!(target, Expr::Var(n) if n == name)
                && matches!(
                    value,
                    Expr::Binary { lhs, .. } if matches!(strip_as_type(lhs.as_ref()), Expr::Var(n) if n == name)
                )
        }
        _ => false,
    }
}

fn try_promote_while(var_name: &str, init_stmt: &mut Stmt, while_stmt: &mut Stmt) -> Option<Stmt> {
    let Stmt::While { cond, body } = while_stmt else {
        return None;
    };

    if !expr_references_for_promote(cond, var_name) {
        return None;
    }

    let extract_init = |init_stmt: &mut Stmt| -> Stmt {
        std::mem::replace(init_stmt, Stmt::Expr(Expr::Literal(Constant::Null)))
    };

    // Pattern 1: tail increment.
    if body.len() >= 2 && is_var_update(body.last().unwrap(), var_name) {
        // Guard: if the update references a variable declared in the body,
        // extracting it into the for-header would reference it before declaration.
        let body_decls = collect_top_level_decls(&body[..body.len() - 1]);
        if !body_decls.is_empty() && stmt_references_any(body.last().unwrap(), &body_decls) {
            return None;
        }
        let update_stmt = body.pop().unwrap();
        let init = extract_init(init_stmt);
        let cond = std::mem::replace(cond, Expr::Literal(Constant::Null));
        let body = std::mem::take(body);
        return Some(Stmt::For {
            init: vec![init],
            cond,
            update: vec![update_stmt],
            body,
        });
    }

    // Pattern 2: else-continue increment.
    if body.len() == 1 {
        if let Stmt::If { else_body, .. } = &body[0] {
            if else_body.len() >= 2
                && matches!(else_body.last(), Some(Stmt::Continue))
                && is_var_update(&else_body[else_body.len() - 2], var_name)
            {
                let Stmt::If {
                    cond: if_cond,
                    then_body: if_then,
                    else_body: if_else,
                } = std::mem::replace(&mut body[0], Stmt::Expr(Expr::Literal(Constant::Null)))
                else {
                    unreachable!()
                };

                let mut if_else = if_else;
                if_else.pop(); // Remove Continue.
                let update_stmt = if_else.pop().unwrap(); // Remove increment.

                let new_body = if if_else.is_empty() {
                    vec![Stmt::If {
                        cond: if_cond,
                        then_body: if_then,
                        else_body: vec![],
                    }]
                } else {
                    vec![Stmt::If {
                        cond: if_cond,
                        then_body: if_then,
                        else_body: if_else,
                    }]
                };

                let init = extract_init(init_stmt);
                let loop_cond = std::mem::replace(cond, Expr::Literal(Constant::Null));
                return Some(Stmt::For {
                    init: vec![init],
                    cond: loop_cond,
                    update: vec![update_stmt],
                    body: new_body,
                });
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::ast::{BinOp, Expr, Stmt};
    use crate::ir::inst::CmpKind;
    use crate::ir::value::Constant;

    fn var(name: &str) -> Expr {
        Expr::Var(name.to_string())
    }

    fn int(n: i64) -> Expr {
        Expr::Literal(Constant::Int(n))
    }

    fn assign(target: Expr, value: Expr) -> Stmt {
        Stmt::Assign { target, value }
    }

    fn str_lit(s: &str) -> Expr {
        Expr::Literal(Constant::String(s.to_string()))
    }

    #[test]
    fn ternary_rewrite_basic() {
        let mut body = vec![Stmt::If {
            cond: var("c"),
            then_body: vec![assign(var("x"), int(1))],
            else_body: vec![assign(var("x"), int(2))],
        }];

        rewrite_ternary(&mut body);

        assert_eq!(body.len(), 1);
        match &body[0] {
            Stmt::Assign { target, value } => {
                assert_eq!(*target, var("x"));
                match value {
                    Expr::Ternary {
                        cond,
                        then_val,
                        else_val,
                    } => {
                        assert_eq!(**cond, var("c"));
                        assert_eq!(**then_val, int(1));
                        assert_eq!(**else_val, int(2));
                    }
                    other => panic!("Expected Ternary, got: {other:?}"),
                }
            }
            other => panic!("Expected Assign, got: {other:?}"),
        }
    }

    #[test]
    fn ternary_no_rewrite_different_targets() {
        let mut body = vec![Stmt::If {
            cond: var("c"),
            then_body: vec![assign(var("x"), int(1))],
            else_body: vec![assign(var("y"), int(2))],
        }];

        rewrite_ternary(&mut body);
        assert!(matches!(&body[0], Stmt::If { .. }));
    }

    #[test]
    fn ternary_no_rewrite_multi_stmt() {
        let mut body = vec![Stmt::If {
            cond: var("c"),
            then_body: vec![assign(var("x"), int(1)), assign(var("y"), int(2))],
            else_body: vec![assign(var("x"), int(3))],
        }];

        rewrite_ternary(&mut body);
        assert!(matches!(&body[0], Stmt::If { .. }));
    }

    #[test]
    fn ternary_recurses_into_nested() {
        let inner_if = Stmt::If {
            cond: var("c"),
            then_body: vec![assign(var("x"), int(1))],
            else_body: vec![assign(var("x"), int(2))],
        };
        let mut body = vec![Stmt::While {
            cond: var("true"),
            body: vec![inner_if],
        }];

        rewrite_ternary(&mut body);

        match &body[0] {
            Stmt::While { body, .. } => match &body[0] {
                Stmt::Assign { value, .. } => {
                    assert!(matches!(value, Expr::Ternary { .. }));
                }
                other => panic!("Expected Assign, got: {other:?}"),
            },
            other => panic!("Expected While, got: {other:?}"),
        }
    }

    #[test]
    fn minmax_rewrite_ge_max() {
        let mut body = vec![assign(
            var("x"),
            Expr::Ternary {
                cond: Box::new(Expr::Cmp {
                    kind: CmpKind::Ge,
                    lhs: Box::new(var("a")),
                    rhs: Box::new(var("b")),
                }),
                then_val: Box::new(var("a")),
                else_val: Box::new(var("b")),
            },
        )];

        rewrite_minmax(&mut body);

        match &body[0] {
            Stmt::Assign { value, .. } => match value {
                Expr::Call { func, args } => {
                    assert_eq!(func, "Math.max");
                    assert_eq!(args.len(), 2);
                    assert_eq!(args[0], var("a"));
                    assert_eq!(args[1], var("b"));
                }
                other => panic!("Expected Call, got: {other:?}"),
            },
            other => panic!("Expected Assign, got: {other:?}"),
        }
    }

    #[test]
    fn minmax_rewrite_ge_min() {
        let mut body = vec![assign(
            var("x"),
            Expr::Ternary {
                cond: Box::new(Expr::Cmp {
                    kind: CmpKind::Ge,
                    lhs: Box::new(var("a")),
                    rhs: Box::new(var("b")),
                }),
                then_val: Box::new(var("b")),
                else_val: Box::new(var("a")),
            },
        )];

        rewrite_minmax(&mut body);

        match &body[0] {
            Stmt::Assign { value, .. } => match value {
                Expr::Call { func, args } => {
                    assert_eq!(func, "Math.min");
                    assert_eq!(args.len(), 2);
                }
                other => panic!("Expected Call, got: {other:?}"),
            },
            other => panic!("Expected Assign, got: {other:?}"),
        }
    }

    #[test]
    fn minmax_rewrite_le_min() {
        let mut body = vec![assign(
            var("x"),
            Expr::Ternary {
                cond: Box::new(Expr::Cmp {
                    kind: CmpKind::Le,
                    lhs: Box::new(var("a")),
                    rhs: Box::new(var("b")),
                }),
                then_val: Box::new(var("a")),
                else_val: Box::new(var("b")),
            },
        )];

        rewrite_minmax(&mut body);

        match &body[0] {
            Stmt::Assign { value, .. } => match value {
                Expr::Call { func, .. } => assert_eq!(func, "Math.min"),
                other => panic!("Expected Call, got: {other:?}"),
            },
            other => panic!("Expected Assign, got: {other:?}"),
        }
    }

    #[test]
    fn minmax_rewrite_le_max() {
        let mut body = vec![assign(
            var("x"),
            Expr::Ternary {
                cond: Box::new(Expr::Cmp {
                    kind: CmpKind::Le,
                    lhs: Box::new(var("a")),
                    rhs: Box::new(var("b")),
                }),
                then_val: Box::new(var("b")),
                else_val: Box::new(var("a")),
            },
        )];

        rewrite_minmax(&mut body);

        match &body[0] {
            Stmt::Assign { value, .. } => match value {
                Expr::Call { func, .. } => assert_eq!(func, "Math.max"),
                other => panic!("Expected Call, got: {other:?}"),
            },
            other => panic!("Expected Assign, got: {other:?}"),
        }
    }

    #[test]
    fn minmax_no_rewrite_mismatched_operands() {
        let mut body = vec![assign(
            var("x"),
            Expr::Ternary {
                cond: Box::new(Expr::Cmp {
                    kind: CmpKind::Ge,
                    lhs: Box::new(var("a")),
                    rhs: Box::new(var("b")),
                }),
                then_val: Box::new(var("c")),
                else_val: Box::new(var("d")),
            },
        )];

        rewrite_minmax(&mut body);
        assert!(matches!(
            &body[0],
            Stmt::Assign {
                value: Expr::Ternary { .. },
                ..
            }
        ));
    }

    #[test]
    fn minmax_no_rewrite_eq() {
        let mut body = vec![assign(
            var("x"),
            Expr::Ternary {
                cond: Box::new(Expr::Cmp {
                    kind: CmpKind::Eq,
                    lhs: Box::new(var("a")),
                    rhs: Box::new(var("b")),
                }),
                then_val: Box::new(var("a")),
                else_val: Box::new(var("b")),
            },
        )];

        rewrite_minmax(&mut body);
        assert!(matches!(
            &body[0],
            Stmt::Assign {
                value: Expr::Ternary { .. },
                ..
            }
        ));
    }

    #[test]
    fn combined_ternary_then_minmax() {
        let mut body = vec![Stmt::If {
            cond: Expr::Cmp {
                kind: CmpKind::Ge,
                lhs: Box::new(var("a")),
                rhs: Box::new(var("b")),
            },
            then_body: vec![assign(var("x"), var("a"))],
            else_body: vec![assign(var("x"), var("b"))],
        }];

        rewrite_ternary(&mut body);
        rewrite_minmax(&mut body);

        match &body[0] {
            Stmt::Assign { value, .. } => match value {
                Expr::Call { func, args } => {
                    assert_eq!(func, "Math.max");
                    assert_eq!(args[0], var("a"));
                    assert_eq!(args[1], var("b"));
                }
                other => panic!("Expected Call, got: {other:?}"),
            },
            other => panic!("Expected Assign, got: {other:?}"),
        }
    }

    #[test]
    fn minmax_with_expressions() {
        let a_plus_1 = Expr::Binary {
            op: BinOp::Add,
            lhs: Box::new(var("a")),
            rhs: Box::new(int(1)),
        };
        let b_times_2 = Expr::Binary {
            op: BinOp::Mul,
            lhs: Box::new(var("b")),
            rhs: Box::new(int(2)),
        };

        let mut body = vec![assign(
            var("x"),
            Expr::Ternary {
                cond: Box::new(Expr::Cmp {
                    kind: CmpKind::Ge,
                    lhs: Box::new(a_plus_1.clone()),
                    rhs: Box::new(b_times_2.clone()),
                }),
                then_val: Box::new(a_plus_1),
                else_val: Box::new(b_times_2),
            },
        )];

        rewrite_minmax(&mut body);

        match &body[0] {
            Stmt::Assign { value, .. } => match value {
                Expr::Call { func, .. } => assert_eq!(func, "Math.max"),
                other => panic!("Expected Call, got: {other:?}"),
            },
            other => panic!("Expected Assign, got: {other:?}"),
        }
    }

    #[test]
    fn compound_assign_basic_sub() {
        let mut body = vec![assign(
            var("HP"),
            Expr::Binary {
                op: BinOp::Sub,
                lhs: Box::new(var("HP")),
                rhs: Box::new(var("damage")),
            },
        )];

        rewrite_compound_assign(&mut body);

        match &body[0] {
            Stmt::CompoundAssign { target, op, value } => {
                assert_eq!(*target, var("HP"));
                assert_eq!(*op, BinOp::Sub);
                assert_eq!(*value, var("damage"));
            }
            other => panic!("Expected CompoundAssign, got: {other:?}"),
        }
    }

    #[test]
    fn compound_assign_add() {
        let mut body = vec![assign(
            var("x"),
            Expr::Binary {
                op: BinOp::Add,
                lhs: Box::new(var("x")),
                rhs: Box::new(int(1)),
            },
        )];

        rewrite_compound_assign(&mut body);

        match &body[0] {
            Stmt::CompoundAssign { target, op, value } => {
                assert_eq!(*target, var("x"));
                assert_eq!(*op, BinOp::Add);
                assert_eq!(*value, int(1));
            }
            other => panic!("Expected CompoundAssign, got: {other:?}"),
        }
    }

    #[test]
    fn compound_assign_no_rewrite_rhs_match() {
        let mut body = vec![assign(
            var("x"),
            Expr::Binary {
                op: BinOp::Add,
                lhs: Box::new(var("y")),
                rhs: Box::new(var("x")),
            },
        )];

        rewrite_compound_assign(&mut body);
        assert!(matches!(&body[0], Stmt::Assign { .. }));
    }

    #[test]
    fn compound_assign_no_rewrite_different_target() {
        let mut body = vec![assign(
            var("x"),
            Expr::Binary {
                op: BinOp::Sub,
                lhs: Box::new(var("y")),
                rhs: Box::new(var("z")),
            },
        )];

        rewrite_compound_assign(&mut body);
        assert!(matches!(&body[0], Stmt::Assign { .. }));
    }

    #[test]
    fn compound_assign_field_access() {
        let field = Expr::Field {
            object: Box::new(var("this")),
            field: "HP".to_string(),
        };
        let mut body = vec![assign(
            field.clone(),
            Expr::Binary {
                op: BinOp::Mul,
                lhs: Box::new(field.clone()),
                rhs: Box::new(int(2)),
            },
        )];

        rewrite_compound_assign(&mut body);

        match &body[0] {
            Stmt::CompoundAssign { target, op, value } => {
                assert_eq!(*target, field);
                assert_eq!(*op, BinOp::Mul);
                assert_eq!(*value, int(2));
            }
            other => panic!("Expected CompoundAssign, got: {other:?}"),
        }
    }

    #[test]
    fn compound_assign_recurses_into_nested() {
        let inner = assign(
            var("x"),
            Expr::Binary {
                op: BinOp::Add,
                lhs: Box::new(var("x")),
                rhs: Box::new(int(1)),
            },
        );
        let mut body = vec![Stmt::While {
            cond: var("true"),
            body: vec![inner],
        }];

        rewrite_compound_assign(&mut body);

        match &body[0] {
            Stmt::While { body, .. } => {
                assert!(matches!(&body[0], Stmt::CompoundAssign { .. }));
            }
            other => panic!("Expected While, got: {other:?}"),
        }
    }

    #[test]
    fn compound_assign_bitwise_ops() {
        let mut body = vec![assign(
            var("x"),
            Expr::Binary {
                op: BinOp::BitOr,
                lhs: Box::new(var("x")),
                rhs: Box::new(var("mask")),
            },
        )];

        rewrite_compound_assign(&mut body);

        match &body[0] {
            Stmt::CompoundAssign { op, .. } => {
                assert_eq!(*op, BinOp::BitOr);
            }
            other => panic!("Expected CompoundAssign, got: {other:?}"),
        }
    }

    // Regression: 868decb
    #[test]
    fn ternary_with_passthrough_branch() {
        use super::super::cleanup::eliminate_self_assigns;

        let mut body = vec![Stmt::If {
            cond: var("c"),
            then_body: vec![assign(var("x"), int(1))],
            else_body: vec![assign(var("x"), var("x"))],
        }];

        rewrite_ternary(&mut body);
        assert!(matches!(&body[0], Stmt::If { .. }), "Should remain If");

        eliminate_self_assigns(&mut body);

        assert_eq!(body.len(), 1);
        match &body[0] {
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                assert_eq!(*cond, var("c"));
                assert_eq!(then_body.len(), 1);
                assert!(matches!(&then_body[0], Stmt::Assign { target, value }
                    if *target == var("x") && *value == int(1)));
                assert!(else_body.is_empty());
            }
            other => panic!("Expected If, got: {other:?}"),
        }
    }

    #[test]
    fn lower_output_nodes_rewrites_harlowe_h() {
        let mut body = vec![Stmt::Expr(Expr::SystemCall {
            system: "Harlowe.H".into(),
            method: "text".into(),
            args: vec![str_lit("hello")],
        })];
        lower_output_nodes(&mut body, "Harlowe.H", "h");
        match &body[0] {
            Stmt::Expr(Expr::MethodCall {
                receiver,
                method,
                args,
            }) => {
                assert_eq!(**receiver, var("h"));
                assert_eq!(method, "text");
                assert_eq!(args.len(), 1);
                assert_eq!(args[0], str_lit("hello"));
            }
            other => panic!("Expected MethodCall h.text, got: {other:?}"),
        }
    }

    #[test]
    fn lower_output_nodes_harlowe_h_no_args() {
        let mut body = vec![Stmt::Expr(Expr::SystemCall {
            system: "Harlowe.H".into(),
            method: "br".into(),
            args: vec![],
        })];
        lower_output_nodes(&mut body, "Harlowe.H", "h");
        match &body[0] {
            Stmt::Expr(Expr::MethodCall {
                receiver,
                method,
                args,
            }) => {
                assert_eq!(**receiver, var("h"));
                assert_eq!(method, "br");
                assert!(args.is_empty());
            }
            other => panic!("Expected MethodCall h.br, got: {other:?}"),
        }
    }

    #[test]
    fn lower_output_nodes_ignores_other_systems() {
        let mut body = vec![Stmt::Expr(Expr::SystemCall {
            system: "Harlowe.State".into(),
            method: "get".into(),
            args: vec![str_lit("x")],
        })];
        lower_output_nodes(&mut body, "Harlowe.H", "h");
        match &body[0] {
            Stmt::Expr(Expr::SystemCall { system, method, .. }) => {
                assert_eq!(system, "Harlowe.State");
                assert_eq!(method, "get");
            }
            other => panic!("Expected SystemCall passthrough, got: {other:?}"),
        }
    }

    #[test]
    fn promote_while_mutable_vardecl_stays_at_scope() {
        let cmp_i_n = Expr::Cmp {
            kind: CmpKind::Lt,
            lhs: Box::new(Expr::Var("_i".into())),
            rhs: Box::new(Expr::Var("n".into())),
        };
        let cmp_i_m = Expr::Cmp {
            kind: CmpKind::Lt,
            lhs: Box::new(Expr::Var("_i".into())),
            rhs: Box::new(Expr::Var("m".into())),
        };

        let float_0 = || Expr::Literal(Constant::Float(0.0));
        let float_1 = || Expr::Literal(Constant::Float(1.0));
        let body_work = || Stmt::Expr(Expr::Var("work".into()));
        let i_add_1 = || Stmt::CompoundAssign {
            target: Expr::Var("_i".into()),
            op: BinOp::Add,
            value: float_1(),
        };

        let mut body = vec![
            Stmt::VarDecl {
                name: "_i".into(),
                ty: Some(crate::ir::ty::Type::Float(64)),
                init: Some(float_0()),
                mutable: true,
            },
            Stmt::While {
                cond: cmp_i_n,
                body: vec![body_work(), i_add_1()],
            },
            Stmt::Assign {
                target: Expr::Var("_i".into()),
                value: float_0(),
            },
            Stmt::While {
                cond: cmp_i_m,
                body: vec![body_work(), i_add_1()],
            },
        ];

        promote_while_to_for(&mut body);

        assert!(
            matches!(&body[0], Stmt::VarDecl { name, mutable: true, init: None, .. } if name == "_i"),
            "VarDecl for _i must stay at function scope with no init: {body:?}"
        );
        assert!(
            matches!(&body[1], Stmt::For { init, .. } if matches!(
                init.first(),
                Some(Stmt::Assign { target: Expr::Var(n), .. }) if n == "_i"
            )),
            "First for-loop init must be an Assign, not a VarDecl: {body:?}"
        );
    }
}
