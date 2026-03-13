use std::collections::{BTreeSet, HashMap, HashSet};

use datawin::bytecode::decode::Instruction;
use datawin::bytecode::opcode::Opcode;
use datawin::bytecode::types::{DataType, InstanceType};
use reincarnate_core::ir::block::BlockId;
use reincarnate_core::ir::builder::FunctionBuilder;
use reincarnate_core::ir::ty::Type;
use reincarnate_core::ir::value::ValueId;

use super::{is_2d_array_access, is_cross_obj_2d_read, is_next_stacktop_access, is_stacktop_ref};

/// Filter instructions to only those reachable from the entry point.
///
/// In GMS2.3+ shared bytecode blobs, the decoded byte range may extend
/// past this function's terminal instruction into sibling functions' code.
/// This function walks the control flow from instruction 0, following
/// branches and fall-through, stopping at Ret/Exit. Only reachable
/// instructions are returned.
pub(super) fn filter_reachable(instructions: &[Instruction]) -> Vec<Instruction> {
    let offset_to_idx: HashMap<usize, usize> = instructions
        .iter()
        .enumerate()
        .map(|(i, inst)| (inst.offset, i))
        .collect();

    let mut visited = vec![false; instructions.len()];
    let mut worklist = vec![0usize]; // start at instruction index 0

    while let Some(idx) = worklist.pop() {
        if idx >= instructions.len() || visited[idx] {
            continue;
        }
        visited[idx] = true;
        let inst = &instructions[idx];

        match inst.opcode {
            Opcode::B => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    if let Some(&ti) = offset_to_idx.get(&target) {
                        worklist.push(ti);
                    }
                }
            }
            Opcode::Bt | Opcode::Bf | Opcode::PushEnv | Opcode::PopEnv => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    if let Some(&ti) = offset_to_idx.get(&target) {
                        worklist.push(ti);
                    }
                }
                worklist.push(idx + 1);
            }
            Opcode::Ret | Opcode::Exit => {
                // Terminal — don't follow.
            }
            _ => {
                worklist.push(idx + 1);
            }
        }
    }

    instructions
        .iter()
        .enumerate()
        .filter(|(i, _)| visited[*i])
        .map(|(_, inst)| inst.clone())
        .collect()
}

/// Pass 1: Identify basic block start offsets.
pub(super) fn find_block_starts(instructions: &[Instruction]) -> BTreeSet<usize> {
    let mut starts = BTreeSet::new();
    starts.insert(0);

    for (i, inst) in instructions.iter().enumerate() {
        match inst.opcode {
            Opcode::B | Opcode::Bt | Opcode::Bf | Opcode::PushEnv | Opcode::PopEnv => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    starts.insert(target);
                }
                // Fall-through for conditional branches.
                if matches!(
                    inst.opcode,
                    Opcode::Bt | Opcode::Bf | Opcode::PushEnv | Opcode::PopEnv
                ) {
                    if let Some(next) = instructions.get(i + 1) {
                        starts.insert(next.offset);
                    }
                }
                // Unconditional branch: next instruction is a block start too
                // (it might be a jump target from elsewhere).
                if inst.opcode == Opcode::B {
                    if let Some(next) = instructions.get(i + 1) {
                        starts.insert(next.offset);
                    }
                }
            }
            Opcode::Ret | Opcode::Exit => {
                // Don't create a block start after Ret/Exit. If the code
                // after is reachable (e.g., after a conditional early return),
                // some branch already targets it and that branch creates the
                // block start. Adding one here would cause spurious blocks
                // from trailing bytecode of sibling functions in GMS2.3+
                // shared bytecode blobs.
            }
            _ => {}
        }
    }

    starts
}

