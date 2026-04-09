use std::collections::{HashMap, HashSet};

use crate::error::CoreError;
use crate::ir::block::BlockId;
use crate::ir::func::FuncId;
use crate::ir::inst::{CmpKind, Terminator};
use crate::ir::{Constant, Function, InstId, Module, Op, Type, ValueId};
use crate::pipeline::{Transform, TransformResult};

use super::util::branch_targets;

/// Constant folding transform — evaluates operations with all-constant operands
/// at compile time, replacing them with `Op::Const(result)`.
pub struct ConstantFolding;

/// Compute the set of blocks reachable from the function's entry block.
fn reachable_blocks(func: &Function) -> HashSet<BlockId> {
    let mut reachable = HashSet::new();
    let mut worklist = vec![func.entry];
    while let Some(block_id) = worklist.pop() {
        if !reachable.insert(block_id) {
            continue;
        }
        for target in branch_targets(&func.blocks[block_id].terminator) {
            worklist.push(target);
        }
    }
    reachable
}

/// Build a map from ValueId → Constant for all `Op::Const` instructions.
///
/// Also propagates constants through block parameters: if every *reachable*
/// predecessor that passes to a block param passes the same constant, the
/// param is effectively a constant.  Only reachable predecessors are counted
/// so that dead-branch edges (from previously folded `BrIf`s) don't prevent
/// propagation.
fn build_const_map(func: &Function) -> HashMap<ValueId, Constant> {
    let mut map = HashMap::new();
    for (_, inst) in func.insts.iter() {
        if let (Op::Const(c), Some(result)) = (&inst.op, inst.result) {
            map.insert(result, c.clone());
        }
    }
    // Propagate constants through block parameters.
    // Only edges from reachable blocks are counted — unreachable blocks
    // (e.g. dead branches from a previously folded BrIf) must not block
    // propagation through the surviving predecessor's constant.
    let reachable = reachable_blocks(func);
    let mut param_vals: HashMap<(BlockId, usize), Vec<ValueId>> = HashMap::new();
    for (block_id, block) in func.blocks.iter() {
        if !reachable.contains(&block_id) {
            continue;
        }
        match &block.terminator {
            Terminator::Br { target, args } => {
                for (i, &arg) in args.iter().enumerate() {
                    param_vals.entry((*target, i)).or_default().push(arg);
                }
            }
            Terminator::BrIf {
                then_target,
                then_args,
                else_target,
                else_args,
                ..
            } => {
                for (i, &arg) in then_args.iter().enumerate() {
                    param_vals.entry((*then_target, i)).or_default().push(arg);
                }
                for (i, &arg) in else_args.iter().enumerate() {
                    param_vals.entry((*else_target, i)).or_default().push(arg);
                }
            }
            Terminator::Switch { cases, default, .. } => {
                for (_, target, args) in cases {
                    for (i, &arg) in args.iter().enumerate() {
                        param_vals.entry((*target, i)).or_default().push(arg);
                    }
                }
                for (i, &arg) in default.1.iter().enumerate() {
                    param_vals.entry((default.0, i)).or_default().push(arg);
                }
            }
            Terminator::Return(_) => {}
        }
    }
    for (block_id, block) in func.blocks.iter() {
        for (i, param) in block.params.iter().enumerate() {
            let Some(vals) = param_vals.get(&(block_id, i)) else {
                continue;
            };
            if vals.is_empty() {
                continue;
            }
            // All reachable predecessor values must resolve to the same constant.
            let mut agreed: Option<Constant> = None;
            let all_const = vals.iter().all(|v| {
                if let Some(c) = map.get(v) {
                    match &agreed {
                        None => {
                            agreed = Some(c.clone());
                            true
                        }
                        Some(a) => a == c,
                    }
                } else {
                    false
                }
            });
            if all_const {
                if let Some(c) = agreed {
                    map.insert(param.value, c);
                }
            }
        }
    }
    map
}

