//! Variable-related passes: scope narrowing, decl/init merging, const folding,
//! forward substitution, and order-preserving single-use inlining.

use super::super::ast::{Expr, Stmt};
use super::{
    count_unconditional_reads, count_var_reads_in_stmt, expr_has_any_var_ref,
    expr_has_side_effects, expr_references_var, find_unique_child_body, is_stable_path,
    precompute_stmt_ref_counts, precompute_var_read_counts, remove_dead_var_decl,
    stmt_assigns_to_prefix_of, stmt_has_side_effects, stmt_references_var, substitute_var_in_stmt,
    var_is_reassigned,
};

// ---------------------------------------------------------------------------
// Single-use const folding
// ---------------------------------------------------------------------------

/// Fold single-use `const x = expr; ... use(x) ...` into `... use(expr) ...`.
///
/// This is AST-level copy propagation: if an immutable variable is assigned
/// once and referenced exactly once, substitute the init expression at the
/// use site and remove the declaration.
///
/// Runs iteratively until fixpoint. Recurses into nested bodies.
///
/// Safety rules:
/// - Adjacent use (next statement): always fold (pure or impure init).
/// - Non-adjacent use: only fold when all intervening statements are free
///   of side effects (pure VarDecls, uninit decls).
pub fn fold_single_use_consts(body: &mut Vec<Stmt>) {
    // Fold at this level first -- batch pass with precomputed counts.
    loop {
        if !try_fold_batch(body) {
            break;
        }
    }
    // Then recurse into nested bodies.
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                fold_single_use_consts(then_body);
                fold_single_use_consts(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                fold_single_use_consts(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                fold_single_use_consts(init);
                fold_single_use_consts(update);
                fold_single_use_consts(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    fold_single_use_consts(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    fold_single_use_consts(case_body);
                }
                fold_single_use_consts(default_body);
            }
            _ => {}
        }
    }
}

/// Batch version: precompute all variable read counts once, then process all
/// VarDecl candidates in a single forward pass. O(n) per call instead of O(n^2).
fn try_fold_batch(body: &mut Vec<Stmt>) -> bool {
    if body.is_empty() {
        return false;
    }

    // Phase 1: count all variable reads in one pass.
    let counts = precompute_var_read_counts(body);

    // Phase 2: process candidates in forward order.
    let mut changed = false;
    let mut i = 0;
    while i < body.len() {
        // Dead uninit decl: `let vN: T;` with no remaining references.
        if let Stmt::VarDecl {
            name, init: None, ..
        } = &body[i]
        {
            // The precomputed count includes the VarDecl name itself (1).
            // refs_after_decl = total - 1.
            let total = counts.get(name.as_str()).copied().unwrap_or(0);
            if total <= 1 {
                remove_dead_var_decl(body, i);
                changed = true;
                continue;
            }
        }

        let name = match &body[i] {
            Stmt::VarDecl {
                name,
                init: Some(_),
                ..
            } => name.clone(),
            _ => {
                i += 1;
                continue;
            }
        };

        let total = counts.get(name.as_str()).copied().unwrap_or(0);
        let refs_after = total.saturating_sub(1);

        // Dead declaration: zero reads after the decl.
        if refs_after == 0 {
            remove_dead_var_decl(body, i);
            changed = true;
            continue;
        }

        if refs_after != 1 {
            i += 1;
            continue;
        }

        // Don't fold if the variable is reassigned.
        if var_is_reassigned(&body[i + 1..], &name) {
            i += 1;
            continue;
        }

        // Find the statement containing the single use.
        let use_idx =
            match (i + 1..body.len()).find(|&j| count_var_reads_in_stmt(&body[j], &name) > 0) {
                Some(idx) => idx,
                None => {
                    i += 1;
                    continue;
                }
            };

        let adjacent = use_idx == i + 1;

        if !adjacent {
            let all_intervening_pure = body[i + 1..use_idx]
                .iter()
                .all(|s| !stmt_has_side_effects(s));
            if !all_intervening_pure {
                let init = match &body[i] {
                    Stmt::VarDecl {
                        init: Some(init), ..
                    } => init,
                    _ => unreachable!(),
                };
                // Pure constant with no variable references (e.g. `[]`, `0`,
                // `"str"`) can be sunk past any statement safely --
                // evaluation order doesn't matter.
                let is_trivial_const = !expr_has_side_effects(init) && !expr_has_any_var_ref(init);
                if !is_trivial_const {
                    let can_sink_path = is_stable_path(init)
                        && body[i + 1..use_idx]
                            .iter()
                            .all(|s| !stmt_assigns_to_prefix_of(s, init));
                    let can_sink_past_locals = !can_sink_path
                        && body[i + 1..use_idx].iter().all(|s| match s {
                            Stmt::Assign {
                                target: Expr::Var(t),
                                ..
                            } => !expr_references_var(init, t),
                            Stmt::VarDecl { name: n, .. } => !expr_references_var(init, n),
                            Stmt::Expr(_) => true,
                            _ => false,
                        });
                    if !can_sink_path && !can_sink_past_locals {
                        i += 1;
                        continue;
                    }
                }
            }
        }

        // Extract init and substitute at the use site.
        let init_expr = match body.remove(i) {
            Stmt::VarDecl {
                init: Some(expr), ..
            } => expr,
            _ => unreachable!(),
        };

        // use_idx shifted left by 1 after removal.
        let mut replacement = Some(init_expr);
        substitute_var_in_stmt(&mut body[use_idx - 1], &name, &mut replacement);
        changed = true;
        // Don't increment i -- the next statement shifted down.
    }

    changed
}

