//! Phase 2: resolve — classify values for inlining
//!
//! Computes use counts, runs dead-code elimination to fixpoint, then
//! classifies each Def as constant, always-inline, lazy-inline, or
//! materialized.

use std::collections::{HashMap, HashSet};

use super::{is_deferrable, is_side_effecting_op, LinearStmt};
use crate::ir::func::Function;
use crate::ir::inst::{InstId, Op};
use crate::ir::value::{Constant, ValueId};
use crate::transforms::util::value_operands;

// -----------------------------------------------------------------------
// ResolveCtx — output of Phase 2
// -----------------------------------------------------------------------

/// Output of Phase 2: inlining classification for Phase 3.
pub(crate) struct ResolveCtx {
    /// Use counts after dead-code fixpoint.
    pub use_counts: HashMap<ValueId, usize>,
    /// Constants — always inlined, not consumed on read.
    pub constant_inlines: HashMap<ValueId, Constant>,
    /// Always-rebuild instructions (scope lookups + cascade).
    pub always_inlines: HashSet<ValueId>,
    /// Pure single-use values — consumed once at use site.
    pub lazy_inlines: HashSet<ValueId>,
    /// Alloc results with merged immediately-following Store.
    pub alloc_inits: HashMap<ValueId, ValueId>,
    /// Store InstIds merged into their preceding Alloc.
    pub skip_stores: HashSet<InstId>,
    /// Side-effecting values (count==1) defined in a nested scope (inside an
    /// if/while branch) but used in an outer scope.  The SE-inline mechanism
    /// would flush them into the branch's local stmts, putting the declaration
    /// out of scope at the use site.  These must be emitted as a shared-name
    /// Assign so `collect_block_param_decls` hoists a `let name!` to the
    /// function scope.
    pub cross_scope_defs: HashSet<ValueId>,
}

/// Classify all values in linearized IR for inlining decisions.
///
/// Computes use counts from LinearStmt (not raw IR — terminators like
/// Br/BrIf/Switch are absent, so their operand uses aren't counted). Runs
/// dead-code elimination to fixpoint, then classifies each Def as constant,
/// always-inline, lazy-inline, or materialized.
pub(crate) fn resolve(
    func: &Function,
    stmts: &[LinearStmt],
    scope_lookup_systems: &[String],
) -> ResolveCtx {
    // Step 1: compute use counts from LinearStmt.
    let mut use_counts = HashMap::new();
    count_uses_in_stmts(func, stmts, &mut use_counts);

    // Step 2: dead code elimination fixpoint.
    let mut dead: HashSet<ValueId> = HashSet::new();
    loop {
        let mut changed = false;
        collect_dead_uses(func, stmts, &mut use_counts, &mut dead, &mut changed);
        if !changed {
            break;
        }
    }

    // Step 3: classify remaining Defs.
    let mut constant_inlines = HashMap::new();
    let mut always_inlines = HashSet::new();
    let mut lazy_inlines = HashSet::new();
    classify_defs(
        func,
        stmts,
        &use_counts,
        &mut constant_inlines,
        &mut always_inlines,
        &mut lazy_inlines,
        scope_lookup_systems,
    );

    // Step 4: detect adjacent Alloc+Store patterns for merged init.
    let (alloc_inits, skip_stores) = find_alloc_store_merges(func, stmts);

    // Step 5: detect side-effecting values that are defined in a nested scope
    // but used in an outer scope.
    let cross_scope_defs = compute_cross_scope_defs(func, stmts, &use_counts);

    ResolveCtx {
        use_counts,
        constant_inlines,
        always_inlines,
        lazy_inlines,
        alloc_inits,
        skip_stores,
        cross_scope_defs,
    }
}

// -----------------------------------------------------------------------
// Use counting
// -----------------------------------------------------------------------