/// Try to fold a binary arithmetic operation on two constants.
fn fold_binary_arith(op_name: &str, a: &Constant, b: &Constant) -> Option<Constant> {
    match (op_name, a, b) {
        ("add", Constant::Int(x), Constant::Int(y)) => Some(Constant::Int(x.wrapping_add(*y))),
        ("add", Constant::UInt(x), Constant::UInt(y)) => Some(Constant::UInt(x.wrapping_add(*y))),
        ("add", Constant::Float(x), Constant::Float(y)) => Some(Constant::Float(x + y)),

        ("sub", Constant::Int(x), Constant::Int(y)) => Some(Constant::Int(x.wrapping_sub(*y))),
        ("sub", Constant::UInt(x), Constant::UInt(y)) => Some(Constant::UInt(x.wrapping_sub(*y))),
        ("sub", Constant::Float(x), Constant::Float(y)) => Some(Constant::Float(x - y)),

        ("mul", Constant::Int(x), Constant::Int(y)) => Some(Constant::Int(x.wrapping_mul(*y))),
        ("mul", Constant::UInt(x), Constant::UInt(y)) => Some(Constant::UInt(x.wrapping_mul(*y))),
        ("mul", Constant::Float(x), Constant::Float(y)) => Some(Constant::Float(x * y)),

        ("div", Constant::Int(_), Constant::Int(0)) => None,
        ("div", Constant::UInt(_), Constant::UInt(0)) => None,
        ("div", Constant::Int(x), Constant::Int(y)) => Some(Constant::Int(x.wrapping_div(*y))),
        ("div", Constant::UInt(x), Constant::UInt(y)) => Some(Constant::UInt(x / y)),
        ("div", Constant::Float(x), Constant::Float(y)) => Some(Constant::Float(x / y)),

        ("rem", Constant::Int(_), Constant::Int(0)) => None,
        ("rem", Constant::UInt(_), Constant::UInt(0)) => None,
        ("rem", Constant::Int(x), Constant::Int(y)) => Some(Constant::Int(x.wrapping_rem(*y))),
        ("rem", Constant::UInt(x), Constant::UInt(y)) => Some(Constant::UInt(x % y)),
        ("rem", Constant::Float(x), Constant::Float(y)) => Some(Constant::Float(x % y)),

        _ => None,
    }
}

/// Try to fold a bitwise binary operation on two constants.
fn fold_binary_bitwise(op_name: &str, a: &Constant, b: &Constant) -> Option<Constant> {
    match (op_name, a, b) {
        ("and", Constant::Int(x), Constant::Int(y)) => Some(Constant::Int(x & y)),
        ("and", Constant::UInt(x), Constant::UInt(y)) => Some(Constant::UInt(x & y)),

        ("or", Constant::Int(x), Constant::Int(y)) => Some(Constant::Int(x | y)),
        ("or", Constant::UInt(x), Constant::UInt(y)) => Some(Constant::UInt(x | y)),

        ("xor", Constant::Int(x), Constant::Int(y)) => Some(Constant::Int(x ^ y)),
        ("xor", Constant::UInt(x), Constant::UInt(y)) => Some(Constant::UInt(x ^ y)),

        ("shl", Constant::Int(x), Constant::Int(y)) => {
            Some(Constant::Int(x.wrapping_shl(*y as u32)))
        }
        ("shl", Constant::UInt(x), Constant::UInt(y)) => {
            Some(Constant::UInt(x.wrapping_shl(*y as u32)))
        }

        ("shr", Constant::Int(x), Constant::Int(y)) => {
            Some(Constant::Int(x.wrapping_shr(*y as u32)))
        }
        ("shr", Constant::UInt(x), Constant::UInt(y)) => {
            Some(Constant::UInt(x.wrapping_shr(*y as u32)))
        }

        _ => None,
    }
}

/// Absorbing-element fold for bitwise ops where only one operand is a constant.
///
/// - `x & 0`     / `0 & x`     → `0`      (zero is absorbing for integer AND)
/// - `x | -1`    / `-1 | x`    → `-1`     (-1 / all-ones is absorbing for integer OR)
///
/// Note: Bool AND/OR absorbing cases are handled by `and_bool`/`or_bool` builtins in `try_fold`.
fn fold_bitwise_absorbing(op_name: &str, known: &Constant) -> Option<Constant> {
    match (op_name, known) {
        ("and", Constant::Int(0)) => Some(Constant::Int(0)),
        ("and", Constant::UInt(0)) => Some(Constant::UInt(0)),
        ("or", Constant::Int(-1)) => Some(Constant::Int(-1)),
        _ => None,
    }
}

/// Try to fold a comparison of two constants.
fn fold_cmp(kind: CmpKind, a: &Constant, b: &Constant) -> Option<Constant> {
    let result = match (a, b) {
        (Constant::Int(x), Constant::Int(y)) => match kind {
            CmpKind::Eq => x == y,
            CmpKind::Ne => x != y,
            CmpKind::Lt => x < y,
            CmpKind::Le => x <= y,
            CmpKind::Gt => x > y,
            CmpKind::Ge => x >= y,
        },
        (Constant::UInt(x), Constant::UInt(y)) => match kind {
            CmpKind::Eq => x == y,
            CmpKind::Ne => x != y,
            CmpKind::Lt => x < y,
            CmpKind::Le => x <= y,
            CmpKind::Gt => x > y,
            CmpKind::Ge => x >= y,
        },
        (Constant::Float(x), Constant::Float(y)) => match kind {
            CmpKind::Eq => x == y,
            CmpKind::Ne => x != y,
            CmpKind::Lt => x < y,
            CmpKind::Le => x <= y,
            CmpKind::Gt => x > y,
            CmpKind::Ge => x >= y,
        },
        (Constant::String(x), Constant::String(y)) => match kind {
            CmpKind::Eq => x == y,
            CmpKind::Ne => x != y,
            CmpKind::Lt => x < y,
            CmpKind::Le => x <= y,
            CmpKind::Gt => x > y,
            CmpKind::Ge => x >= y,
        },
        _ => return None,
    };
    Some(Constant::Bool(result))
}

