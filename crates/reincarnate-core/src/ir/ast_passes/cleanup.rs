//! Cleanup passes: dead code removal, self-assign elimination, stub removal,
//! empty-then inversion, unreachable code elimination, identical branch hoisting.

use super::super::ast::{Expr, Stmt};
use super::super::value::Constant;
use super::{
    body_always_exits, count_var_reads_in_stmt, expr_has_side_effects, negate_expr,
    stmt_references_var,
};

// ---------------------------------------------------------------------------
// Identical branch-assign hoisting
// ---------------------------------------------------------------------------

/// Hoist identical terminal assigns out of if/else branches.
///
/// Matches:
/// ```text
/// if (cond) { ...stmts; x = E; } else { ...stmts; x = E; }
/// ```
/// and rewrites to:
/// ```text
/// if (cond) { ...stmts; } else { ...stmts; }
/// x = E;
/// ```
///
/// When the resulting else branch is empty, it is cleared (the emitter will
/// omit it). When *both* branches become empty, the if is replaced by just
/// the condition as an expression statement (preserving any side effects),
/// or removed entirely if the condition is pure.
///
/// This enables `merge_decl_init` and `fold_single_use_consts` to later
/// convert the hoisted assign into an inlineable `const`.
pub fn fold_identical_branch_assigns(body: &mut Vec<Stmt>) {
    // Process at this level first.
    let mut i = 0;
    while i < body.len() {
        // Recurse into nested bodies first.
        match &mut body[i] {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                fold_identical_branch_assigns(then_body);
                fold_identical_branch_assigns(else_body);
            }
            Stmt::While { body: inner, .. }
            | Stmt::Loop { body: inner }
            | Stmt::ForOf { body: inner, .. } => {
                fold_identical_branch_assigns(inner);
            }
            Stmt::For {
                init,
                update,
                body: inner,
                ..
            } => {
                fold_identical_branch_assigns(init);
                fold_identical_branch_assigns(update);
                fold_identical_branch_assigns(inner);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    fold_identical_branch_assigns(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    fold_identical_branch_assigns(case_body);
                }
                fold_identical_branch_assigns(default_body);
            }
            _ => {}
        }

        // Now try to hoist from this if/else.
        let hoisted = match &mut body[i] {
            Stmt::If {
                then_body,
                else_body,
                ..
            } if !then_body.is_empty() && !else_body.is_empty() => {
                match (then_body.last(), else_body.last()) {
                    (
                        Some(Stmt::Assign {
                            target: Expr::Var(t1),
                            value: v1,
                        }),
                        Some(Stmt::Assign {
                            target: Expr::Var(t2),
                            value: v2,
                        }),
                    ) if t1 == t2 && v1 == v2 => {
                        // Both branches end with the same assign -- hoist it.
                        let assign = then_body.pop().unwrap();
                        else_body.pop();
                        Some(assign)
                    }
                    _ => None,
                }
            }
            _ => None,
        };

        if let Some(assign) = hoisted {
            // Clean up: if both branches are now empty, remove the if entirely
            // (or keep the condition as an expr if it has side effects).
            let remove_if = match &body[i] {
                Stmt::If {
                    then_body,
                    else_body,
                    ..
                } => then_body.is_empty() && else_body.is_empty(),
                _ => false,
            };

            if remove_if {
                let cond = match body.remove(i) {
                    Stmt::If { cond, .. } => cond,
                    _ => unreachable!(),
                };
                if expr_has_side_effects(&cond) {
                    body.insert(i, Stmt::Expr(cond));
                    body.insert(i + 1, assign);
                } else {
                    body.insert(i, assign);
                }
            } else {
                body.insert(i + 1, assign);
                i += 1; // skip past the if
            }
        }

        i += 1;
    }
}

// ---------------------------------------------------------------------------
// Forwarding stub elimination
// ---------------------------------------------------------------------------