/// Count value uses across all LinearStmts recursively.
fn count_uses_in_stmts(
    func: &Function,
    stmts: &[LinearStmt],
    counts: &mut HashMap<ValueId, usize>,
) {
    for stmt in stmts {
        match stmt {
            LinearStmt::Def { inst_id, .. } | LinearStmt::Effect { inst_id } => {
                for v in value_operands(&func.insts[*inst_id].op) {
                    *counts.entry(v).or_default() += 1;
                }
            }
            LinearStmt::Assign { src, .. } => {
                *counts.entry(*src).or_default() += 1;
            }
            LinearStmt::If {
                cond,
                then_body,
                else_body,
            } => {
                *counts.entry(*cond).or_default() += 1;
                count_uses_in_stmts(func, then_body, counts);
                count_uses_in_stmts(func, else_body, counts);
            }
            LinearStmt::While {
                header, cond, body, ..
            } => {
                count_uses_in_stmts(func, header, counts);
                *counts.entry(*cond).or_default() += 1;
                count_uses_in_stmts(func, body, counts);
            }
            LinearStmt::For {
                init,
                header,
                cond,
                update,
                body,
                ..
            } => {
                count_uses_in_stmts(func, init, counts);
                count_uses_in_stmts(func, header, counts);
                *counts.entry(*cond).or_default() += 1;
                count_uses_in_stmts(func, update, counts);
                count_uses_in_stmts(func, body, counts);
            }
            LinearStmt::Loop { body } => {
                count_uses_in_stmts(func, body, counts);
            }
            LinearStmt::Return { value } => {
                if let Some(v) = value {
                    *counts.entry(*v).or_default() += 1;
                }
            }
            // LogicalOr/And: cond used once (as lhs of `||`/`&&`).
            // When rhs == phi (nested logical op), the emitter skips the
            // rhs reference, so don't count it.
            LinearStmt::LogicalOr {
                cond,
                phi,
                rhs_body,
                rhs,
            }
            | LinearStmt::LogicalAnd {
                cond,
                phi,
                rhs_body,
                rhs,
            } => {
                *counts.entry(*cond).or_default() += 1;
                count_uses_in_stmts(func, rhs_body, counts);
                if *rhs != *phi {
                    *counts.entry(*rhs).or_default() += 1;
                }
            }
            LinearStmt::Dispatch { blocks, .. } => {
                for (_, block_stmts) in blocks {
                    count_uses_in_stmts(func, block_stmts, counts);
                }
            }
            LinearStmt::Switch {
                value,
                cases,
                default_body,
            } => {
                *counts.entry(*value).or_default() += 1;
                for (_, case_stmts) in cases {
                    count_uses_in_stmts(func, case_stmts, counts);
                }
                count_uses_in_stmts(func, default_body, counts);
            }
            LinearStmt::Break | LinearStmt::Continue | LinearStmt::LabeledBreak { .. } => {}
        }
    }
}

// -----------------------------------------------------------------------
// Cross-scope def detection
// -----------------------------------------------------------------------