/// Try to fold a cast from a constant to a target type.
fn fold_cast(c: &Constant, ty: &Type) -> Option<Constant> {
    match (c, ty) {
        // Identity: same-type coercion is always a no-op.
        (Constant::Bool(_), Type::Bool) => Some(c.clone()),
        (Constant::String(_), Type::String) => Some(c.clone()),
        // Same-family: truncate/widen within the stored representation.
        (Constant::Int(x), Type::Int(bits)) => {
            let bits = *bits as u32;
            if bits >= 64 {
                Some(Constant::Int(*x))
            } else {
                // Truncate and sign-extend from bit (bits-1).
                let mask = (1i64 << bits) - 1;
                let truncated = *x & mask;
                let sign_bit = 1i64 << (bits - 1);
                if truncated & sign_bit != 0 {
                    Some(Constant::Int(truncated | !mask))
                } else {
                    Some(Constant::Int(truncated))
                }
            }
        }
        (Constant::UInt(x), Type::UInt(bits)) => {
            let bits = *bits as u32;
            if bits >= 64 {
                Some(Constant::UInt(*x))
            } else {
                Some(Constant::UInt(*x & ((1u64 << bits) - 1)))
            }
        }
        (Constant::Float(_), Type::Float(_)) => Some(c.clone()),
        // Cross-family conversions.
        (Constant::Int(x), Type::Float(_)) => Some(Constant::Float(*x as f64)),
        (Constant::Int(x), Type::UInt(_)) => Some(Constant::UInt(*x as u64)),
        (Constant::UInt(x), Type::Float(_)) => Some(Constant::Float(*x as f64)),
        (Constant::UInt(x), Type::Int(_)) => Some(Constant::Int(*x as i64)),
        (Constant::Float(x), Type::Int(_)) => Some(Constant::Int(*x as i64)),
        (Constant::Float(x), Type::UInt(_)) => Some(Constant::UInt(*x as u64)),
        // Conversions to/from Bool: non-zero is truthy (matches GML semantics).
        (Constant::Int(x), Type::Bool) => Some(Constant::Bool(*x != 0)),
        (Constant::UInt(x), Type::Bool) => Some(Constant::Bool(*x != 0)),
        (Constant::Float(x), Type::Bool) => Some(Constant::Bool(*x != 0.0 && !x.is_nan())),
        (Constant::Bool(b), Type::Int(_)) => Some(Constant::Int(*b as i64)),
        (Constant::Bool(b), Type::UInt(_)) => Some(Constant::UInt(*b as u64)),
        (Constant::Bool(b), Type::Float(_)) => Some(Constant::Float(*b as u8 as f64)),
        _ => None,
    }
}

