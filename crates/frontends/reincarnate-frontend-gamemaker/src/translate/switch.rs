use std::collections::HashSet;

use reincarnate_core::entity::EntityRef;
use reincarnate_core::ir::block::BlockId;
use reincarnate_core::ir::func::Function;
use reincarnate_core::ir::inst::{CmpKind, Op, Terminator};
use reincarnate_core::ir::value::{Constant, ValueId};

/// Detect BrIf chains that represent switch statements and rewrite them
/// as `Op::Switch`. GML bytecode compiles switch statements as a chain of
/// Dup+Cmp(Eq)+Bf, producing the following IR pattern:
///
/// ```text
/// block0(..., switch_val):
///   copy_v = Copy(switch_val)
///   case_const = Const(42)
///   cmp = Cmp(Eq, copy_v, case_const)
///   BrIf cmp, case_body[...], next_block[switch_val]
/// ```
///
/// Each block in the chain tests one case. The switch value is threaded
/// through block parameters. The chain ends with a `Br` to the default body.
pub(super) fn detect_switches(func: &mut Function) {
    let num_blocks = func.blocks.len();
    let mut consumed = HashSet::new();

    for block_idx in 0..num_blocks {
        let block_id = BlockId::new(block_idx as u32);
        if consumed.contains(&block_id) {
            continue;
        }

        // Try to extract a switch chain starting at this block.
        if let Some(chain) = extract_switch_chain(func, block_id) {
            if chain.cases.len() < 2 {
                continue;
            }
            // Mark intermediate blocks as consumed.
            for &mid in &chain.intermediate_blocks {
                consumed.insert(mid);
            }
            // Rewrite the first block's terminator to Op::Switch.
            rewrite_to_switch(func, block_id, &chain);
        }
    }
}

/// A detected switch chain.
struct SwitchChain {
    /// The original switch value in the first block.
    switch_value: ValueId,
    /// Collected cases: (constant, target_block, target_args).
    cases: Vec<(Constant, BlockId, Vec<ValueId>)>,
    /// Default target and args (from the final Br).
    default: (BlockId, Vec<ValueId>),
    /// Intermediate comparison blocks (to be cleared).
    intermediate_blocks: Vec<BlockId>,
    /// Instruction IDs to remove from the first block (Copy, Const, Cmp).
    first_block_remove_insts: Vec<reincarnate_core::ir::inst::InstId>,
}

/// Try to extract a switch chain starting from `block_id`.
fn extract_switch_chain(func: &Function, block_id: BlockId) -> Option<SwitchChain> {
    let (switch_value, case_const, case_target, case_args, next_block, next_args, remove_insts) =
        match_switch_block(func, block_id, None)?;

    let mut cases = vec![(case_const, case_target, case_args)];
    let mut intermediate = Vec::new();
    let mut current = next_block;
    // `incoming_args` are the args passed into `current` from the previous block.
    // They always reference values in the first (non-cleared) block, so substituting
    // intermediate block params with them keeps all ValueIds valid after clearing.
    let mut incoming_args = next_args;

    // The switch value is passed to the next block via args. Find which param
    // position it maps to.
    let param_idx = if incoming_args.len() == 1 {
        0
    } else {
        incoming_args.iter().position(|a| *a == switch_value)?
    };

    loop {
        // The switch value in the next block is its block parameter at param_idx.
        let next_block_data = &func.blocks[current];
        if param_idx >= next_block_data.params.len() {
            return None;
        }
        let next_switch_val = next_block_data.params[param_idx].value;

        // Build a substitution closure: map each param of `current` to the
        // corresponding value from `incoming_args`. This is required because
        // `current` will be cleared by `rewrite_to_switch`, so any ValueId
        // that references one of its params would become undefined.
        let block_params: Vec<ValueId> = next_block_data.params.iter().map(|p| p.value).collect();
        let subst = |v: ValueId| -> ValueId {
            block_params
                .iter()
                .position(|&p| p == v)
                .and_then(|pos| incoming_args.get(pos).copied())
                .unwrap_or(v)
        };

        if let Some((_, case_const, case_target, case_args, next, next_incoming, _)) =
            match_switch_block(func, current, Some(next_switch_val))
        {
            // Remap case_args and next_incoming through the substitution so that
            // after `current` is cleared, all ValueIds trace back to the first block.
            let mapped_args: Vec<ValueId> = case_args.iter().map(|&v| subst(v)).collect();
            let mapped_next_incoming: Vec<ValueId> =
                next_incoming.iter().map(|&v| subst(v)).collect();
            cases.push((case_const, case_target, mapped_args));
            intermediate.push(current);
            incoming_args = mapped_next_incoming;
            current = next;
        } else {
            // Check if this block is the default (just a Br).
            let (def_target, def_args) = match_default_block(func, current)?;
            let mapped_def_args: Vec<ValueId> = def_args.iter().map(|&v| subst(v)).collect();
            intermediate.push(current);
            return Some(SwitchChain {
                switch_value,
                cases,
                default: (def_target, mapped_def_args),
                intermediate_blocks: intermediate,
                first_block_remove_insts: remove_insts,
            });
        }
    }
}

/// A single case match result from a switch chain block.
type SwitchBlockMatch = (
    ValueId,                                 // switch_value
    Constant,                                // case_constant
    BlockId,                                 // case_target
    Vec<ValueId>,                            // case_args
    BlockId,                                 // else_target
    Vec<ValueId>,                            // else_args
    Vec<reincarnate_core::ir::inst::InstId>, // insts_to_remove
);