/// Populate def_depths and min_use_depths by recursively walking the
/// LinearStmt tree.
fn collect_def_use_depths(
    func: &Function,
    stmts: &[LinearStmt],
    depth: usize,
    defs: &mut HashMap<ValueId, (usize, InstId)>,
    min_use_depths: &mut HashMap<ValueId, usize>,
) {
    let update_use = |v: ValueId, d: usize, mu: &mut HashMap<ValueId, usize>| {
        let e = mu.entry(v).or_insert(usize::MAX);
        if d < *e {
            *e = d;
        }
    };
    for stmt in stmts {
        match stmt {
            LinearStmt::Def { result, inst_id } => {
                defs.entry(*result).or_insert((depth, *inst_id));
                for v in value_operands(&func.insts[*inst_id].op) {
                    update_use(v, depth, min_use_depths);
                }
            }
            LinearStmt::Effect { inst_id } => {
                for v in value_operands(&func.insts[*inst_id].op) {
                    update_use(v, depth, min_use_depths);
                }
            }
            LinearStmt::Assign { src, .. } => {
                update_use(*src, depth, min_use_depths);
            }
            LinearStmt::Return { value: Some(v) } => {
                update_use(*v, depth, min_use_depths);
            }
            LinearStmt::If {
                cond,
                then_body,
                else_body,
            } => {
                update_use(*cond, depth, min_use_depths);
                collect_def_use_depths(func, then_body, depth + 1, defs, min_use_depths);
                collect_def_use_depths(func, else_body, depth + 1, defs, min_use_depths);
            }
            LinearStmt::While {
                header, cond, body, ..
            } => {
                collect_def_use_depths(func, header, depth, defs, min_use_depths);
                update_use(*cond, depth, min_use_depths);
                collect_def_use_depths(func, body, depth + 1, defs, min_use_depths);
            }
            LinearStmt::For {
                init,
                header,
                cond,
                update,
                body,
                ..
            } => {
                collect_def_use_depths(func, init, depth, defs, min_use_depths);
                collect_def_use_depths(func, header, depth, defs, min_use_depths);
                update_use(*cond, depth, min_use_depths);
                collect_def_use_depths(func, update, depth, defs, min_use_depths);
                collect_def_use_depths(func, body, depth + 1, defs, min_use_depths);
            }
            LinearStmt::Loop { body } => {
                collect_def_use_depths(func, body, depth + 1, defs, min_use_depths);
            }
            LinearStmt::LogicalOr {
                cond,
                rhs_body,
                rhs,
                phi,
            }
            | LinearStmt::LogicalAnd {
                cond,
                rhs_body,
                rhs,
                phi,
            } => {
                update_use(*cond, depth, min_use_depths);
                collect_def_use_depths(func, rhs_body, depth + 1, defs, min_use_depths);
                if *rhs != *phi {
                    update_use(*rhs, depth, min_use_depths);
                }
            }
            LinearStmt::Switch {
                value,
                cases,
                default_body,
            } => {
                update_use(*value, depth, min_use_depths);
                for (_, case_stmts) in cases {
                    collect_def_use_depths(func, case_stmts, depth + 1, defs, min_use_depths);
                }
                collect_def_use_depths(func, default_body, depth + 1, defs, min_use_depths);
            }
            LinearStmt::Dispatch { blocks, .. } => {
                for (_, block_stmts) in blocks {
                    collect_def_use_depths(func, block_stmts, depth + 1, defs, min_use_depths);
                }
            }
            LinearStmt::Return { value: None }
            | LinearStmt::Break
            | LinearStmt::Continue
            | LinearStmt::LabeledBreak { .. } => {}
        }
    }
}

/// Compute the set of values that are defined in a nested scope but used in
/// an outer scope.
fn compute_cross_scope_defs(
    func: &Function,
    stmts: &[LinearStmt],
    use_counts: &HashMap<ValueId, usize>,
) -> HashSet<ValueId> {
    let mut defs: HashMap<ValueId, (usize, InstId)> = HashMap::new();
    let mut min_use_depths: HashMap<ValueId, usize> = HashMap::new();
    collect_def_use_depths(func, stmts, 0, &mut defs, &mut min_use_depths);

    defs.iter()
        .filter(|(&v, &(def_d, inst_id))| {
            let count = use_counts.get(&v).copied().unwrap_or(0);
            if count == 1 && !is_side_effecting_op(&func.insts[inst_id].op) {
                return false;
            }
            if count == 0 {
                return false;
            }
            let min_use_d = min_use_depths.get(&v).copied().unwrap_or(def_d);
            min_use_d < def_d
        })
        .map(|(&v, _)| v)
        .collect()
}

// -----------------------------------------------------------------------
// Dead code fixpoint
// -----------------------------------------------------------------------