/// Create IR blocks for a set of instructions, skipping with-body offsets.
///
/// Returns `(block_map, block_params, block_entry_depths)`.
///
/// - `entry_offset`: the bytecode offset that maps to the entry block (0 for
///   outer functions, the first body instruction's offset for inner closures).
/// - With-body offsets (inside any PushEnv/PopEnv range in `with_ranges`) are
///   excluded from block creation — they will be handled by separate closure
///   functions and must not appear as dead blocks in the outer function's CFG.
#[allow(clippy::type_complexity)]
pub(super) fn setup_blocks(
    fb: &mut FunctionBuilder,
    instructions: &[Instruction],
    with_ranges: &HashMap<usize, usize>,
    entry_offset: usize,
) -> (
    HashMap<usize, BlockId>,
    HashMap<usize, Vec<ValueId>>,
    HashMap<usize, usize>,
) {
    let block_starts = find_block_starts(instructions);
    let block_entry_depths = compute_block_stack_depths(instructions, &block_starts);

    // Collect offsets that belong to with-body ranges (body + PopEnv).
    // Blocks at these offsets are owned by extracted closures, not the outer function.
    let body_offsets: HashSet<usize> = with_ranges
        .iter()
        .flat_map(|(&pi, &popi)| instructions[pi + 1..=popi].iter().map(|i| i.offset))
        .collect();

    let mut block_map: HashMap<usize, BlockId> = HashMap::new();
    let mut block_params: HashMap<usize, Vec<ValueId>> = HashMap::new();
    block_map.insert(entry_offset, fb.entry_block());
    for &off in &block_starts {
        if off != entry_offset && !body_offsets.contains(&off) {
            let block = fb.create_block();
            block_map.insert(off, block);
            let depth = block_entry_depths.get(&off).copied().unwrap_or(0);
            if depth > 0 {
                let types: Vec<Type> = vec![Type::Dynamic; depth];
                let params = fb.add_block_params(block, &types);
                block_params.insert(off, params);
            }
        }
    }
    (block_map, block_params, block_entry_depths)
}

/// Return the GML stack slot size of a DataType in 4-byte units.
///
/// The GML VM stack uses variable-width slots:
///   - Int16, Int32, Boolean, String: 4 bytes (1 unit)
///   - Double, Int64: 8 bytes (2 units)
///   - Variable (RValue): 16 bytes (4 units)
///
/// This matters for `Dup(N)` which duplicates `(N+1) * sizeof(type1)` bytes,
/// not `N+1` items.
pub(super) fn gml_slot_units(dt: DataType) -> u8 {
    match dt {
        DataType::Variable => 4,
        DataType::Double | DataType::Int64 => 2,
        _ => 1, // Int16, Int32, Boolean, String
    }
}

/// Compute the stack effect (pops, pushes) of an instruction.
/// `rest` is the slice of instructions *following* `inst` in the same
/// function body, used by `is_cross_obj_2d_read` to detect read vs write
/// context via lookahead.
pub(super) fn stack_effect(inst: &Instruction, rest: &[Instruction]) -> (usize, usize) {
    match inst.opcode {
        Opcode::PushI | Opcode::Push | Opcode::PushLoc | Opcode::PushGlb | Opcode::PushBltn => {
            if let datawin::bytecode::decode::Operand::Variable { var_ref, instance } =
                &inst.operand
            {
                if matches!(
                    InstanceType::from_i16(*instance),
                    Some(InstanceType::Stacktop)
                ) || is_stacktop_ref(var_ref, *instance)
                {
                    (1, 1) // pops instance from stack, pushes field value
                } else if is_2d_array_access(var_ref, *instance)
                    || is_cross_obj_2d_read(var_ref, *instance, rest)
                {
                    (2, 1) // pops dim1+dim2, pushes value
                } else {
                    (0, 1)
                }
            } else if matches!(inst.operand, datawin::bytecode::decode::Operand::Int16(-9))
                && is_next_stacktop_access(rest)
            {
                // PushI -9 sentinel before stacktop access — skipped at
                // translation time for cross-object, net-zero for self-access.
                (0, 0)
            } else {
                (0, 1)
            }
        }
        Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::Div | Opcode::Rem | Opcode::Mod => (2, 1),
        Opcode::Neg | Opcode::Not => (1, 1),
        Opcode::And | Opcode::Or | Opcode::Xor | Opcode::Shl | Opcode::Shr => (2, 1),
        Opcode::Cmp => (2, 1),
        Opcode::Conv => (1, 1),
        Opcode::Dup => {
            // Dup(N): high byte is DupExtra (GMS2.3+ extended flag), low byte is dup_size.
            // DupExtra != 0 → GMS2.3+ extended encoding (no-op for our IR, no net stack change).
            // DupExtra == 0 → normal dup; approximated as pushing dup_size+1 items.
            if let datawin::bytecode::decode::Operand::Dup(n) = inst.operand {
                let dup_extra = (n >> 8) & 0xFF;
                let dup_size = n & 0xFF;
                if dup_extra != 0 {
                    (0, 0) // swap or no-op: no net item change
                } else {
                    (0, dup_size as usize + 1)
                }
            } else {
                (0, 1)
            }
        }
        Opcode::Popz => (1, 0),
        Opcode::Pop => {
            if let datawin::bytecode::decode::Operand::Variable { var_ref, instance } =
                &inst.operand
            {
                if matches!(
                    InstanceType::from_i16(*instance),
                    Some(InstanceType::Stacktop)
                ) || is_stacktop_ref(var_ref, *instance)
                {
                    (2, 0) // pops value + instance from stack
                } else if is_2d_array_access(var_ref, *instance) {
                    (3, 0) // pops value + 2D indices
                } else {
                    (1, 0)
                }
            } else {
                (1, 0)
            }
        }
        Opcode::Call => {
            if let datawin::bytecode::decode::Operand::Call { argc, .. } = inst.operand {
                (argc as usize, 1)
            } else {
                (0, 1)
            }
        }
        Opcode::CallV => {
            // CallV pops: function ref + instance + argc args
            if let datawin::bytecode::decode::Operand::Call { argc, .. } = inst.operand {
                (argc as usize + 2, 1)
            } else {
                (2, 1)
            }
        }
        Opcode::Ret => (1, 0),
        Opcode::Exit => (0, 0),
        Opcode::B => (0, 0),
        Opcode::Bt | Opcode::Bf => (1, 0),
        Opcode::PushEnv => (1, 0),
        Opcode::PopEnv => (0, 0),
        Opcode::Break => {
            if let datawin::bytecode::decode::Operand::Break { signal, .. } = inst.operand {
                match signal {
                    0xFFFF => (0, 0),          // chkindex
                    0xFFFC => (1, 0),          // pushac — captures array ref (pops 1)
                    0xFFFB => (1, 0),          // setowner — pops owner ID
                    0xFFFE => (2, 1),          // pushaf
                    0xFFFD => (2, 0),          // popaf — pops value + index (array from pushac)
                    0xFFF6 => (0, 1),          // chknullish — pushes boolean
                    0xFFF5 => (0, 1),          // pushref — pushes function ref
                    0xFFFA => (0, 1),          // isstaticok — pushes boolean
                    0xFFF9 => (0, 0),          // setstatic — nop
                    0xFFF8 | 0xFFF7 => (0, 0), // savearef, restorearef — nop
                    _ => (0, 0),
                }
            } else {
                (0, 0)
            }
        }
    }
}