// ---------------------------------------------------------------------------
// Order-preserving inline of single-use variables
// ---------------------------------------------------------------------------

/// Inline single-use `VarDecl`s at their use sites in declaration order.
///
/// Unlike `fold_single_use_consts` which sinks expressions past intervening
/// statements (and must refuse when that would reorder calls), this pass
/// substitutes variables in forward (declaration) order -- preserving
/// relative call order by construction.
///
/// A variable qualifies if:
/// 1. It has exactly one read after the declaration.
/// 2. That read is in an **unconditional** position (not inside an
///    if/loop body) -- otherwise an unconditional call would become
///    conditional.
/// 3. It is not reassigned anywhere after the declaration.
pub fn inline_ordered_single_use(body: &mut Vec<Stmt>) {
    try_inline_ordered(body);

    for stmt in body.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                inline_ordered_single_use(then_body);
                inline_ordered_single_use(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                inline_ordered_single_use(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                inline_ordered_single_use(init);
                inline_ordered_single_use(update);
                inline_ordered_single_use(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    inline_ordered_single_use(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    inline_ordered_single_use(case_body);
                }
                inline_ordered_single_use(default_body);
            }
            _ => {}
        }
    }
}

fn try_inline_ordered(body: &mut Vec<Stmt>) {
    // Fixpoint: each iteration may inline the bottom-most eligible decl,
    // removing a barrier and unblocking the decl above it on the next pass.
    loop {
        if body.len() < 2 {
            return;
        }

        let counts = precompute_var_read_counts(body);
        let mut did_change = false;

        let mut i = 0;
        while i < body.len() {
            let (name, init_has_effects) = match &body[i] {
                Stmt::VarDecl {
                    name,
                    init: Some(e),
                    ..
                } => (name.clone(), expr_has_side_effects(e)),
                _ => {
                    i += 1;
                    continue;
                }
            };

            // Total reads includes the VarDecl name itself (1).
            // We need exactly 1 additional read.
            let total = counts.get(name.as_str()).copied().unwrap_or(0);
            if total != 2 {
                i += 1;
                continue;
            }

            // Find the statement containing the single use.
            let use_idx =
                match (i + 1..body.len()).find(|&j| count_var_reads_in_stmt(&body[j], &name) > 0) {
                    Some(idx) => idx,
                    None => {
                        i += 1;
                        continue;
                    }
                };

            // The read must be in an unconditional position within that
            // statement -- not inside an if/loop body where it would make
            // an unconditional call conditional.
            if count_unconditional_reads(&body[use_idx], &name) != 1 {
                i += 1;
                continue;
            }

            if var_is_reassigned(&body[i + 1..], &name) {
                i += 1;
                continue;
            }

            // If the init contains calls, don't move it past any statement
            // that also contains calls -- that would rearrange call order.
            if init_has_effects && (i + 1..use_idx).any(|j| stmt_has_side_effects(&body[j])) {
                i += 1;
                continue;
            }

            // Inline: remove VarDecl, substitute at use site.
            let init_expr = match body.remove(i) {
                Stmt::VarDecl {
                    init: Some(expr), ..
                } => expr,
                _ => unreachable!(),
            };

            let mut replacement = Some(init_expr);
            substitute_var_in_stmt(&mut body[use_idx - 1], &name, &mut replacement);
            did_change = true;
            // Don't increment i -- next statement shifted down.
        }

        if !did_change {
            break;
        }
    }
}