/// Try to fold a single instruction given the current constant map.
/// Returns the folded constant if successful.
fn try_fold(
    op: &Op,
    consts: &HashMap<ValueId, Constant>,
    func_names: &HashMap<super::super::ir::func::FuncId, String>,
) -> Option<Constant> {
    match op {
        // Comparison
        Op::Cmp(kind, a, b) => fold_cmp(*kind, consts.get(a)?, consts.get(b)?),

        // Cast
        Op::Cast(a, ty, _) => fold_cast(consts.get(a)?, ty),

        // Builtin arithmetic/logic calls and type-conversion function calls.
        Op::Call { func: fid, args } => {
            let fname = func_names.get(fid).map(|s| s.as_str()).unwrap_or("");
            // Derive the base operation name (strip type suffix, e.g. "add_f64" → "add").
            let base = fname.rsplit_once('_').map(|(b, _)| b).unwrap_or(fname);

            match (base, args.as_slice()) {
                // Binary arithmetic builtins
                ("add", [a, b]) => fold_binary_arith("add", consts.get(a)?, consts.get(b)?),
                ("sub", [a, b]) => fold_binary_arith("sub", consts.get(a)?, consts.get(b)?),
                ("mul", [a, b]) => fold_binary_arith("mul", consts.get(a)?, consts.get(b)?),
                ("div", [a, b]) => fold_binary_arith("div", consts.get(a)?, consts.get(b)?),
                ("rem", [a, b]) => fold_binary_arith("rem", consts.get(a)?, consts.get(b)?),

                // Unary negation
                ("neg", [a]) => {
                    let c = consts.get(a)?;
                    match c {
                        Constant::Int(x) => Some(Constant::Int(x.wrapping_neg())),
                        Constant::UInt(x) => Some(Constant::Int(-(*x as i64))),
                        Constant::Float(x) => Some(Constant::Float(-x)),
                        _ => None,
                    }
                }

                // Binary bitwise builtins
                ("bitand", [a, b]) => {
                    if let (Some(ca), Some(cb)) = (consts.get(a), consts.get(b)) {
                        fold_binary_bitwise("and", ca, cb)
                            .or_else(|| fold_bitwise_absorbing("and", ca))
                            .or_else(|| fold_bitwise_absorbing("and", cb))
                    } else if let Some(ca) = consts.get(a) {
                        fold_bitwise_absorbing("and", ca)
                    } else if let Some(cb) = consts.get(b) {
                        fold_bitwise_absorbing("and", cb)
                    } else {
                        None
                    }
                }
                ("bitor", [a, b]) => {
                    if let (Some(ca), Some(cb)) = (consts.get(a), consts.get(b)) {
                        fold_binary_bitwise("or", ca, cb)
                            .or_else(|| fold_bitwise_absorbing("or", ca))
                            .or_else(|| fold_bitwise_absorbing("or", cb))
                    } else if let Some(ca) = consts.get(a) {
                        fold_bitwise_absorbing("or", ca)
                    } else if let Some(cb) = consts.get(b) {
                        fold_bitwise_absorbing("or", cb)
                    } else {
                        None
                    }
                }
                ("bitxor", [a, b]) => fold_binary_bitwise("xor", consts.get(a)?, consts.get(b)?),
                ("shl", [a, b]) => fold_binary_bitwise("shl", consts.get(a)?, consts.get(b)?),
                ("shr", [a, b]) => fold_binary_bitwise("shr", consts.get(a)?, consts.get(b)?),

                // Unary bitwise not
                ("bitnot", [a]) => {
                    let c = consts.get(a)?;
                    match c {
                        Constant::Int(x) => Some(Constant::Int(!x)),
                        Constant::UInt(x) => Some(Constant::UInt(!x)),
                        _ => None,
                    }
                }

                // Logical not — folds Bool, Int, UInt, Float, and String using JS truthiness.
                ("not", [a]) => match consts.get(a)? {
                    Constant::Bool(v) => Some(Constant::Bool(!v)),
                    Constant::Int(v) => Some(Constant::Bool(*v == 0)),
                    Constant::UInt(v) => Some(Constant::Bool(*v == 0)),
                    Constant::Float(v) => Some(Constant::Bool(*v == 0.0)),
                    Constant::String(s) => Some(Constant::Bool(s.is_empty())),
                    Constant::Null => Some(Constant::Bool(true)),
                },

                // Boolean AND/OR: short-circuit absorbing elements, then full fold.
                ("and", [a, b]) => match (consts.get(a), consts.get(b)) {
                    (Some(Constant::Bool(false)), _) | (_, Some(Constant::Bool(false))) => {
                        Some(Constant::Bool(false))
                    }
                    (Some(Constant::Bool(x)), Some(Constant::Bool(y))) => {
                        Some(Constant::Bool(*x && *y))
                    }
                    (Some(Constant::Bool(true)), _) | (_, Some(Constant::Bool(true))) => None,
                    _ => None,
                },
                ("or", [a, b]) => match (consts.get(a), consts.get(b)) {
                    (Some(Constant::Bool(true)), _) | (_, Some(Constant::Bool(true))) => {
                        Some(Constant::Bool(true))
                    }
                    (Some(Constant::Bool(x)), Some(Constant::Bool(y))) => {
                        Some(Constant::Bool(*x || *y))
                    }
                    (Some(Constant::Bool(false)), _) | (_, Some(Constant::Bool(false))) => None,
                    _ => None,
                },

                // Pure type-conversion function calls: int(x), uint(x), real(x), string(x)
                ("int", [a]) => {
                    let c = consts.get(a)?;
                    match c {
                        Constant::Int(x) => Some(Constant::Int(*x)),
                        Constant::UInt(x) => Some(Constant::Int(*x as i64)),
                        Constant::Float(x) => Some(Constant::Int(*x as i64)),
                        _ => None,
                    }
                }
                ("uint", [a]) => {
                    let c = consts.get(a)?;
                    match c {
                        Constant::Int(x) => Some(Constant::UInt(*x as u64)),
                        Constant::UInt(x) => Some(Constant::UInt(*x)),
                        Constant::Float(x) => Some(Constant::UInt(*x as u64)),
                        _ => None,
                    }
                }
                ("real", [a]) => {
                    let c = consts.get(a)?;
                    match c {
                        Constant::Int(x) => Some(Constant::Float(*x as f64)),
                        Constant::UInt(x) => Some(Constant::Float(*x as f64)),
                        Constant::Float(x) => Some(Constant::Float(*x)),
                        _ => None,
                    }
                }
                ("string", [a]) => {
                    let c = consts.get(a)?;
                    match c {
                        Constant::Int(x) => Some(Constant::String(x.to_string())),
                        Constant::UInt(x) => Some(Constant::String(x.to_string())),
                        Constant::Float(x) => Some(Constant::String(x.to_string())),
                        Constant::String(_) => Some(c.clone()),
                        Constant::Bool(b) => {
                            Some(Constant::String(if *b { "1" } else { "0" }.into()))
                        }
                        _ => None,
                    }
                }

                _ => None,
            }
        }

        _ => None,
    }
}