/// Pre-compute the operand stack depth at each block entry point.
fn record_depth(depths: &mut HashMap<usize, usize>, offset: usize, depth: usize) {
    // Use min across all predecessor paths — if paths disagree on depth, only the
    // values present on every path can be safely passed as block args.
    depths
        .entry(offset)
        .and_modify(|v| *v = (*v).min(depth))
        .or_insert(depth);
}

pub(super) fn compute_block_stack_depths(
    instructions: &[Instruction],
    block_starts: &BTreeSet<usize>,
) -> HashMap<usize, usize> {
    let mut depths: HashMap<usize, usize> = HashMap::new();
    depths.insert(0, 0);

    let mut depth: i32 = 0;
    let mut terminated = false;

    for (i, inst) in instructions.iter().enumerate() {
        if block_starts.contains(&inst.offset) && i > 0 {
            if !terminated {
                record_depth(&mut depths, inst.offset, depth as usize);
            }
            if let Some(&d) = depths.get(&inst.offset) {
                depth = d as i32;
                terminated = false;
            } else {
                // Unreachable block (no incoming edge recorded a depth).
                // Don't process instructions or propagate depths from here.
                depth = 0;
                terminated = true;
            }
        }

        if terminated {
            continue;
        }

        let (pops, pushes) = stack_effect(inst, &instructions[i + 1..]);
        depth -= pops as i32;
        if depth < 0 {
            depth = 0;
        }

        match inst.opcode {
            Opcode::B => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    record_depth(&mut depths, target, depth as usize);
                }
                terminated = true;
            }
            Opcode::Bt | Opcode::Bf => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    record_depth(&mut depths, target, depth as usize);
                    if let Some(next) = instructions.get(i + 1) {
                        record_depth(&mut depths, next.offset, depth as usize);
                    }
                }
                terminated = true;
            }
            Opcode::PushEnv => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    record_depth(&mut depths, target, depth as usize);
                    if let Some(next) = instructions.get(i + 1) {
                        record_depth(&mut depths, next.offset, depth as usize);
                    }
                }
                terminated = true;
            }
            Opcode::PopEnv => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    record_depth(&mut depths, target, depth as usize);
                    if let Some(next) = instructions.get(i + 1) {
                        record_depth(&mut depths, next.offset, depth as usize);
                    }
                }
                terminated = true;
            }
            Opcode::Ret | Opcode::Exit => {
                terminated = true;
            }
            _ => {}
        }

        depth += pushes as i32;
    }

    depths
}

/// Build branch arguments from the current stack based on target block's entry depth.
pub(super) fn get_branch_args(stack: &[ValueId], target_depth: usize) -> Vec<ValueId> {
    stack.iter().take(target_depth).copied().collect()
}