// ---------------------------------------------------------------------------
// Forward substitution
// ---------------------------------------------------------------------------

/// Forward-substitute single-use assigns into adjacent use sites.
///
/// For each `x = E;` at position i, if `x` appears exactly once in the
/// remaining body and that use is at position i+1 (adjacent), substitute E
/// directly into the use site and remove the assign. Also removes dead
/// assigns (zero refs remaining) -- keeping E as a bare expression statement
/// if it has side effects.
///
/// Recurses into nested bodies.
pub fn forward_substitute(body: &mut Vec<Stmt>) {
    // Substitute at this level first.
    loop {
        if !try_forward_substitute_one(body) {
            break;
        }
    }
    // Then recurse into nested bodies.
    for stmt in body.iter_mut() {
        match stmt {
            Stmt::If {
                then_body,
                else_body,
                ..
            } => {
                forward_substitute(then_body);
                forward_substitute(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                forward_substitute(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                forward_substitute(init);
                forward_substitute(update);
                forward_substitute(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    forward_substitute(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    forward_substitute(case_body);
                }
                forward_substitute(default_body);
            }
            _ => {}
        }
    }
}

fn try_forward_substitute_one(body: &mut Vec<Stmt>) -> bool {
    for i in 0..body.len() {
        // Match: x = E; where x is a Var
        let name = match &body[i] {
            Stmt::Assign {
                target: Expr::Var(name),
                ..
            } => name.clone(),
            _ => continue,
        };

        // Don't substitute assignments to outer-scope variables -- removing
        // the assignment would lose the update visible to outer scopes.
        let is_local = body
            .iter()
            .any(|s| matches!(s, Stmt::VarDecl { name: n, .. } if n == &name));
        if !is_local {
            continue;
        }

        // Count ALL references in the remaining body at this scope level.
        let total_refs: usize = body[i + 1..]
            .iter()
            .map(|s| count_var_reads_in_stmt(s, &name))
            .sum();

        if total_refs != 1 {
            continue;
        }

        // Don't substitute if the variable is reassigned in the remaining body.
        if var_is_reassigned(&body[i + 1..], &name) {
            continue;
        }

        // Adjacent check: the single use (read) must be at position i+1.
        if i + 1 >= body.len() || count_var_reads_in_stmt(&body[i + 1], &name) == 0 {
            continue;
        }

        // Extract value and substitute at the use site.
        let value = match body.remove(i) {
            Stmt::Assign { value, .. } => value,
            _ => unreachable!(),
        };

        let mut replacement = Some(value);
        substitute_var_in_stmt(&mut body[i], &name, &mut replacement);
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Declaration/init merging
// ---------------------------------------------------------------------------

/// Merge uninitialized `let x: T;` declarations with their first assignment.
///
/// Rewrites:
/// ```text
/// let x: number;
/// ...            // no references to x
/// x = expr;
/// ```
/// into:
/// ```text
/// ...
/// let x: number = expr;
/// ```
///
/// Recurses into nested bodies after merging at the current level.
pub fn merge_decl_init(body: &mut Vec<Stmt>) {
    loop {
        if !try_merge_one_decl(body) {
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
                merge_decl_init(then_body);
                merge_decl_init(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                merge_decl_init(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                merge_decl_init(init);
                merge_decl_init(update);
                merge_decl_init(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    merge_decl_init(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    merge_decl_init(case_body);
                }
                merge_decl_init(default_body);
            }
            _ => {}
        }
    }
}

fn try_merge_one_decl(body: &mut Vec<Stmt>) -> bool {
    for i in 0..body.len() {
        let (name, ty) = match &body[i] {
            Stmt::VarDecl {
                name,
                ty,
                init: None,
                mutable: true,
            } => (name.clone(), ty.clone()),
            _ => continue,
        };

        // Find the first top-level statement after the decl that references this var.
        for j in (i + 1)..body.len() {
            if !stmt_references_var(&body[j], &name) {
                continue;
            }

            // First reference found. Is it a plain `name = value;`?
            let is_plain_assign = matches!(
                &body[j],
                Stmt::Assign { target: Expr::Var(tname), value }
                    if tname == &name && !expr_references_var(value, &name)
            );

            if !is_plain_assign {
                break; // first reference isn't a mergeable assign
            }

            // Safe to merge. Remove the uninit decl at i.
            body.remove(i);
            // The assign shifted left by 1.
            let assign_idx = j - 1;
            // Extract the value and replace with an initialized VarDecl.
            let value = match std::mem::replace(&mut body[assign_idx], Stmt::Break) {
                Stmt::Assign { value, .. } => value,
                _ => unreachable!(),
            };
            body[assign_idx] = Stmt::VarDecl {
                name,
                ty,
                init: Some(value),
                mutable: true,
            };
            return true;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Scope narrowing
// ---------------------------------------------------------------------------

/// Push uninitialized `let` declarations into the innermost scope that uses them.
///
/// When a `let vN: T;` at the current scope is referenced only inside a single
/// child scope body (one if-branch, one loop body, etc.), move the declaration
/// into that child body.
///
/// Recurses into nested scopes after narrowing.
pub fn narrow_var_scope(body: &mut Vec<Stmt>) {
    loop {
        if !try_narrow_batch(body) {
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
                narrow_var_scope(then_body);
                narrow_var_scope(else_body);
            }
            Stmt::While { body, .. } | Stmt::Loop { body } | Stmt::ForOf { body, .. } => {
                narrow_var_scope(body);
            }
            Stmt::For {
                init, update, body, ..
            } => {
                narrow_var_scope(init);
                narrow_var_scope(update);
                narrow_var_scope(body);
            }
            Stmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    narrow_var_scope(block_body);
                }
            }
            Stmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    narrow_var_scope(case_body);
                }
                narrow_var_scope(default_body);
            }
            _ => {}
        }
    }
}

fn try_narrow_batch(body: &mut Vec<Stmt>) -> bool {
    if body.is_empty() {
        return false;
    }

    let ref_counts = precompute_stmt_ref_counts(body);

    let mut changed = false;
    let mut i = 0;
    while i < body.len() {
        let (name, ty) = match &body[i] {
            Stmt::VarDecl {
                name,
                ty,
                init: None,
                mutable: true,
            } => (name.clone(), ty.clone()),
            _ => {
                i += 1;
                continue;
            }
        };

        let total = ref_counts.get(name.as_str()).copied().unwrap_or(0);
        if total != 2 {
            i += 1;
            continue;
        }

        let j = match (i + 1..body.len()).find(|&j| stmt_references_var(&body[j], &name)) {
            Some(j) => j,
            None => {
                i += 1;
                continue;
            }
        };

        if let Some(target_body) = find_unique_child_body(&mut body[j], &name) {
            target_body.insert(
                0,
                Stmt::VarDecl {
                    name,
                    ty,
                    init: None,
                    mutable: true,
                },
            );
            body.remove(i);
            changed = true;
            continue;
        }

        i += 1;
    }

    changed
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

    fn const_decl(name: &str, init: Expr) -> Stmt {
        Stmt::VarDecl {
            name: name.to_string(),
            ty: None,
            init: Some(init),
            mutable: false,
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

    fn str_lit(s: &str) -> Expr {
        Expr::Literal(Constant::String(s.to_string()))
    }

    // -----------------------------------------------------------------------
    // Decl/init merge tests
    // -----------------------------------------------------------------------

    #[test]
    fn merge_decl_basic() {
        let mut body = vec![uninit_decl("x"), assign(var("x"), int(5))];
        merge_decl_init(&mut body);
        assert_eq!(body.len(), 1);
        match &body[0] {
            Stmt::VarDecl {
                name,
                init,
                mutable,
                ..
            } => {
                assert_eq!(name, "x");
                assert_eq!(*init, Some(int(5)));
                assert!(*mutable);
            }
            other => panic!("Expected VarDecl, got: {other:?}"),
        }
    }

    #[test]
    fn merge_decl_with_gap() {
        let y_decl = Stmt::VarDecl {
            name: "y".to_string(),
            ty: None,
            init: Some(int(10)),
            mutable: false,
        };
        let mut body = vec![uninit_decl("x"), y_decl, assign(var("x"), int(5))];
        merge_decl_init(&mut body);
        assert_eq!(body.len(), 2);
        assert!(matches!(&body[0], Stmt::VarDecl { name, .. } if name == "y"));
        match &body[1] {
            Stmt::VarDecl { name, init, .. } => {
                assert_eq!(name, "x");
                assert_eq!(*init, Some(int(5)));
            }
            other => panic!("Expected VarDecl, got: {other:?}"),
        }
    }

    #[test]
    fn merge_decl_no_merge_first_ref_is_read() {
        let mut body = vec![
            uninit_decl("x"),
            assign(
                var("y"),
                Expr::Call {
                    func: "add_i32".to_string(),
                    args: vec![var("x"), int(1)],
                },
            ),
            assign(var("x"), int(5)),
        ];
        merge_decl_init(&mut body);
        assert_eq!(body.len(), 3);
        assert!(matches!(&body[0], Stmt::VarDecl { init: None, .. }));
    }

    #[test]
    fn merge_decl_no_merge_self_reference() {
        let mut body = vec![
            uninit_decl("x"),
            assign(
                var("x"),
                Expr::Call {
                    func: "add_i32".to_string(),
                    args: vec![var("x"), int(1)],
                },
            ),
        ];
        merge_decl_init(&mut body);
        assert_eq!(body.len(), 2);
        assert!(matches!(&body[0], Stmt::VarDecl { init: None, .. }));
    }

    #[test]
    fn merge_decl_no_merge_inside_if() {
        let mut body = vec![
            uninit_decl("x"),
            Stmt::If {
                cond: var("c"),
                then_body: vec![assign(var("x"), int(1))],
                else_body: vec![assign(var("x"), int(2))],
            },
        ];
        merge_decl_init(&mut body);
        assert_eq!(body.len(), 2);
        assert!(matches!(&body[0], Stmt::VarDecl { init: None, .. }));
    }

    #[test]
    fn merge_decl_ternary() {
        let mut body = vec![
            uninit_decl("x"),
            assign(
                var("x"),
                Expr::Ternary {
                    cond: Box::new(var("c")),
                    then_val: Box::new(var("a")),
                    else_val: Box::new(var("b")),
                },
            ),
        ];
        merge_decl_init(&mut body);
        assert_eq!(body.len(), 1);
        match &body[0] {
            Stmt::VarDecl { name, init, .. } => {
                assert_eq!(name, "x");
                assert!(matches!(init, Some(Expr::Ternary { .. })));
            }
            other => panic!("Expected VarDecl, got: {other:?}"),
        }
    }

    #[test]
    fn merge_decl_preserves_order() {
        let mut body = vec![
            uninit_decl("x"),
            uninit_decl("y"),
            assign(var("y"), int(5)),
            assign(
                var("x"),
                Expr::Call {
                    func: "add_i32".to_string(),
                    args: vec![var("y"), int(1)],
                },
            ),
        ];
        merge_decl_init(&mut body);
        assert_eq!(body.len(), 2);
        match &body[0] {
            Stmt::VarDecl { name, init, .. } => {
                assert_eq!(name, "y");
                assert_eq!(*init, Some(int(5)));
            }
            other => panic!("Expected VarDecl for y, got: {other:?}"),
        }
        match &body[1] {
            Stmt::VarDecl { name, init, .. } => {
                assert_eq!(name, "x");
                assert!(init.is_some());
            }
            other => panic!("Expected VarDecl for x, got: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Single-use const folding tests
    // -----------------------------------------------------------------------

    #[test]
    fn fold_adjacent_pure() {
        let logical_and = Expr::LogicalAnd {
            lhs: Box::new(var("a")),
            rhs: Box::new(var("b")),
        };
        let mut body = vec![
            const_decl("v17", logical_and.clone()),
            assign(
                var("x"),
                Expr::Ternary {
                    cond: Box::new(var("v17")),
                    then_val: Box::new(int(1)),
                    else_val: Box::new(int(2)),
                },
            ),
        ];
        fold_single_use_consts(&mut body);
        assert_eq!(body.len(), 1);
        match &body[0] {
            Stmt::Assign { value, .. } => match value {
                Expr::Ternary { cond, .. } => {
                    assert!(matches!(cond.as_ref(), Expr::LogicalAnd { .. }));
                }
                other => panic!("Expected Ternary, got: {other:?}"),
            },
            other => panic!("Expected Assign, got: {other:?}"),
        }
    }

    #[test]
    fn fold_adjacent_impure() {
        let call = Expr::Call {
            func: "f".to_string(),
            args: vec![],
        };
        let mut body = vec![
            const_decl("v", call),
            Stmt::If {
                cond: var("v"),
                then_body: vec![assign(var("x"), int(1))],
                else_body: vec![],
            },
        ];
        fold_single_use_consts(&mut body);
        assert_eq!(body.len(), 1);
        match &body[0] {
            Stmt::If { cond, .. } => {
                assert!(matches!(cond, Expr::Call { .. }));
            }
            other => panic!("Expected If, got: {other:?}"),
        }
    }

    #[test]
    fn fold_no_fold_multi_use() {
        let mut body = vec![
            const_decl(
                "v",
                Expr::Call {
                    func: "add_i32".to_string(),
                    args: vec![var("a"), var("b")],
                },
            ),
            assign(var("x"), var("v")),
            assign(var("y"), var("v")),
        ];
        fold_single_use_consts(&mut body);
        assert_eq!(body.len(), 3);
        assert!(matches!(&body[0], Stmt::VarDecl { .. }));
    }

    #[test]
    fn fold_cascading() {
        let mut body = vec![
            const_decl("a", int(1)),
            const_decl(
                "b",
                Expr::Call {
                    func: "add_i32".to_string(),
                    args: vec![var("a"), int(2)],
                },
            ),
            assign(var("x"), var("b")),
        ];
        fold_single_use_consts(&mut body);
        assert_eq!(body.len(), 1);
        assert!(matches!(
            &body[0],
            Stmt::Assign {
                value: Expr::Call { .. },
                ..
            }
        ));
    }

    #[test]
    fn fold_non_adjacent_pure_across_pure_decls() {
        let mut body = vec![
            const_decl(
                "a",
                Expr::Call {
                    func: "add_i32".to_string(),
                    args: vec![var("x"), int(1)],
                },
            ),
            const_decl(
                "b",
                Expr::Call {
                    func: "add_i32".to_string(),
                    args: vec![var("y"), int(2)],
                },
            ),
            assign(
                var("z"),
                Expr::Call {
                    func: "add_i32".to_string(),
                    args: vec![var("a"), var("b")],
                },
            ),
        ];
        fold_single_use_consts(&mut body);
        assert_eq!(body.len(), 1);
    }

    #[test]
    fn fold_single_use_mutable() {
        let mut body = vec![
            Stmt::VarDecl {
                name: "v".to_string(),
                ty: None,
                init: Some(int(1)),
                mutable: true,
            },
            assign(var("x"), var("v")),
        ];
        fold_single_use_consts(&mut body);
        assert_eq!(body.len(), 1);
        assert!(matches!(&body[0], Stmt::Assign { value, .. } if *value == int(1)));
    }

    #[test]
    fn fold_hp_pattern() {
        let mut body = vec![
            const_decl("v35", var("HP")),
            assign(
                var("HP"),
                Expr::Call {
                    func: "sub_i32".to_string(),
                    args: vec![var("v35"), var("v11")],
                },
            ),
        ];
        fold_single_use_consts(&mut body);
        assert_eq!(body.len(), 1);
        match &body[0] {
            Stmt::Assign { target, value } => {
                assert_eq!(*target, var("HP"));
                match value {
                    Expr::Call { func, args } => {
                        assert_eq!(func, "sub_i32");
                        assert_eq!(args[0], var("HP"));
                        assert_eq!(args[1], var("v11"));
                    }
                    other => panic!("Expected Call, got: {other:?}"),
                }
            }
            other => panic!("Expected Assign, got: {other:?}"),
        }
    }

    // -----------------------------------------------------------------------
    // Regression tests
    // -----------------------------------------------------------------------

    #[test]
    fn fold_const_skips_bare_write() {
        let mut body = vec![
            const_decl("x", var("a")),
            assign(var("x"), int(5)),
            assign(var("y"), var("x")),
        ];
        fold_single_use_consts(&mut body);
        assert_eq!(
            body.len(),
            3,
            "All three statements should be preserved: {body:?}"
        );
        assert!(matches!(&body[0], Stmt::VarDecl { name, init: Some(_), .. } if name == "x"));
    }

    #[test]
    fn dead_decl_removes_orphaned_assigns() {
        let mut body = vec![uninit_decl("x"), assign(var("x"), int(0))];
        fold_single_use_consts(&mut body);
        assert!(body.is_empty(), "Expected empty body: {body:?}");
    }

    #[test]
    fn forward_sub_preserves_outer_scope_assign() {
        let call = Expr::Call {
            func: "f".to_string(),
            args: vec![],
        };
        let mut body = vec![assign(var("x"), call.clone()), assign(var("y"), var("x"))];
        forward_substitute(&mut body);
        assert!(
            body.iter()
                .any(|s| matches!(s, Stmt::Assign { target, .. } if *target == var("x"))),
            "Expected outer-scope assign to x to be preserved: {body:?}"
        );
    }

    #[test]
    fn forward_sub_no_replace_assign_target() {
        let call = Expr::Call {
            func: "f".to_string(),
            args: vec![],
        };
        let mut body = vec![const_decl("v", call), assign(var("v"), int(5))];
        forward_substitute(&mut body);
        let has_var_target = body.iter().any(|s| match s {
            Stmt::Assign { target, value } => *target == var("v") && *value == int(5),
            _ => false,
        });
        assert!(has_var_target, "Expected `v = 5` with Var target: {body:?}");
    }

    #[test]
    fn dead_decl_pure_removed() {
        let mut body = vec![const_decl("v", int(42))];
        fold_single_use_consts(&mut body);
        assert!(
            body.is_empty(),
            "Expected pure dead decl to be removed: {body:?}"
        );
    }

    #[test]
    fn dead_decl_impure_kept_as_expr() {
        let call = Expr::Call {
            func: "f".to_string(),
            args: vec![],
        };
        let mut body = vec![const_decl("v", call.clone())];
        fold_single_use_consts(&mut body);
        assert_eq!(body.len(), 1, "Expected one statement: {body:?}");
        match &body[0] {
            Stmt::Expr(expr) => {
                assert!(matches!(expr, Expr::Call { func, .. } if func == "f"));
            }
            other => panic!("Expected Stmt::Expr, got: {other:?}"),
        }
    }

    #[test]
    fn forward_sub_no_consume_loop_carried_var() {
        let mut body = vec![
            Stmt::VarDecl {
                name: "count".to_string(),
                ty: None,
                init: None,
                mutable: true,
            },
            assign(var("count"), int(3)),
            Stmt::Loop {
                body: vec![
                    Stmt::Expr(Expr::Call {
                        func: "doWork".to_string(),
                        args: vec![],
                    }),
                    assign(
                        var("count"),
                        Expr::Call {
                            func: "sub_i32".to_string(),
                            args: vec![var("count"), int(1)],
                        },
                    ),
                    Stmt::If {
                        cond: Expr::Call {
                            func: "isDone".to_string(),
                            args: vec![var("count")],
                        },
                        then_body: vec![Stmt::Break],
                        else_body: vec![],
                    },
                ],
            },
        ];
        forward_substitute(&mut body);
        let has_count_assign = body.iter().any(|s| {
            matches!(s, Stmt::Assign { target, value }
                if *target == var("count") && *value == int(3))
        });
        assert!(
            has_count_assign,
            "Init assign was incorrectly consumed: {body:?}"
        );
    }

    #[test]
    fn forward_sub_no_consume_nested_write_only() {
        let mut body = vec![
            Stmt::VarDecl {
                name: "i".to_string(),
                ty: None,
                init: None,
                mutable: true,
            },
            assign(
                var("i"),
                Expr::Ternary {
                    cond: Box::new(var("cond")),
                    then_val: Box::new(int(1)),
                    else_val: Box::new(int(0)),
                },
            ),
            Stmt::If {
                cond: var("x"),
                then_body: vec![assign(var("i"), int(0))],
                else_body: vec![],
            },
            Stmt::Return(Some(var("i"))),
        ];
        forward_substitute(&mut body);
        let has_ternary_assign = body.iter().any(|s| {
            matches!(s, Stmt::Assign { target, value }
                if *target == var("i") && matches!(value, Expr::Ternary { .. }))
        });
        assert!(
            has_ternary_assign,
            "Ternary assign was incorrectly consumed: {body:?}"
        );
    }

    #[test]
    fn inline_ordered_blocked_by_if() {
        let mut body = vec![
            Stmt::VarDecl {
                name: "v0".into(),
                ty: None,
                init: Some(br_call()),
                mutable: false,
            },
            Stmt::VarDecl {
                name: "v6".into(),
                ty: None,
                init: Some(Expr::Call {
                    func: "em".into(),
                    args: vec![Expr::ArrayInit(vec![str_lit("x")])],
                }),
                mutable: false,
            },
            Stmt::If {
                cond: var("cond"),
                then_body: vec![side_effect_call()],
                else_body: vec![],
            },
            Stmt::Return(Some(Expr::ArrayInit(vec![var("v0"), var("v6")]))),
        ];
        inline_ordered_single_use(&mut body);
        assert_eq!(body.len(), 4, "VarDecls should remain: {body:?}");
        assert!(matches!(&body[0], Stmt::VarDecl { name, .. } if name == "v0"));
        assert!(matches!(&body[1], Stmt::VarDecl { name, .. } if name == "v6"));
    }

    #[test]
    fn inline_ordered_cascading_no_barrier() {
        let em_call = Expr::Call {
            func: "em".into(),
            args: vec![Expr::ArrayInit(vec![str_lit("x")])],
        };
        let mut body = vec![
            Stmt::VarDecl {
                name: "v0".into(),
                ty: None,
                init: Some(br_call()),
                mutable: false,
            },
            Stmt::VarDecl {
                name: "v1".into(),
                ty: None,
                init: Some(em_call.clone()),
                mutable: false,
            },
            Stmt::VarDecl {
                name: "v2".into(),
                ty: None,
                init: Some(br_call()),
                mutable: false,
            },
            Stmt::Return(Some(Expr::ArrayInit(vec![var("v0"), var("v1"), var("v2")]))),
        ];
        inline_ordered_single_use(&mut body);
        assert_eq!(body.len(), 1, "All VarDecls should be removed: {body:?}");
        match &body[0] {
            Stmt::Return(Some(Expr::ArrayInit(elems))) => {
                assert_eq!(elems.len(), 3);
                assert_eq!(elems[0], br_call());
                assert_eq!(elems[1], em_call);
                assert_eq!(elems[2], br_call());
            }
            _ => panic!("Expected return with array: {body:?}"),
        }
    }

    #[test]
    fn inline_ordered_into_if_condition() {
        let mut body = vec![
            Stmt::VarDecl {
                name: "v0".into(),
                ty: None,
                init: Some(br_call()),
                mutable: false,
            },
            Stmt::If {
                cond: var("v0"),
                then_body: vec![side_effect_call()],
                else_body: vec![],
            },
        ];
        inline_ordered_single_use(&mut body);
        assert_eq!(body.len(), 1, "VarDecl should be removed: {body:?}");
        match &body[0] {
            Stmt::If { cond, .. } => {
                assert_eq!(*cond, br_call());
            }
            _ => panic!("Expected if: {body:?}"),
        }
    }

    #[test]
    fn inline_ordered_refuses_conditional_use() {
        let mut body = vec![
            Stmt::VarDecl {
                name: "v0".into(),
                ty: None,
                init: Some(br_call()),
                mutable: false,
            },
            Stmt::If {
                cond: var("cond"),
                then_body: vec![Stmt::Return(Some(Expr::ArrayInit(vec![var("v0")])))],
                else_body: vec![Stmt::Return(Some(empty_array()))],
            },
        ];
        inline_ordered_single_use(&mut body);
        assert_eq!(body.len(), 2, "VarDecl should remain: {body:?}");
        assert!(matches!(&body[0], Stmt::VarDecl { name, .. } if name == "v0"));
    }
}