/// Find dead deferrable Defs and decrement their operands' use counts.
fn collect_dead_uses(
    func: &Function,
    stmts: &[LinearStmt],
    counts: &mut HashMap<ValueId, usize>,
    dead: &mut HashSet<ValueId>,
    changed: &mut bool,
) {
    for stmt in stmts {
        match stmt {
            LinearStmt::Def { result, inst_id } => {
                if dead.contains(result) {
                    continue;
                }
                let count = counts.get(result).copied().unwrap_or(0);
                let op = &func.insts[*inst_id].op;
                if count == 0 && is_deferrable(op) {
                    dead.insert(*result);
                    for v in value_operands(op) {
                        if let Some(c) = counts.get_mut(&v) {
                            *c = c.saturating_sub(1);
                        }
                    }
                    *changed = true;
                }
            }
            LinearStmt::If {
                then_body,
                else_body,
                ..
            } => {
                collect_dead_uses(func, then_body, counts, dead, changed);
                collect_dead_uses(func, else_body, counts, dead, changed);
            }
            LinearStmt::While { header, body, .. } => {
                collect_dead_uses(func, header, counts, dead, changed);
                collect_dead_uses(func, body, counts, dead, changed);
            }
            LinearStmt::For {
                init,
                header,
                update,
                body,
                ..
            } => {
                collect_dead_uses(func, init, counts, dead, changed);
                collect_dead_uses(func, header, counts, dead, changed);
                collect_dead_uses(func, update, counts, dead, changed);
                collect_dead_uses(func, body, counts, dead, changed);
            }
            LinearStmt::Loop { body } => {
                collect_dead_uses(func, body, counts, dead, changed);
            }
            LinearStmt::LogicalOr { rhs_body, .. } | LinearStmt::LogicalAnd { rhs_body, .. } => {
                collect_dead_uses(func, rhs_body, counts, dead, changed);
            }
            LinearStmt::Dispatch { blocks, .. } => {
                for (_, block_stmts) in blocks {
                    collect_dead_uses(func, block_stmts, counts, dead, changed);
                }
            }
            LinearStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_stmts) in cases {
                    collect_dead_uses(func, case_stmts, counts, dead, changed);
                }
                collect_dead_uses(func, default_body, counts, dead, changed);
            }
            _ => {}
        }
    }
}

// -----------------------------------------------------------------------
// Classification
// -----------------------------------------------------------------------

/// Scope-lookup calls are pure metadata — always rebuild.
fn is_scope_lookup_op(op: &Op, scope_lookup_systems: &[String]) -> bool {
    if scope_lookup_systems.is_empty() {
        return false;
    }
    matches!(
        op,
        Op::SystemCall { system, method, .. }
            if scope_lookup_systems.contains(system)
                && (method == "findPropStrict" || method == "findProperty")
    )
}