/// Peephole: rewrite `Not(Cmp(kind, a, b))` → `Cmp(inverse(kind), a, b)`.
///
/// This turns `!(x >= 1)` into `x < 1` at the IR level, so the emitter
/// doesn't need to handle comparison flipping as a syntax transformation.
fn fold_not_cmp(
    func: &mut Function,
    func_names: &HashMap<super::super::ir::func::FuncId, String>,
) -> bool {
    // Map from ValueId → (CmpKind, lhs, rhs) for all Cmp instructions.
    let mut cmp_defs: HashMap<ValueId, (CmpKind, ValueId, ValueId)> = HashMap::new();
    for (_, inst) in func.insts.iter() {
        if let (Op::Cmp(kind, a, b), Some(result)) = (&inst.op, inst.result) {
            cmp_defs.insert(result, (*kind, *a, *b));
        }
    }

    let updates: Vec<(InstId, CmpKind, ValueId, ValueId)> = func
        .insts
        .keys()
        .filter_map(|inst_id| {
            let inst = &func.insts[inst_id];
            // Match not_bool(inner) — the canonical not-of-cmp pattern.
            if let Op::Call { func: fid, args } = &inst.op {
                let fname = func_names.get(fid).map(|s| s.as_str()).unwrap_or("");
                if fname == "not_bool" && args.len() == 1 {
                    let inner = args[0];
                    let &(kind, a, b) = cmp_defs.get(&inner)?;
                    return Some((inst_id, kind.inverse(), a, b));
                }
            }
            None
        })
        .collect();

    let changed = !updates.is_empty();
    for (inst_id, inv_kind, a, b) in updates {
        func.insts[inst_id].op = Op::Cmp(inv_kind, a, b);
    }
    changed
}

/// Run constant folding on a single function. Returns true if any changes were made.
fn fold_function(
    func: &mut Function,
    func_names: &HashMap<super::super::ir::func::FuncId, String>,
) -> bool {
    let mut any_changed = false;

    loop {
        let consts = build_const_map(func);
        let mut changed = false;

        // Collect updates: (InstId, result ValueId, folded Constant)
        let updates: Vec<(InstId, ValueId, Constant)> = func
            .insts
            .keys()
            .filter_map(|inst_id| {
                let inst = &func.insts[inst_id];
                let result = inst.result?;
                // Skip instructions that are already constants.
                if matches!(&inst.op, Op::Const(_)) {
                    return None;
                }
                let folded = try_fold(&inst.op, &consts, func_names)?;
                Some((inst_id, result, folded))
            })
            .collect();

        for (inst_id, result, constant) in updates {
            let ty = constant.ty();
            func.insts[inst_id].op = Op::Const(constant);
            func.value_types[result] = ty;
            changed = true;
        }

        if !changed {
            break;
        }
        any_changed = true;
    }

    // Peephole: Not(Cmp) → Cmp(inverse). Runs after constant folding
    // since it doesn't create new folding opportunities.
    any_changed |= fold_not_cmp(func, func_names);

    // Fold BrIf with a constant condition → unconditional Br.
    any_changed |= fold_brif_constants(func);

    any_changed
}

/// JavaScript-style truthiness for IR constants.
///
/// Returns `Some(true)` if the constant is always truthy, `Some(false)` if
/// always falsy, or `None` if undetermined.
fn is_constant_truthy(c: &Constant) -> Option<bool> {
    match c {
        Constant::Bool(b) => Some(*b),
        Constant::Int(x) => Some(*x != 0),
        Constant::UInt(x) => Some(*x != 0),
        Constant::Float(x) => Some(*x != 0.0 && !x.is_nan()),
        Constant::String(s) => Some(!s.is_empty()),
        Constant::Null => Some(false),
    }
}

/// Fold `BrIf(const_cond, then, else)` → `Br(then)` or `Br(else)`.
///
/// Runs iteratively: each iteration rebuilds the const map (with reachability
/// filtering) so that blocks made unreachable by a previous fold don't prevent
/// subsequent block-param propagation.  The resulting dead branches are pruned
/// by DCE.
fn fold_brif_constants(func: &mut Function) -> bool {
    let mut any_changed = false;
    loop {
        let consts = build_const_map(func);

        let updates: Vec<(BlockId, Terminator)> = func
            .blocks
            .keys()
            .filter_map(|block_id| {
                if let Terminator::BrIf {
                    cond,
                    then_target,
                    then_args,
                    else_target,
                    else_args,
                } = &func.blocks[block_id].terminator
                {
                    let c = consts.get(cond)?;
                    let truthy = is_constant_truthy(c)?;
                    let new_term = if truthy {
                        Terminator::Br {
                            target: *then_target,
                            args: then_args.clone(),
                        }
                    } else {
                        Terminator::Br {
                            target: *else_target,
                            args: else_args.clone(),
                        }
                    };
                    Some((block_id, new_term))
                } else {
                    None
                }
            })
            .collect();

        if updates.is_empty() {
            break;
        }
        for (block_id, new_term) in updates {
            func.blocks[block_id].terminator = new_term;
        }
        any_changed = true;
    }
    any_changed
}