/// Match a single block in the switch chain.
fn match_switch_block(
    func: &Function,
    block_id: BlockId,
    expected_switch_val: Option<ValueId>,
) -> Option<SwitchBlockMatch> {
    let block = &func.blocks[block_id];

    // The terminator must be a BrIf.
    let (cond_val, then_target, then_args, else_target, else_args) = match &block.terminator {
        Terminator::BrIf {
            cond,
            then_target,
            then_args,
            else_target,
            else_args,
        } => (
            *cond,
            *then_target,
            then_args.clone(),
            *else_target,
            else_args.clone(),
        ),
        _ => return None,
    };

    // The condition must be a Cmp(Eq, lhs, rhs) where one operand is the
    // switch value (or a Copy of it) and the other is a Const.
    let cond_inst_id = find_def_inst(func, block_id, cond_val)?;
    let cond_inst = &func.insts[cond_inst_id];
    let (cmp_lhs, cmp_rhs) = match &cond_inst.op {
        Op::Cmp(CmpKind::Eq, lhs, rhs) => (*lhs, *rhs),
        _ => return None,
    };

    // One of lhs/rhs should be a Const, the other the switch value (possibly via Copy).
    let (switch_operand, case_const) = {
        let lhs_const = find_const(func, block_id, cmp_lhs);
        let rhs_const = find_const(func, block_id, cmp_rhs);
        match (lhs_const, rhs_const) {
            (None, Some(c)) => (cmp_lhs, c),
            (Some(c), None) => (cmp_rhs, c),
            _ => return None,
        }
    };

    // Resolve through Copy to find the actual switch value.
    let switch_val = resolve_through_copy(func, block_id, switch_operand);

    // If we have an expected switch value, verify it matches.
    if let Some(expected) = expected_switch_val {
        if switch_val != expected {
            return None;
        }
    }

    // Collect instruction IDs to remove from this block (Const, Copy, Cmp).
    let mut remove = Vec::new();
    // Only collect remove_insts for the first block; intermediate blocks
    // will be cleared entirely.
    if expected_switch_val.is_none() {
        if let Some(id) = find_def_inst(func, block_id, switch_operand) {
            if matches!(func.insts[id].op, Op::Copy(_)) {
                remove.push(id);
            }
        }
        // Find the Const instruction.
        let const_val = if find_const(func, block_id, cmp_lhs).is_some() {
            cmp_lhs
        } else {
            cmp_rhs
        };
        if let Some(id) = find_def_inst(func, block_id, const_val) {
            remove.push(id);
        }
        remove.push(cond_inst_id);
    }

    // GML Bf swaps then/else: then=fallthrough (next case), else=case body.
    // But in our IR, the BrIf condition is true → then_target.
    // With Cmp(Eq), true means "matched", so then_target is the case body
    // and else_target is the next comparison block.
    Some((
        switch_val,
        case_const,
        then_target,
        then_args,
        else_target,
        else_args,
        remove,
    ))
}

/// Match a default block (just a Br terminator, possibly with block-arg assigns).
fn match_default_block(func: &Function, block_id: BlockId) -> Option<(BlockId, Vec<ValueId>)> {
    let block = &func.blocks[block_id];
    // The block's terminator should be Br.
    match &block.terminator {
        Terminator::Br { target, args } => Some((*target, args.clone())),
        _ => None,
    }
}

/// Find the instruction in `block_id` that defines `value`.
fn find_def_inst(
    func: &Function,
    block_id: BlockId,
    value: ValueId,
) -> Option<reincarnate_core::ir::inst::InstId> {
    func.blocks[block_id]
        .insts
        .iter()
        .find(|&&inst_id| func.insts[inst_id].result == Some(value))
        .copied()
}

/// If `value` is defined by Op::Const in `block_id`, return the constant.
fn find_const(func: &Function, block_id: BlockId, value: ValueId) -> Option<Constant> {
    let inst_id = find_def_inst(func, block_id, value)?;
    match &func.insts[inst_id].op {
        Op::Const(c) => Some(c.clone()),
        _ => None,
    }
}

/// Resolve through Copy instructions: if `value` is defined by Copy(src),
/// return src; otherwise return value as-is.
fn resolve_through_copy(func: &Function, block_id: BlockId, value: ValueId) -> ValueId {
    if let Some(inst_id) = find_def_inst(func, block_id, value) {
        if let Op::Copy(src) = &func.insts[inst_id].op {
            return *src;
        }
    }
    value
}

/// Rewrite a block's terminator from BrIf to Op::Switch.
fn rewrite_to_switch(func: &mut Function, block_id: BlockId, chain: &SwitchChain) {
    // Remove the Copy, Const, and Cmp instructions from the first block.
    let remove_set: HashSet<_> = chain.first_block_remove_insts.iter().copied().collect();
    func.blocks[block_id]
        .insts
        .retain(|id| !remove_set.contains(id));

    // Replace the block's BrIf terminator with Switch.
    func.blocks[block_id].terminator = Terminator::Switch {
        value: chain.switch_value,
        cases: chain.cases.clone(),
        default: chain.default.clone(),
    };

    // Clear intermediate blocks (they're now dead).
    for &mid in &chain.intermediate_blocks {
        func.blocks[mid].insts.clear();
        func.blocks[mid].params.clear();
    }
}
