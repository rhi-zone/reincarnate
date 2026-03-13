//! Structured linear IR for the hybrid lowering pipeline.
//!
//! The pipeline converts `Shape + Function` → `Vec<Stmt>` in three phases:
//!
//! - **Phase 1** (`linearize`): Walk the Shape tree and produce a flat
//!   `Vec<LinearStmt>` where every instruction is a `Def(ValueId, InstId)`,
//!   control flow comes from shapes, and branch args become `Assign(dst, src)`.
//!   No inlining decisions — trivial shape walk.
//!
//! - **Phase 2** (`resolve`): Pure resolution on LinearStmt. Constants always
//!   inlined, scope lookups marked always-rebuild, pure single-use values
//!   marked for substitution, dead pure code dropped. This handles ~90% of
//!   inlining with zero side-effect concerns.
//!
//! - **Phase 3** (`emit`): LinearStmt → Vec<Stmt>. Remaining side-effecting
//!   single-use values inlined if no intervening side effects. Multi-use
//!   values get `const`/`let` declarations. Produces the AST for existing
//!   AST-to-AST passes.

mod emit;
mod linearize;
mod resolve;

#[cfg(test)]
mod tests;

use super::ast::{AstFunction, Stmt};
use super::ast_passes;
use super::func::Function;
use super::inst::{InstId, Op};
use super::structurize::Shape;
use super::ty::Type;
use super::value::{Constant, ValueId};
use crate::pipeline::DebugConfig;
use crate::pipeline::LoweringConfig;

use emit::EmitCtx;
use linearize::linearize;
use resolve::resolve;

// -----------------------------------------------------------------------
// LinearStmt — structured IR with ValueId/InstId references
// -----------------------------------------------------------------------

/// A statement in the structured linear IR.
///
/// References IR entities (ValueId, InstId) rather than carrying string names
/// or materialized expressions. The `Function` provides context for looking up
/// instruction ops and value metadata.
#[derive(Debug, Clone)]
pub(crate) enum LinearStmt {
    /// Instruction with a result value: `result = op(...)`.
    Def { result: ValueId, inst_id: InstId },
    /// Instruction without a useful result (void calls, stores, etc.).
    Effect { inst_id: InstId },
    /// Branch argument assignment: `dst = src`.
    Assign { dst: ValueId, src: ValueId },
    /// Conditional: `if (cond) { then } else { else }`.
    If {
        cond: ValueId,
        then_body: Vec<LinearStmt>,
        else_body: Vec<LinearStmt>,
    },
    /// While loop. Header instructions compute the condition each iteration.
    While {
        header: Vec<LinearStmt>,
        cond: ValueId,
        cond_negated: bool,
        body: Vec<LinearStmt>,
    },
    /// For loop: init; header+cond; body; update.
    For {
        init: Vec<LinearStmt>,
        header: Vec<LinearStmt>,
        cond: ValueId,
        cond_negated: bool,
        update: Vec<LinearStmt>,
        body: Vec<LinearStmt>,
    },
    /// Infinite loop (`while (true) { ... }`).
    Loop { body: Vec<LinearStmt> },
    /// Return from function.
    Return { value: Option<ValueId> },
    /// Break out of innermost loop.
    Break,
    /// Continue to next iteration.
    Continue,
    /// Break to an outer loop (`depth` levels up).
    LabeledBreak { depth: usize },
    /// Short-circuit OR: `phi = cond || rhs`.
    LogicalOr {
        cond: ValueId,
        phi: ValueId,
        rhs_body: Vec<LinearStmt>,
        rhs: ValueId,
    },
    /// Short-circuit AND: `phi = cond && rhs`.
    LogicalAnd {
        cond: ValueId,
        phi: ValueId,
        rhs_body: Vec<LinearStmt>,
        rhs: ValueId,
    },
    /// Switch statement: `switch (value) { case X: ...; default: ...; }`.
    Switch {
        value: ValueId,
        cases: Vec<(Constant, Vec<LinearStmt>)>,
        default_body: Vec<LinearStmt>,
    },
    /// Dispatch (fallback for irreducible CFGs).
    Dispatch {
        blocks: Vec<(usize, Vec<LinearStmt>)>,
        entry: usize,
    },
}

// -----------------------------------------------------------------------
// Public entry point
// -----------------------------------------------------------------------