impl Transform for ConstantFolding {
    fn name(&self) -> &str {
        "constant-folding"
    }

    fn requires(&self) -> &[&str] {
        &["constraint-solve-hm"]
    }

    fn apply(
        &self,
        mut module: Module,
        dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        // Build FuncId → name map for const_fold's name-based dispatch.
        let func_names: HashMap<FuncId, String> = module
            .functions
            .keys()
            .map(|fid| (fid, module.func_name(fid).to_string()))
            .collect();

        let mut changed_funcs: HashSet<FuncId> = HashSet::new();
        for func_id in module.functions.keys().collect::<Vec<_>>() {
            if dirty.is_some_and(|d| !d.contains(&func_id)) {
                continue;
            }
            if fold_function(&mut module.functions[func_id], &func_names) {
                changed_funcs.insert(func_id);
            }
        }
        let changed = !changed_funcs.is_empty();
        Ok(TransformResult {
            module,
            changed,
            changed_funcs,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::ty::FunctionSig;
    use crate::ir::Visibility;

    fn apply_fold(func: crate::ir::Function) -> (crate::ir::Function, crate::ir::Module) {
        let mut mb = ModuleBuilder::new("test");
        let fid = mb.add_function(func);
        let module = mb.build();
        let result = ConstantFolding.apply(module, None).unwrap();
        let func = result.module.functions[fid].clone();
        (func, result.module)
    }

    /// Create a `FunctionBuilder` with the core builtin registry pre-installed,
    /// so arithmetic helpers (`add`, `sub`, `not`, etc.) can resolve their callees.
    fn fb_with_registry(name: &str, sig: FunctionSig) -> FunctionBuilder {
        let mb = ModuleBuilder::new("test");
        let registry = mb.runtime_registry().clone();
        let mut fb = FunctionBuilder::new(name, sig, Visibility::Private);
        fb.set_registry(registry);
        fb
    }

    /// Find the instruction that produces a given value.
    fn find_inst_for(func: &crate::ir::Function, value: ValueId) -> &crate::ir::Inst {
        func.insts
            .iter()
            .find(|(_, inst)| inst.result == Some(value))
            .map(|(_, inst)| inst)
            .expect("no instruction produces this value")
    }

    /// `2 + 3` folds to `Int(5)`.
    #[test]
    fn int_arithmetic() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.const_int(2, 64);
        let b = fb.const_int(3, 64);
        let sum = fb.add(a, b);
        fb.ret(Some(sum));

        let (func, _module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, sum).op,
            Op::Const(Constant::Int(5))
        ));
        assert_eq!(func.value_types[sum], Type::Int(64));
    }

    /// `1.5 * 2.0` folds to `Float(3.0)`.
    #[test]
    fn float_arithmetic() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.const_float(1.5);
        let b = fb.const_float(2.0);
        let product = fb.mul(a, b);
        fb.ret(Some(product));