/// Classify non-dead Defs into constant, always-inline, or lazy-inline.
fn classify_defs(
    func: &Function,
    stmts: &[LinearStmt],
    counts: &HashMap<ValueId, usize>,
    constant_inlines: &mut HashMap<ValueId, Constant>,
    always_inlines: &mut HashSet<ValueId>,
    lazy_inlines: &mut HashSet<ValueId>,
    scope_lookup_systems: &[String],
) {
    for stmt in stmts {
        match stmt {
            LinearStmt::Def { result, inst_id } => {
                let count = counts.get(result).copied().unwrap_or(0);
                let op = &func.insts[*inst_id].op;

                if count == 0 && is_deferrable(op) {
                    continue;
                }

                if let Op::Const(c) = op {
                    constant_inlines.insert(*result, c.clone());
                    continue;
                }

                if is_scope_lookup_op(op, scope_lookup_systems) {
                    always_inlines.insert(*result);
                    continue;
                }

                if matches!(op, Op::GlobalRef(_)) {
                    always_inlines.insert(*result);
                    continue;
                }

                let object_always_inlined = match op {
                    Op::GetField { object, .. }
                    | Op::GetIndex {
                        collection: object, ..
                    } => always_inlines.contains(object),
                    _ => false,
                };
                if object_always_inlined {
                    always_inlines.insert(*result);
                    continue;
                }

                if count == 1 && is_deferrable(op) {
                    lazy_inlines.insert(*result);
                }
            }
            LinearStmt::If {
                then_body,
                else_body,
                ..
            } => {
                classify_defs(
                    func,
                    then_body,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
                classify_defs(
                    func,
                    else_body,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
            }
            LinearStmt::While { header, body, .. } => {
                classify_defs(
                    func,
                    header,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
                classify_defs(
                    func,
                    body,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
            }
            LinearStmt::For {
                init,
                header,
                update,
                body,
                ..
            } => {
                classify_defs(
                    func,
                    init,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
                classify_defs(
                    func,
                    header,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
                classify_defs(
                    func,
                    update,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
                classify_defs(
                    func,
                    body,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
            }
            LinearStmt::Loop { body } => {
                classify_defs(
                    func,
                    body,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
            }
            LinearStmt::LogicalOr { rhs_body, .. } | LinearStmt::LogicalAnd { rhs_body, .. } => {
                classify_defs(
                    func,
                    rhs_body,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
            }
            LinearStmt::Dispatch { blocks, .. } => {
                for (_, block_stmts) in blocks {
                    classify_defs(
                        func,
                        block_stmts,
                        counts,
                        constant_inlines,
                        always_inlines,
                        lazy_inlines,
                        scope_lookup_systems,
                    );
                }
            }
            LinearStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_stmts) in cases {
                    classify_defs(
                        func,
                        case_stmts,
                        counts,
                        constant_inlines,
                        always_inlines,
                        lazy_inlines,
                        scope_lookup_systems,
                    );
                }
                classify_defs(
                    func,
                    default_body,
                    counts,
                    constant_inlines,
                    always_inlines,
                    lazy_inlines,
                    scope_lookup_systems,
                );
            }
            _ => {}
        }
    }
}

// -----------------------------------------------------------------------
// Alloc+Store merging
// -----------------------------------------------------------------------

/// Find adjacent Alloc+Store pairs where the Store immediately initializes
/// the Alloc result.
fn find_alloc_store_merges(
    func: &Function,
    stmts: &[LinearStmt],
) -> (HashMap<ValueId, ValueId>, HashSet<InstId>) {
    let mut alloc_inits = HashMap::new();
    let mut skip_stores = HashSet::new();
    scan_alloc_stores(func, stmts, &mut alloc_inits, &mut skip_stores);
    (alloc_inits, skip_stores)
}

fn scan_alloc_stores(
    func: &Function,
    stmts: &[LinearStmt],
    alloc_inits: &mut HashMap<ValueId, ValueId>,
    skip_stores: &mut HashSet<InstId>,
) {
    for pair in stmts.windows(2) {
        if let (
            LinearStmt::Def {
                result: alloc_r,
                inst_id: alloc_iid,
            },
            LinearStmt::Effect { inst_id: store_iid },
        ) = (&pair[0], &pair[1])
        {
            if matches!(func.insts[*alloc_iid].op, Op::Alloc(_)) {
                if let Op::Store { ptr, value } = &func.insts[*store_iid].op {
                    if *ptr == *alloc_r {
                        alloc_inits.insert(*alloc_r, *value);
                        skip_stores.insert(*store_iid);
                    }
                }
            }
        }
    }
    // Recurse into nested bodies.
    for stmt in stmts {
        match stmt {
            LinearStmt::If {
                then_body,
                else_body,
                ..
            } => {
                scan_alloc_stores(func, then_body, alloc_inits, skip_stores);
                scan_alloc_stores(func, else_body, alloc_inits, skip_stores);
            }
            LinearStmt::While { header, body, .. } => {
                scan_alloc_stores(func, header, alloc_inits, skip_stores);
                scan_alloc_stores(func, body, alloc_inits, skip_stores);
            }
            LinearStmt::For {
                init,
                header,
                update,
                body,
                ..
            } => {
                scan_alloc_stores(func, init, alloc_inits, skip_stores);
                scan_alloc_stores(func, header, alloc_inits, skip_stores);
                scan_alloc_stores(func, update, alloc_inits, skip_stores);
                scan_alloc_stores(func, body, alloc_inits, skip_stores);
            }
            LinearStmt::Loop { body } => {
                scan_alloc_stores(func, body, alloc_inits, skip_stores);
            }
            LinearStmt::LogicalOr { rhs_body, .. } | LinearStmt::LogicalAnd { rhs_body, .. } => {
                scan_alloc_stores(func, rhs_body, alloc_inits, skip_stores);
            }
            LinearStmt::Dispatch { blocks, .. } => {
                for (_, block_stmts) in blocks {
                    scan_alloc_stores(func, block_stmts, alloc_inits, skip_stores);
                }
            }
            LinearStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_stmts) in cases {
                    scan_alloc_stores(func, case_stmts, alloc_inits, skip_stores);
                }
                scan_alloc_stores(func, default_body, alloc_inits, skip_stores);
            }
            _ => {}
        }
    }
}
