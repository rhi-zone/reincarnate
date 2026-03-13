use std::collections::{BTreeSet, HashMap, HashSet};

use datawin::bytecode::decode::Instruction;
use datawin::bytecode::opcode::Opcode;
use datawin::bytecode::types::{DataType, InstanceType};
use reincarnate_core::ir::block::BlockId;
use reincarnate_core::ir::builder::FunctionBuilder;
use reincarnate_core::ir::ty::Type;
use reincarnate_core::ir::value::ValueId;

use datawin::bytecode::decode::Operand;

use super::{
    is_2d_array_access, is_cross_obj_2d_read, is_next_stacktop_ref_access, is_stacktop_ref,
};

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
    function_names: &HashMap<u32, String>,
    bytecode_offset: usize,
    func_ref_map: &HashMap<usize, usize>,
) -> (
    HashMap<usize, BlockId>,
    HashMap<usize, Vec<ValueId>>,
    HashMap<usize, usize>,
) {
    let block_starts = find_block_starts(instructions);
    let block_entry_depths = compute_block_stack_depths(
        instructions,
        &block_starts,
        function_names,
        bytecode_offset,
        func_ref_map,
    );

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

/// Pre-compute the operand stack depth at each block entry point.
fn record_depth(depths: &mut HashMap<usize, usize>, offset: usize, depth: usize) {
    // Use min across all predecessor paths — if paths disagree on depth, only the
    // values present on every path can be safely passed as block args.
    depths
        .entry(offset)
        .and_modify(|v| *v = (*v).min(depth))
        .or_insert(depth);
}

/// Pop `n` items from the width stack, clamping to avoid underflow.
fn ws_pop(ws: &mut Vec<u8>, n: usize) {
    ws.truncate(ws.len().saturating_sub(n));
}

/// Compute block entry depths using a width-aware stack simulation.
///
/// The GML VM stack uses variable-width slots (1/2/4 units per item depending
/// on DataType). Most opcodes have fixed item-count effects, but `Dup(N)`
/// duplicates `(N+1) * gml_slot_units(type1)` *bytes* — the actual number of
/// items depends on the byte widths of items already on the stack. This
/// function tracks a `Vec<u8>` of per-item slot widths so that Dup computes
/// the same item count as the actual translator in `translate_stack_op`.
pub(super) fn compute_block_stack_depths(
    instructions: &[Instruction],
    block_starts: &BTreeSet<usize>,
    function_names: &HashMap<u32, String>,
    bytecode_offset: usize,
    func_ref_map: &HashMap<usize, usize>,
) -> HashMap<usize, usize> {
    let mut depths: HashMap<usize, usize> = HashMap::new();
    depths.insert(0, 0);

    // Width stack: each entry is the gml_slot_units of that stack item.
    let mut ws: Vec<u8> = Vec::new();
    let mut terminated = false;
    // Track pushac state: when pushac captures an array index, the
    // subsequent popaf pops only 2 items (value + array) instead of 3.
    let mut pushac_pending = false;
    // Track @@Global@@ scope: Call @@Global@@ sets this; PushI -9
    // sentinel is skipped unconditionally when this flag is set.
    let mut global_scope_on_stack = false;

    for (i, inst) in instructions.iter().enumerate() {
        if block_starts.contains(&inst.offset) && i > 0 {
            if !terminated {
                record_depth(&mut depths, inst.offset, ws.len());
            }
            if let Some(&d) = depths.get(&inst.offset) {
                // Reconstruct: block params are Variable-width (4 units each)
                ws = vec![4u8; d];
                terminated = false;
                pushac_pending = false;
                global_scope_on_stack = false;
            } else {
                // Unreachable block (no incoming edge recorded a depth).
                ws.clear();
                terminated = true;
                pushac_pending = false;
                global_scope_on_stack = false;
            }
        }

        if terminated {
            continue;
        }

        let rest = &instructions[i + 1..];

        // Apply stack effect with per-item width tracking.
        // For branch/terminator instructions only pops are applied here;
        // depths are recorded at targets below (pushes are always 0 for those).
        match inst.opcode {
            Opcode::PushI | Opcode::Push | Opcode::PushLoc | Opcode::PushGlb | Opcode::PushBltn => {
                if let datawin::bytecode::decode::Operand::Variable {
                    var_ref, instance, ..
                } = &inst.operand
                {
                    if matches!(
                        InstanceType::from_i16(*instance),
                        Some(InstanceType::Stacktop)
                    ) || is_stacktop_ref(var_ref, *instance)
                    {
                        ws_pop(&mut ws, 1); // pop instance
                        ws.push(4); // push Variable-width result
                    } else if is_2d_array_access(var_ref, *instance)
                        || is_cross_obj_2d_read(var_ref, *instance, rest)
                    {
                        ws_pop(&mut ws, 2); // pop dim1+dim2
                        ws.push(4); // push Variable-width result
                    } else {
                        ws.push(4); // variable access → Variable (4 units)
                    }
                } else if matches!(inst.operand, Operand::Int16(-9))
                    && i > 0
                    && matches!(
                        instructions[i - 1].opcode,
                        Opcode::PushLoc
                            | Opcode::PushGlb
                            | Opcode::Push
                            | Opcode::PushBltn
                            | Opcode::PushI
                            | Opcode::Call
                            | Opcode::CallV
                            | Opcode::Break
                    )
                    && (is_next_stacktop_ref_access(rest) || global_scope_on_stack)
                {
                    // PushI -9 sentinel — skipped (net zero).
                    // Matches translation logic: skip when previous instruction is a
                    // push/call/break AND next is a stacktop ref access or @@Global@@.
                    global_scope_on_stack = false;
                } else {
                    ws.push(gml_slot_units(inst.type1));
                }
            }

            Opcode::Add | Opcode::Sub | Opcode::Mul | Opcode::Div | Opcode::Rem | Opcode::Mod => {
                ws_pop(&mut ws, 2);
                ws.push(gml_slot_units(inst.type1).max(gml_slot_units(inst.type2)));
            }

            Opcode::Neg | Opcode::Not => {
                ws_pop(&mut ws, 1);
                ws.push(gml_slot_units(inst.type1).max(gml_slot_units(inst.type2)));
            }

            Opcode::And | Opcode::Or | Opcode::Xor | Opcode::Shl | Opcode::Shr | Opcode::Cmp => {
                ws_pop(&mut ws, 2);
                ws.push(gml_slot_units(inst.type1).max(gml_slot_units(inst.type2)));
            }

            Opcode::Conv => {
                ws_pop(&mut ws, 1);
                ws.push(gml_slot_units(inst.type2));
            }

            Opcode::Dup => {
                if let datawin::bytecode::decode::Operand::Dup(n) = inst.operand {
                    let dup_extra = (n >> 8) & 0xFF;
                    let dup_size = (n & 0xFF) as usize;
                    if dup_extra == 0 {
                        // Normal dup: duplicate (dup_size+1) * type_unit bytes.
                        // Walk backwards by per-item widths to find actual item count,
                        // matching the logic in translate_stack_op (ops.rs).
                        let type_unit = gml_slot_units(inst.type1) as usize;
                        let total_units = (dup_size + 1) * type_unit;

                        let mut units_remaining = total_units;
                        let mut item_count = 0;
                        for &w in ws.iter().rev() {
                            if units_remaining == 0 {
                                break;
                            }
                            let item_units = w as usize;
                            if item_units > units_remaining {
                                item_count += 1;
                                break;
                            }
                            units_remaining -= item_units;
                            item_count += 1;
                        }

                        // Duplicate the widths of the top item_count items.
                        let start = ws.len().saturating_sub(item_count);
                        let duped: Vec<u8> = ws[start..].to_vec();
                        ws.extend(duped);
                    }
                    // dup_extra != 0 → GMS2.3+ swap/no-op, no net stack change
                }
            }

            Opcode::Popz => {
                ws_pop(&mut ws, 1);
            }

            Opcode::Pop => {
                if let datawin::bytecode::decode::Operand::Variable {
                    var_ref, instance, ..
                } = &inst.operand
                {
                    if matches!(
                        InstanceType::from_i16(*instance),
                        Some(InstanceType::Stacktop)
                    ) || is_stacktop_ref(var_ref, *instance)
                    {
                        ws_pop(&mut ws, 2); // value + instance
                    } else if is_2d_array_access(var_ref, *instance) {
                        ws_pop(&mut ws, 3); // value + 2D indices
                    } else {
                        ws_pop(&mut ws, 1);
                    }
                } else {
                    ws_pop(&mut ws, 1);
                }
            }

            Opcode::Call => {
                if let Operand::Call {
                    function_id, argc, ..
                } = inst.operand
                {
                    ws_pop(&mut ws, argc as usize);
                    // Detect Call @@Global@@ (argc=0) to set global_scope_on_stack.
                    // Use func_ref_map (absolute address → FUNC index) for resolution,
                    // matching the translation path in translate_call_op.
                    if argc == 0 {
                        let abs_addr = bytecode_offset + inst.offset;
                        let resolved_name = func_ref_map
                            .get(&abs_addr)
                            .and_then(|&idx| function_names.get(&(idx as u32)));
                        let fallback_name = function_names.get(&function_id);
                        let name = resolved_name.or(fallback_name);
                        if matches!(name, Some(n) if n == "@@Global@@") {
                            global_scope_on_stack = true;
                        }
                    }
                }
                ws.push(4); // return value = Variable
            }

            Opcode::CallV => {
                if let datawin::bytecode::decode::Operand::Call { argc, .. } = inst.operand {
                    ws_pop(&mut ws, argc as usize + 2);
                } else {
                    ws_pop(&mut ws, 2);
                }
                ws.push(4); // return value = Variable
            }

            // Branch/terminator pops (pushes are always 0):
            Opcode::Bt | Opcode::Bf => {
                ws_pop(&mut ws, 1);
            }
            Opcode::PushEnv => {
                ws_pop(&mut ws, 1);
            }
            Opcode::Ret => {
                ws_pop(&mut ws, 1);
            }
            Opcode::B | Opcode::Exit | Opcode::PopEnv => {}

            Opcode::Break => {
                if let datawin::bytecode::decode::Operand::Break { signal, .. } = inst.operand {
                    match signal {
                        0xFFFF => {} // chkindex
                        0xFFFC => {
                            // pushac — captures array index for upcoming popaf.
                            ws_pop(&mut ws, 1);
                            pushac_pending = true;
                        }
                        0xFFFB => ws_pop(&mut ws, 1), // setowner
                        0xFFFE => {
                            // pushaf
                            ws_pop(&mut ws, 2);
                            ws.push(4);
                        }
                        0xFFFD => {
                            // popaf — array element set.
                            // If pushac captured the index, popaf pops 2 (value + array).
                            // Otherwise (simple or compound write), popaf pops 3
                            // (value + array + index).
                            if pushac_pending {
                                ws_pop(&mut ws, 2);
                                pushac_pending = false;
                            } else {
                                ws_pop(&mut ws, 3);
                            }
                        }
                        0xFFF6 => ws.push(1), // chknullish (bool)
                        0xFFF5 => ws.push(4), // pushref
                        0xFFFA => ws.push(1), // isstaticok (bool)
                        0xFFF7..=0xFFF9 => {} // setstatic, savearef, restorearef
                        _ => {}
                    }
                }
            }
        }

        // Record depths at branch targets.
        match inst.opcode {
            Opcode::B => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    record_depth(&mut depths, target, ws.len());
                }
                terminated = true;
            }
            Opcode::Bt | Opcode::Bf => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    record_depth(&mut depths, target, ws.len());
                    if let Some(next) = instructions.get(i + 1) {
                        record_depth(&mut depths, next.offset, ws.len());
                    }
                }
                terminated = true;
            }
            Opcode::PushEnv => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    record_depth(&mut depths, target, ws.len());
                    if let Some(next) = instructions.get(i + 1) {
                        record_depth(&mut depths, next.offset, ws.len());
                    }
                }
                terminated = true;
            }
            Opcode::PopEnv => {
                if let datawin::bytecode::decode::Operand::Branch(offset) = inst.operand {
                    let target = (inst.offset as i64 + offset as i64) as usize;
                    record_depth(&mut depths, target, ws.len());
                    if let Some(next) = instructions.get(i + 1) {
                        record_depth(&mut depths, next.offset, ws.len());
                    }
                }
                terminated = true;
            }
            Opcode::Ret | Opcode::Exit => {
                terminated = true;
            }
            _ => {}
        }
    }

    depths
}

/// Build branch arguments from the current stack based on target block's entry depth.
pub(super) fn get_branch_args(stack: &[ValueId], target_depth: usize) -> Vec<ValueId> {
    stack.iter().take(target_depth).copied().collect()
}