        let (func, _module) = apply_fold(fb.build());
        assert!(
            matches!(&find_inst_for(&func, product).op, Op::Const(Constant::Float(f)) if *f == 3.0)
        );
    }

    /// `5 < 10` folds to `Bool(true)`.
    #[test]
    fn comparison() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.const_int(5, 64);
        let b = fb.const_int(10, 64);
        let cmp = fb.cmp(CmpKind::Lt, a, b);
        fb.ret(Some(cmp));

        let (func, _module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, cmp).op,
            Op::Const(Constant::Bool(true))
        ));
    }

    /// `not(true)` folds to `Bool(false)`.
    #[test]
    fn logical_not() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.const_bool(true);
        let result = fb.not(a);
        fb.ret(Some(result));

        let (func, _module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, result).op,
            Op::Const(Constant::Bool(false))
        ));
    }

    /// `5 / 0` stays unfolded (division by zero).
    #[test]
    fn division_by_zero_preserved() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.const_int(5, 64);
        let b = fb.const_int(0, 64);
        let div = fb.div(a, b);
        fb.ret(Some(div));

        let (func, module) = apply_fold(fb.build());
        let inst = find_inst_for(&func, div);
        assert!(
            matches!(&inst.op, Op::Call { func: fid, .. } if module.func_name(*fid).starts_with("div_")),
            "expected div builtin call, got {:?}",
            &inst.op
        );
    }

    /// `param + 3` stays unfolded (non-constant operand).
    #[test]
    fn non_constant_operand() {
        let sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let param = fb.param(0);
        let b = fb.const_int(3, 64);
        let sum = fb.add(param, b);
        fb.ret(Some(sum));

        let (func, module) = apply_fold(fb.build());
        let inst = find_inst_for(&func, sum);
        assert!(
            matches!(&inst.op, Op::Call { func: fid, .. } if module.func_name(*fid).starts_with("add_")),
            "expected add builtin call, got {:?}",
            &inst.op
        );
    }

    /// `neg(42)` folds to `Int(-42)`.
    #[test]
    fn negation() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.const_int(42, 64);
        let result = fb.neg(a);
        fb.ret(Some(result));

        let (func, _module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, result).op,
            Op::Const(Constant::Int(-42))
        ));
    }

    /// `Not(Cmp(Ge, param, 1))` folds to `Cmp(Lt, param, 1)`.
    #[test]
    fn not_cmp_folds_to_inverse() {
        let sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let param = fb.param(0);
        let one = fb.const_int(1, 64);
        let cmp = fb.cmp(CmpKind::Ge, param, one);
        let result = fb.not(cmp);
        fb.ret(Some(result));

        let (func, _module) = apply_fold(fb.build());
        let inst = find_inst_for(&func, result);
        assert!(
            matches!(&inst.op, Op::Cmp(CmpKind::Lt, a, b) if *a == param && *b == one),
            "expected Cmp(Lt, param, 1), got {:?}",
            inst.op
        );
    }

    // ---- Identity & idempotency tests ----

    /// No constants in arithmetic → nothing to fold, changed == false.
    #[test]
    fn identity_no_change() {
        let sig = FunctionSig {
            params: vec![Type::Int(64), Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.param(0);
        let b = fb.param(1);
        let sum = fb.add(a, b);
        fb.ret(Some(sum));

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        let result = ConstantFolding.apply(module, None).unwrap();
        assert!(!result.changed);
    }

    /// Folding is idempotent: second apply reports no change.
    #[test]
    fn idempotent_after_transform() {
        use crate::transforms::util::test_helpers::assert_idempotent;
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.const_int(2, 64);
        let b = fb.const_int(3, 64);
        let sum = fb.add(a, b);
        fb.ret(Some(sum));
        assert_idempotent(&ConstantFolding, fb.build());
    }

    /// `0xFF & 0x0F` folds to `Int(15)`.
    #[test]
    fn bitwise_and() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.const_int(0xFF, 64);
        let b = fb.const_int(0x0F, 64);
        let result = fb.bit_and(a, b);
        fb.ret(Some(result));

        let (func, _module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, result).op,
            Op::Const(Constant::Int(15))
        ));
    }

    /// `param && false` folds to `Bool(false)` (absorbing element).
    /// This covers GML `&&` patterns like `condition && isstaticok()` where
    /// `isstaticok()` always produces `Bool(false)`.
    #[test]
    fn bool_and_absorbing_false() {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let param = fb.param(0);
        let b = fb.const_bool(false);
        let result = fb.bool_and(param, b);
        fb.ret(Some(result));

        let (func, _module) = apply_fold(fb.build());
        assert!(
            matches!(
                &find_inst_for(&func, result).op,
                Op::Const(Constant::Bool(false))
            ),
            "expected BoolAnd(param, false) to fold to Bool(false)"
        );
    }

    /// `false && param` folds to `Bool(false)` (absorbing element, commutative).
    #[test]
    fn bool_and_absorbing_false_lhs() {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let param = fb.param(0);
        let a = fb.const_bool(false);
        let result = fb.bool_and(a, param);
        fb.ret(Some(result));

        let (func, _module) = apply_fold(fb.build());
        assert!(
            matches!(
                &find_inst_for(&func, result).op,
                Op::Const(Constant::Bool(false))
            ),
            "expected BoolAnd(false, param) to fold to Bool(false)"
        );
    }

    /// `param || true` folds to `Bool(true)` (absorbing element for OR).
    #[test]
    fn bool_or_absorbing_true() {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let param = fb.param(0);
        let b = fb.const_bool(true);
        let result = fb.bool_or(param, b);
        fb.ret(Some(result));

        let (func, _module) = apply_fold(fb.build());
        assert!(
            matches!(
                &find_inst_for(&func, result).op,
                Op::Const(Constant::Bool(true))
            ),
            "expected BoolOr(param, true) to fold to Bool(true)"
        );
    }

    // ---- Edge case tests ----

    /// Void function with no arithmetic — nothing to fold.
    #[test]
    fn void_function_no_fold() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Void,
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        fb.ret(None);

        let mut mb = ModuleBuilder::new("test");
        mb.add_function(fb.build());
        let module = mb.build();
        let result = ConstantFolding.apply(module, None).unwrap();
        assert!(!result.changed);
    }

    /// Constants in different blocks all fold.
    #[test]
    fn fold_in_multi_block() {
        let sig = FunctionSig {
            params: vec![Type::Bool],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let cond = fb.param(0);
        let then_block = fb.create_block();
        let else_block = fb.create_block();
        fb.br_if(cond, then_block, &[], else_block, &[]);

        fb.switch_to_block(then_block);
        let a = fb.const_int(2, 64);
        let b = fb.const_int(3, 64);
        let sum = fb.add(a, b);
        fb.ret(Some(sum));

        fb.switch_to_block(else_block);
        let c = fb.const_int(10, 64);
        let d = fb.const_int(20, 64);
        let product = fb.mul(c, d);
        fb.ret(Some(product));

        let (func, _module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, sum).op,
            Op::Const(Constant::Int(5))
        ));
        assert!(matches!(
            &find_inst_for(&func, product).op,
            Op::Const(Constant::Int(200))
        ));
    }

    /// Chained fold: `(1+2) * 3` folds to `9` in one pass via fixpoint.
    #[test]
    fn chained_fold() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.const_int(1, 64);
        let b = fb.const_int(2, 64);
        let sum = fb.add(a, b);
        let c = fb.const_int(3, 64);
        let product = fb.mul(sum, c);
        fb.ret(Some(product));

        let (func, _module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, product).op,
            Op::Const(Constant::Int(9))
        ));
    }

    // ---- Adversarial tests ----

    /// Same constant used in foldable and non-foldable contexts.
    #[test]
    fn shared_constant_folded_and_used() {
        let sig = FunctionSig {
            params: vec![Type::Int(64)],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let param = fb.param(0);
        let c = fb.const_int(5, 64);
        let fold_me = fb.add(c, c); // foldable: 5+5=10
        let keep_me = fb.add(param, c); // not foldable: param+5
        let result = fb.add(fold_me, keep_me);
        fb.ret(Some(result));

        let (func, module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, fold_me).op,
            Op::Const(Constant::Int(10))
        ));
        let keep_inst = find_inst_for(&func, keep_me);
        assert!(
            matches!(&keep_inst.op, Op::Call { func: fid, .. } if module.func_name(*fid).starts_with("add_")),
            "expected add builtin call, got {:?}",
            &keep_inst.op
        );
    }

    /// i64::MAX + 1 wraps correctly (wrapping_add).
    #[test]
    fn overflow_wraps() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let a = fb.const_int(i64::MAX, 64);
        let b = fb.const_int(1, 64);
        let sum = fb.add(a, b);
        fb.ret(Some(sum));

        let (func, _module) = apply_fold(fb.build());
        // i64::MAX + 1 wraps to i64::MIN.
        assert!(matches!(
            &find_inst_for(&func, sum).op,
            Op::Const(Constant::Int(v)) if *v == i64::MIN
        ));
    }

    /// NaN arithmetic: NaN + 1.0 should fold to NaN.
    #[test]
    fn float_nan_propagation() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let nan = fb.const_float(f64::NAN);
        let one = fb.const_float(1.0);
        let sum = fb.add(nan, one);
        fb.ret(Some(sum));

        let (func, _module) = apply_fold(fb.build());
        if let Op::Const(Constant::Float(v)) = &find_inst_for(&func, sum).op {
            assert!(v.is_nan(), "NaN + 1.0 should be NaN");
        } else {
            panic!("expected float constant");
        }
    }

    /// Deeply chained folds: 10-deep chain folds completely.
    #[test]
    fn deeply_chained_folds() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Int(64),
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let mut acc = fb.const_int(0, 64);
        for i in 1..=10 {
            let c = fb.const_int(i, 64);
            acc = fb.add(acc, c);
        }
        fb.ret(Some(acc));

        let (func, _module) = apply_fold(fb.build());
        // 0+1+2+...+10 = 55
        assert!(matches!(
            &find_inst_for(&func, acc).op,
            Op::Const(Constant::Int(55))
        ));
    }

    /// `Not(Int(16777215))` folds to `Bool(false)` (truthiness: non-zero → truthy → !truthy = false).
    #[test]
    fn not_int_folds_to_bool() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let c = fb.const_int(16777215, 64);
        let n = fb.not(c);
        fb.ret(Some(n));

        let (func, _module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, n).op,
            Op::Const(Constant::Bool(false))
        ));
    }

    /// `Not(Int(0))` folds to `Bool(true)` (0 is falsy).
    #[test]
    fn not_zero_folds_to_true() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let c = fb.const_int(0, 64);
        let n = fb.not(c);
        fb.ret(Some(n));

        let (func, _module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, n).op,
            Op::Const(Constant::Bool(true))
        ));
    }

    /// `Not(Not(Int(42)))` folds to `Bool(true)` (double negation of truthy).
    #[test]
    fn double_not_int_folds_to_bool() {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Bool,
            ..Default::default()
        };
        let mut fb = fb_with_registry("test", sig);
        let c = fb.const_int(42, 64);
        let n1 = fb.not(c);
        let n2 = fb.not(n1);
        fb.ret(Some(n2));

        let (func, _module) = apply_fold(fb.build());
        assert!(matches!(
            &find_inst_for(&func, n2).op,
            Op::Const(Constant::Bool(true))
        ));
    }
}