/// Lower a function through all 3 phases of the hybrid pipeline.
pub fn lower_function_linear(
    func: &Function,
    shape: &Shape,
    config: &LoweringConfig,
    debug: &DebugConfig,
) -> AstFunction {
    let linear = linearize(func, shape);
    let rctx = resolve(func, &linear, &config.scope_lookup_systems);
    let mut ctx = EmitCtx::new(func, &rctx, config);

    let mut body = ctx.emit_stmts(&linear);
    strip_trailing_void_return(&mut body);

    let decls = ctx.collect_block_param_decls();
    let mut full_body = decls;
    full_body.append(&mut body);

    if debug.dump_ast && debug.should_dump(&func.name) {
        eprintln!("=== AST (pre-passes): {} ===", func.name);
        for stmt in &full_body {
            eprintln!("{stmt:#?}");
        }
        eprintln!("=== end AST: {} ===\n", func.name);
    }

    // AST-to-AST rewrite passes.
    // Lower Harlowe.H.* SystemCalls to h.method() MethodCall nodes before
    // optimization, so passes see them as regular method calls.
    ast_passes::lower_output_nodes(&mut full_body);
    // Cleanup first: self-assigns and stubs block ternary detection by adding
    // extra statements to if/else branches.
    ast_passes::eliminate_self_assigns(&mut full_body);
    ast_passes::eliminate_duplicate_assigns(&mut full_body);
    ast_passes::eliminate_forwarding_stubs(&mut full_body);
    ast_passes::invert_empty_then(&mut full_body);
    ast_passes::eliminate_unreachable_after_exit(&mut full_body);
    if config.ternary {
        ast_passes::rewrite_ternary(&mut full_body);
    }
    if config.minmax {
        ast_passes::rewrite_minmax(&mut full_body);
    }

    // Fixpoint: forward sub enables ternary, ternary enables narrow/merge/fold,
    // fold may remove statements that enable further forward sub, absorb_phi
    // merges split-path conditions into their assigning branch.
    loop {
        let before = ast_passes::count_stmts(&full_body);
        ast_passes::forward_substitute(&mut full_body);
        if config.ternary {
            ast_passes::rewrite_ternary(&mut full_body);
        }
        ast_passes::simplify_ternary_to_logical(&mut full_body);
        ast_passes::absorb_phi_condition(&mut full_body);
        ast_passes::fold_identical_branch_assigns(&mut full_body);
        ast_passes::narrow_var_scope(&mut full_body);
        ast_passes::merge_decl_init(&mut full_body);
        ast_passes::fold_single_use_consts(&mut full_body);
        if ast_passes::count_stmts(&full_body) == before {
            break;
        }
    }

    // Order-preserving inline: substitute single-use vars at their use
    // sites in declaration order. Only inlines into unconditional positions
    // (not inside if/loop bodies) to avoid making calls conditional.
    ast_passes::inline_ordered_single_use(&mut full_body);

    if config.foreach_rewrite {
        ast_passes::rewrite_foreach_loops(&mut full_body);
        // Clean up dead variables left by the foreach rewrite
        // (e.g., the index register decl, single-use collection var).
        ast_passes::narrow_var_scope(&mut full_body);
        ast_passes::merge_decl_init(&mut full_body);
        ast_passes::fold_single_use_consts(&mut full_body);
    }

    ast_passes::rewrite_compound_assign(&mut full_body);
    ast_passes::rewrite_post_increment(&mut full_body);
    ast_passes::promote_while_to_for(&mut full_body);

    // Build param defaults: thread from FunctionSig, then pad with None for capture params.
    let mut param_defaults = func.sig.defaults.clone();
    let total_params = func.sig.params.len() + func.capture_params.len();
    param_defaults.resize(total_params, None);

    AstFunction {
        name: func.name.clone(),
        params: ctx.build_params(),
        param_defaults,
        return_ty: func.sig.return_ty.clone(),
        body: full_body,
        is_generator: func.coroutine.is_some(),
        visibility: func.visibility,
        method_kind: func.method_kind,
        has_rest_param: func.sig.has_rest_param,
        num_capture_params: func.capture_params.len(),
        capture_modes: func.capture_params.iter().map(|cp| cp.mode).collect(),
    }
}

// -----------------------------------------------------------------------
// Shared helpers
// -----------------------------------------------------------------------

/// Whether an instruction is pure enough to defer for inlining.
fn is_deferrable(op: &Op) -> bool {
    matches!(
        op,
        Op::Const(_)
            | Op::Add(..)
            | Op::Sub(..)
            | Op::Mul(..)
            | Op::Div(..)
            | Op::Rem(..)
            | Op::Neg(..)
            | Op::Not(..)
            | Op::BoolAnd(..)
            | Op::BoolOr(..)
            | Op::BitAnd(..)
            | Op::BitOr(..)
            | Op::BitXor(..)
            | Op::BitNot(..)
            | Op::Shl(..)
            | Op::Shr(..)
            | Op::Cmp(..)
            | Op::Cast(..)
            | Op::Copy(..)
            | Op::GetField { .. }
            | Op::GetIndex { .. }
            | Op::Load(..)
            | Op::Select { .. }
            | Op::ArrayInit(..)
            | Op::TupleInit(..)
            | Op::StructInit { .. }
            | Op::GlobalRef(..)
            | Op::TypeCheck(..)
    )
}

fn is_side_effecting_op(op: &Op) -> bool {
    matches!(
        op,
        Op::Call { .. }
            | Op::CallIndirect { .. }
            | Op::SystemCall { .. }
            | Op::MethodCall { .. }
            | Op::Yield(..)
            | Op::CoroutineResume(..)
    )
}

fn is_memory_write(op: &Op) -> bool {
    matches!(
        op,
        Op::SetField { .. } | Op::SetIndex { .. } | Op::Store { .. }
    )
}

fn is_js_ident(s: &str) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        None => return false,
        Some(c) if c == '_' || c == '$' || unicode_ident::is_xid_start(c) => {}
        Some(_) => return false,
    }
    chars.all(|c| c == '_' || c == '$' || unicode_ident::is_xid_continue(c))
}

/// Return true if `init_ty` is assignment-compatible with `decl_ty` for
/// TypeScript purposes.
fn types_coalesce_compatible(decl_ty: &Type, init_ty: &Type) -> bool {
    if decl_ty == init_ty {
        return true;
    }
    match (decl_ty, init_ty) {
        (Type::Dynamic, _) | (_, Type::Dynamic) => true,
        (Type::Option(inner), ty) | (ty, Type::Option(inner)) => inner.as_ref() == ty,
        _ => false,
    }
}

fn strip_trailing_continue(stmts: &mut Vec<Stmt>) {
    if matches!(stmts.last(), Some(Stmt::Continue)) {
        stmts.pop();
    }
}

fn strip_trailing_void_return(stmts: &mut Vec<Stmt>) {
    if matches!(stmts.last(), Some(Stmt::Return(None))) {
        stmts.pop();
    }
}