/// Remove `let vN: T; vM = vN;` stubs where `vN` is uninit and has no other refs.
///
/// These are structurizer artifacts from empty else-branches: an uninit phi
/// variable is immediately forwarded to another phi. Both the decl and the
/// forwarding assign are meaningless. Recurses into nested bodies.
pub fn eliminate_forwarding_stubs(body: &mut Vec<Stmt>) {
    loop {
        if !try_eliminate_one_stub(body) {
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
                eliminate_forwarding_stubs(then_body);
                eliminate_forwarding_stubs(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                eliminate_forwarding_stubs(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                eliminate_forwarding_stubs(init);
                eliminate_forwarding_stubs(update);
                eliminate_forwarding_stubs(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    eliminate_forwarding_stubs(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    eliminate_forwarding_stubs(case_body);
                }
                eliminate_forwarding_stubs(default_body);
            }
            _ => {}
        }
    }
}

fn try_eliminate_one_stub(body: &mut Vec<Stmt>) -> bool {
    for i in 0..body.len().saturating_sub(1) {
        // Match: let vN: T; (uninit, mutable)
        let name = match &body[i] {
            Stmt::VarDecl {
                name,
                init: None,
                mutable: true,
                ..
            } => name.clone(),
            _ => continue,
        };

        // Next statement must be: vM = vN;
        let is_forwarding = matches!(
            &body[i + 1],
            Stmt::Assign { value: Expr::Var(v), .. } if v == &name
        );
        if !is_forwarding {
            continue;
        }

        // vN must have no other references in the body (only the forwarding assign).
        let other_refs: usize = body[i + 2..]
            .iter()
            .map(|s| count_var_reads_in_stmt(s, &name))
            .sum();
        if other_refs != 0 {
            continue;
        }

        // Remove both the decl and the forwarding assign.
        body.remove(i + 1);
        body.remove(i);
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Self-assignment elimination
// ---------------------------------------------------------------------------

/// Remove no-op self-assignments (`x = x;`) produced by out-of-SSA coalescing.
///
/// When multiple SSA values share a name, pass-through branches emit `x = x`
/// which is a no-op. This runs BEFORE ternary detection so that self-assigns
/// don't bloat if/else branches and prevent ternary pattern matching.
pub fn eliminate_self_assigns(body: &mut Vec<Stmt>) {
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                eliminate_self_assigns(then_body);
                eliminate_self_assigns(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                eliminate_self_assigns(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                eliminate_self_assigns(init);
                eliminate_self_assigns(update);
                eliminate_self_assigns(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    eliminate_self_assigns(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    eliminate_self_assigns(case_body);
                }
                eliminate_self_assigns(default_body);
            }
            _ => {}
        }
    }
    body.retain(|stmt| !is_self_assign(stmt));

    // Fix empty-then-body if/else by flipping condition.
    // After self-assign removal, some if-bodies become empty while their
    // else-body is non-empty: `if (c) {} else { body }` -> `if (!c) { body }`.
    for stmt in body.iter_mut() {
        if let Stmt::If {
            cond,
            then_body,
            else_body,
        } = stmt
        {
            if then_body.is_empty() && !else_body.is_empty() {
                // Replace cond with a placeholder, negate it, and put it back.
                let old_cond = std::mem::replace(cond, Expr::Literal(Constant::Bool(false)));
                *cond = negate_expr(old_cond);
                std::mem::swap(then_body, else_body);
            }
        }
    }

    // Remove fully-empty if/else statements (both branches empty after cleanup).
    body.retain(|stmt| {
        !matches!(stmt, Stmt::If { then_body, else_body, .. }
            if then_body.is_empty() && else_body.is_empty())
    });
}

fn is_self_assign(stmt: &Stmt) -> bool {
    matches!(stmt, Stmt::Assign { target: Expr::Var(t), value: Expr::Var(v) } if t == v)
}

/// Remove consecutive duplicate assignments (`x = a; x = a;` -> `x = a;`).
///
/// Structurizer duplicate edges emit the same phi-assignment in both arms of a
/// diamond that converge to the same merge block. After ternary/other rewrites
/// these can end up adjacent. Recurses into nested bodies.
pub fn eliminate_duplicate_assigns(body: &mut Vec<Stmt>) {
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                eliminate_duplicate_assigns(then_body);
                eliminate_duplicate_assigns(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                eliminate_duplicate_assigns(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                eliminate_duplicate_assigns(init);
                eliminate_duplicate_assigns(update);
                eliminate_duplicate_assigns(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    eliminate_duplicate_assigns(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    eliminate_duplicate_assigns(case_body);
                }
                eliminate_duplicate_assigns(default_body);
            }
            _ => {}
        }
    }

    // Remove consecutive duplicate assigns.
    let mut i = 0;
    while i + 1 < body.len() {
        let is_dup = match (&body[i], &body[i + 1]) {
            (
                Stmt::Assign {
                    target: t1,
                    value: v1,
                },
                Stmt::Assign {
                    target: t2,
                    value: v2,
                },
            ) => t1 == t2 && v1 == v2,
            _ => false,
        };
        if is_dup {
            body.remove(i + 1);
        } else {
            i += 1;
        }
    }
}

// ---------------------------------------------------------------------------
// Invert empty-then if-blocks
// ---------------------------------------------------------------------------

/// Invert `if (x) {} else { ... }` to `if (!x) { ... }`.
///
/// When the then-body is empty and the else-body is non-empty, negates the
/// condition and swaps the bodies. Recurses into all nested statement bodies.
pub fn invert_empty_then(body: &mut [Stmt]) {
    for stmt in body.iter_mut() {
        // Recurse first (children before parent).
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                invert_empty_then(then_body);
                invert_empty_then(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                invert_empty_then(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                invert_empty_then(init);
                invert_empty_then(update);
                invert_empty_then(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    invert_empty_then(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    invert_empty_then(case_body);
                }
                invert_empty_then(default_body);
            }
            _ => {}
        }

        // Then try to invert this statement.
        if let Stmt::If {
            cond,
            then_body,
            else_body,
        } = stmt
        {
            if then_body.is_empty() && !else_body.is_empty() {
                let old_cond = std::mem::replace(cond, Expr::Literal(Constant::Null));
                *cond = negate_expr(old_cond);
                *then_body = std::mem::take(else_body);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Eliminate unreachable code after exits
// ---------------------------------------------------------------------------

/// Remove statements after unconditional exits (return, break, continue,
/// or if/else where both branches always exit).
///
/// Recurses into nested bodies first (children before parent), then scans
/// the current body and truncates after the first unconditionally-exiting
/// statement.
pub fn eliminate_unreachable_after_exit(body: &mut Vec<Stmt>) {
    // Recurse into children first.
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                eliminate_unreachable_after_exit(then_body);
                eliminate_unreachable_after_exit(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                eliminate_unreachable_after_exit(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                eliminate_unreachable_after_exit(init);
                eliminate_unreachable_after_exit(update);
                eliminate_unreachable_after_exit(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    eliminate_unreachable_after_exit(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    eliminate_unreachable_after_exit(case_body);
                }
                eliminate_unreachable_after_exit(default_body);
            }
            _ => {}
        }
    }

    // Scan for unconditional exits and truncate.
    for i in 0..body.len() {
        let exits = match &body[i] {
            Stmt::Return(_) | Stmt::Break | Stmt::Continue | Stmt::LabeledBreak { .. } => true,
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                !else_body.is_empty()
                    && body_always_exits(then_body)
                    && body_always_exits(else_body)
            }
            _ => false,
        };
        if exits && i + 1 < body.len() {
            body.truncate(i + 1);
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Absorb split-path phi conditions
// ---------------------------------------------------------------------------

/// Absorb split-path phi conditions into their assigning branch.
///
/// Matches:
/// ```text
/// let vN: T;
/// ...
/// if (C) { ...; vN = E; } else { B; }
/// if (vN) { D; }
/// ```
/// and rewrites to:
/// ```text
/// ...
/// if (C) { ...; if (E) { D; } } else { B; }
/// ```
///
/// When the `if (vN)` has an else body and the then-body always exits,
/// the else body is pulled out as the continuation after the outer if.
///
/// This eliminates split-path phi booleans that are assigned in one
/// branch and left undefined (= false) on fallthrough paths.
pub fn absorb_phi_condition(body: &mut Vec<Stmt>) {
    loop {
        if !try_absorb_phi_condition(body) {
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
                absorb_phi_condition(then_body);
                absorb_phi_condition(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                absorb_phi_condition(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                absorb_phi_condition(init);
                absorb_phi_condition(update);
                absorb_phi_condition(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    absorb_phi_condition(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    absorb_phi_condition(case_body);
                }
                absorb_phi_condition(default_body);
            }
            _ => {}
        }
    }
}

fn try_absorb_phi_condition(body: &mut Vec<Stmt>) -> bool {
    use super::{count_var_reads_in_expr, substitute_var_in_expr};

    for i in 0..body.len().saturating_sub(1) {
        // Match: if (C) { ...; vN = E; } else { B; }
        let var_name = match &body[i] {
            Stmt::If { then_body, .. } => match then_body.last() {
                Some(Stmt::Assign {
                    target: Expr::Var(name),
                    ..
                }) => name.clone(),
                _ => continue,
            },
            _ => continue,
        };

        // Must have a corresponding uninit decl earlier in this scope.
        let decl_idx = (0..i).rev().find(|&j| {
            matches!(
                &body[j],
                Stmt::VarDecl { name, init: None, .. } if name == &var_name
            )
        });
        let Some(decl_idx) = decl_idx else {
            continue;
        };

        // vN must not appear between the decl and the outer if (ensures it's
        // a dedicated phi variable, not a general-purpose variable with earlier
        // assignments like `dodged = 1.0; ... if (x) { dodged = 4.0; }`).
        // Use stmt_references_var (reads AND writes) -- a bare write like
        // `vN = initialValue;` between decl and if means vN carries a value
        // on the else path that absorption would discard.
        let has_refs_before = body[decl_idx + 1..i]
            .iter()
            .any(|s| stmt_references_var(s, &var_name));
        if has_refs_before {
            continue;
        }

        // vN must not appear in the else_body at all (reads or writes).
        // Use stmt_references_var which checks both, unlike
        // count_var_reads_in_stmt which only counts reads.
        let else_has_refs = match &body[i] {
            Stmt::If { else_body, .. } => {
                else_body.iter().any(|s| stmt_references_var(s, &var_name))
            }
            _ => unreachable!(),
        };
        if else_has_refs {
            continue;
        }

        // Next statement must be if (expr_with_vN) { D } [else { D2 }].
        let (use_cond, use_then, use_else) = match &body[i + 1] {
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => (cond, then_body, else_body),
            _ => continue,
        };

        // use_cond must reference vN exactly once.
        if count_var_reads_in_expr(use_cond, &var_name) != 1 {
            continue;
        }

        // No refs to vN anywhere after body[i+1] (reads or writes).
        let has_refs_after = body[i + 2..]
            .iter()
            .any(|s| stmt_references_var(s, &var_name));
        if has_refs_after {
            continue;
        }

        // Case A: use_else is empty -- simple absorption.
        // Case B: use_else non-empty, use_then always exits -- pull else out.
        // Case C: use_else non-empty, neither exits -- duplicate else into outer else.

        // Clone values before mutating.
        let assign_value = match &body[i] {
            Stmt::If { then_body, .. } => match then_body.last() {
                Some(Stmt::Assign { value, .. }) => value.clone(),
                _ => unreachable!(),
            },
            _ => unreachable!(),
        };
        let mut new_cond = use_cond.clone();
        let use_then = use_then.clone();
        let use_else = use_else.clone();

        // Substitute E for vN in the condition.
        let mut replacement = Some(assign_value);
        substitute_var_in_expr(&mut new_cond, &var_name, &mut replacement);

        let is_case_c = !use_else.is_empty() && !body_always_exits(&use_then);

        // Build the merged if.
        // Case A/B: no else on the merged if (Case B's else becomes continuation).
        // Case C: keep the full else -- the then-branch gets `if (E) { D } else { F }`.
        let merged_if = Stmt::If {
            cond: new_cond,
            then_body: use_then,
            else_body: if is_case_c { use_else.clone() } else { vec![] },
        };

        // Modify outer if's then_body: remove vN = E, append merged_if.
        if let Stmt::If {
            then_body,
            else_body,
            ..
        } = &mut body[i]
        {
            then_body.pop();
            then_body.push(merged_if);

            // Case C: append use_else to outer else_body too -- when vN is
            // unassigned (falsy/undefined), the use-site if always takes
            // the else path.
            if is_case_c {
                else_body.extend(use_else.clone());
            }
        }

        // Remove the if(vN) statement.
        body.remove(i + 1);

        // Case B: insert use_else as continuation after body[i].
        if !use_else.is_empty() && !is_case_c {
            for (j, stmt) in use_else.into_iter().enumerate() {
                body.insert(i + 1 + j, stmt);
            }
        }

        // Remove the uninit decl.
        body.remove(decl_idx);

        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::ast::{Expr, Stmt};
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

    fn uninit_decl(name: &str) -> Stmt {
        Stmt::VarDecl {
            name: name.to_string(),
            ty: Some(crate::ir::ty::Type::Int(64)),
            init: None,
            mutable: true,
        }
    }

    fn empty_array() -> Expr {
        Expr::ArrayInit(vec![])
    }

    fn side_effect_call() -> Stmt {
        Stmt::Expr(Expr::Call {
            func: "side_effect".to_string(),
            args: vec![],
        })
    }

    fn br_call() -> Expr {
        Expr::Call {
            func: "br".to_string(),
            args: vec![],
        }
    }

    // Regression: 2a0a15a -- eliminate_duplicate_assigns removes consecutive
    // identical assignments (structurizer artifacts from duplicate edges).
    #[test]
    fn eliminate_duplicate_assigns_basic() {
        let mut body = vec![assign(var("x"), var("a")), assign(var("x"), var("a"))];

        eliminate_duplicate_assigns(&mut body);

        assert_eq!(
            body.len(),
            1,
            "Expected single assign after dedup: {body:?}"
        );
        assert!(matches!(&body[0], Stmt::Assign { target, value }
            if *target == var("x") && *value == var("a")));
    }

    // Regression: 6aa3e9f -- eliminate_forwarding_stubs removes uninit decl +
    // forwarding assign patterns from empty else-branches.
    #[test]
    fn eliminate_forwarding_stub() {
        // let v1: i64; v2 = v1;  (v1 is uninit and has no other refs)
        // Both the decl and the forwarding assign should be removed.
        let mut body = vec![uninit_decl("v1"), assign(var("v2"), var("v1"))];

        eliminate_forwarding_stubs(&mut body);

        assert!(
            body.is_empty(),
            "Expected empty body after forwarding stub elimination: {body:?}"
        );
    }

    // Regression: 46e4a73 -- after self-assign elimination empties the then-branch,
    // the if-statement should flip: `if (c) {} else { B }` -> `if (!c) { B }`.
    #[test]
    fn flip_empty_then_after_self_assign() {
        let mut body = vec![Stmt::If {
            cond: var("c"),
            then_body: vec![assign(var("x"), var("x"))],
            else_body: vec![assign(var("x"), int(1))],
        }];

        eliminate_self_assigns(&mut body);

        assert_eq!(body.len(), 1);
        match &body[0] {
            Stmt::If {
                cond,
                then_body,
                else_body,
            } => {
                assert!(
                    matches!(cond, Expr::Not(_)),
                    "Expected negated condition, got: {cond:?}"
                );
                assert_eq!(then_body.len(), 1);
                assert!(matches!(&then_body[0], Stmt::Assign { target, value }
                    if *target == var("x") && *value == int(1)));
                assert!(else_body.is_empty());
            }
            other => panic!("Expected If, got: {other:?}"),
        }
    }

    // Regression: 46e4a73 -- when self-assign elimination empties both branches,
    // the entire if-statement is removed.
    #[test]
    fn remove_fully_empty_if() {
        let mut body = vec![Stmt::If {
            cond: var("c"),
            then_body: vec![assign(var("x"), var("x"))],
            else_body: vec![assign(var("y"), var("y"))],
        }];

        eliminate_self_assigns(&mut body);

        assert!(
            body.is_empty(),
            "Expected empty body after fully-empty if removal: {body:?}"
        );
    }

    #[test]
    fn fold_identical_branch_assigns_hoists_empty_array() {
        let mut body = vec![
            Stmt::VarDecl {
                name: "v0".into(),
                ty: None,
                init: None,
                mutable: true,
            },
            Stmt::If {
                cond: var("cond"),
                then_body: vec![side_effect_call(), assign(var("v0"), empty_array())],
                else_body: vec![assign(var("v0"), empty_array())],
            },
            Stmt::Return(Some(Expr::ArrayInit(vec![var("v0")]))),
        ];

        fold_identical_branch_assigns(&mut body);

        assert_eq!(body.len(), 4);
        assert!(matches!(&body[0], Stmt::VarDecl { name, init: None, .. } if name == "v0"));
        assert!(matches!(&body[1], Stmt::If { then_body, else_body, .. }
            if then_body.len() == 1 && else_body.is_empty()));
        assert!(
            matches!(&body[2], Stmt::Assign { target: Expr::Var(n), value }
            if n == "v0" && *value == empty_array())
        );
    }

    #[test]
    fn fold_identical_branch_assigns_removes_empty_if() {
        let mut body = vec![Stmt::If {
            cond: int(1),
            then_body: vec![assign(var("v0"), empty_array())],
            else_body: vec![assign(var("v0"), empty_array())],
        }];

        fold_identical_branch_assigns(&mut body);

        assert_eq!(body.len(), 1);
        assert!(matches!(&body[0], Stmt::Assign { target: Expr::Var(n), .. } if n == "v0"));
    }

    #[test]
    fn fold_identical_branch_then_inline_full_pipeline() {
        use super::super::{fold_single_use_consts, merge_decl_init, narrow_var_scope};

        let mut body = vec![
            Stmt::VarDecl {
                name: "v0".into(),
                ty: None,
                init: None,
                mutable: true,
            },
            Stmt::VarDecl {
                name: "v12".into(),
                ty: None,
                init: None,
                mutable: true,
            },
            Stmt::If {
                cond: var("c1"),
                then_body: vec![side_effect_call(), assign(var("v0"), empty_array())],
                else_body: vec![assign(var("v0"), empty_array())],
            },
            Stmt::VarDecl {
                name: "v11".into(),
                ty: None,
                init: Some(br_call()),
                mutable: false,
            },
            Stmt::If {
                cond: var("c2"),
                then_body: vec![side_effect_call(), assign(var("v12"), empty_array())],
                else_body: vec![assign(var("v12"), empty_array())],
            },
            Stmt::Return(Some(Expr::ArrayInit(vec![
                var("v0"),
                var("v11"),
                var("v12"),
            ]))),
        ];

        fold_identical_branch_assigns(&mut body);
        narrow_var_scope(&mut body);
        merge_decl_init(&mut body);
        fold_single_use_consts(&mut body);

        let ret_stmt = body.last().unwrap();
        match ret_stmt {
            Stmt::Return(Some(Expr::ArrayInit(elems))) => {
                assert_eq!(
                    elems[0],
                    empty_array(),
                    "v0 should have been inlined to []: {body:?}"
                );
                assert_eq!(
                    elems[2],
                    empty_array(),
                    "v12 should have been inlined to []: {body:?}"
                );
            }
            _ => panic!("Expected return with array: {body:?}"),
        }
    }

    #[test]
    fn absorb_phi_skips_else_branch_write() {
        let mut body = vec![
            Stmt::VarDecl {
                name: "v620".to_string(),
                ty: None,
                init: None,
                mutable: true,
            },
            Stmt::If {
                cond: var("cond1"),
                then_body: vec![assign(var("v620"), var("expr1"))],
                else_body: vec![assign(var("v620"), var("expr2"))],
            },
            Stmt::If {
                cond: var("v620"),
                then_body: vec![Stmt::Expr(var("action"))],
                else_body: vec![],
            },
        ];

        absorb_phi_condition(&mut body);

        assert_eq!(
            body.len(),
            3,
            "absorb_phi_condition incorrectly absorbed a phi with an else-branch write: {body:?}"
        );
        assert!(matches!(&body[0], Stmt::VarDecl { name, .. } if name == "v620"));
        assert!(matches!(&body[1], Stmt::If { .. }));
        assert!(matches!(&body[2], Stmt::If { cond, .. } if *cond == var("v620")));
    }
}
