use std::collections::HashMap;

use datawin::bytecode::decode::{Instruction, Operand};
use datawin::bytecode::opcode::Opcode;
use datawin::bytecode::types::DataType;
use reincarnate_core::ir::block::BlockId;
use reincarnate_core::ir::builder::FunctionBuilder;
use reincarnate_core::ir::inst::CmpKind;
use reincarnate_core::ir::ty::Type;
use reincarnate_core::ir::value::ValueId;

use super::cfg::{get_branch_args, gml_slot_units};
use super::variable_access::{translate_pop, translate_push};
use super::{
    comparison_to_cmp_kind, datatype_to_ir_type, is_next_stacktop_ref_access, pop,
    resolve_branch_target, resolve_fallthrough, TranslateCtx,
};

/// Translate a single instruction — thin dispatcher to themed helpers.
#[allow(clippy::too_many_arguments)]
pub(super) fn translate_instruction(
    inst: &Instruction,
    instructions: &[Instruction],
    inst_idx: usize,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    block_map: &HashMap<usize, BlockId>,
    locals: &mut HashMap<String, ValueId>,
    ctx: &TranslateCtx,
    terminated: &mut bool,
    block_entry_depths: &HashMap<usize, usize>,
    gml_sizes: &mut HashMap<ValueId, u8>,
    compound_2d_pending: &mut bool,
    pushac_array: &mut Option<ValueId>,
    compound_popaf_pending: &mut bool,
    global_arg_count: u16,
    obj_ref_values: &mut HashMap<ValueId, String>,
    global_scope_on_stack: &mut bool,
) -> Result<(), String> {
    match inst.opcode {
        // Constants (push)
        Opcode::PushI | Opcode::Push | Opcode::PushLoc | Opcode::PushGlb | Opcode::PushBltn => {
            translate_push_instruction(
                inst,
                instructions,
                inst_idx,
                fb,
                stack,
                locals,
                ctx,
                gml_sizes,
                compound_2d_pending,
                global_arg_count,
                global_scope_on_stack,
            )?;
        }

        // Arithmetic & unary
        Opcode::Add
        | Opcode::Sub
        | Opcode::Mul
        | Opcode::Div
        | Opcode::Rem
        | Opcode::Mod
        | Opcode::Neg
        | Opcode::Not => {
            translate_arithmetic_op(inst, fb, stack, gml_sizes)?;
        }

        // Bitwise, boolean, comparison
        Opcode::And | Opcode::Or | Opcode::Xor | Opcode::Shl | Opcode::Shr | Opcode::Cmp => {
            translate_bitwise_cmp_op(inst, fb, stack, gml_sizes)?;
        }

        // Control flow (branches, return, exit)
        Opcode::B | Opcode::Bt | Opcode::Bf | Opcode::Ret | Opcode::Exit => {
            translate_control_flow_op(
                inst,
                instructions,
                inst_idx,
                fb,
                stack,
                block_map,
                terminated,
                block_entry_depths,
            )?;
        }

        // Stack management
        Opcode::Popz | Opcode::Dup => {
            translate_stack_op(inst, fb, stack, gml_sizes, compound_popaf_pending)?;
        }

        // Pop (variable store)
        Opcode::Pop => {
            translate_pop(
                inst,
                fb,
                stack,
                locals,
                ctx,
                compound_2d_pending,
                global_arg_count,
            )?;
        }

        // Function calls
        Opcode::Call | Opcode::CallV => {
            translate_call_op(inst, fb, stack, ctx, gml_sizes, global_scope_on_stack)?;
        }

        // Type conversion
        Opcode::Conv => {
            translate_conv_op(inst, fb, stack, gml_sizes)?;
        }

        // PushEnv / PopEnv (with-blocks)
        Opcode::PushEnv | Opcode::PopEnv => {
            translate_env_op(
                inst,
                instructions,
                inst_idx,
                fb,
                stack,
                block_map,
                ctx,
                terminated,
                block_entry_depths,
            )?;
        }

        // Break (special signals)
        Opcode::Break => {
            translate_break_op(
                inst,
                fb,
                stack,
                ctx,
                gml_sizes,
                pushac_array,
                compound_popaf_pending,
                obj_ref_values,
            )?;
        }
    }

    Ok(())
}

// ============================================================
// Push constants
// ============================================================

/// Handle Push* opcodes — delegates to `translate_push` and annotates GML type size.
#[allow(clippy::too_many_arguments)]
fn translate_push_instruction(
    inst: &Instruction,
    instructions: &[Instruction],
    inst_idx: usize,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    locals: &mut HashMap<String, ValueId>,
    ctx: &TranslateCtx,
    gml_sizes: &mut HashMap<ValueId, u8>,
    compound_2d_pending: &mut bool,
    global_arg_count: u16,
    global_scope_on_stack: &mut bool,
) -> Result<(), String> {
    // Skip PushI -9 sentinel for cross-object stacktop field access.
    // Pattern: PushLoc/PushGlb/Push target → PushI -9 → Push.v/Pop.v [ref_type=0x80]
    // The -9 is a redundant sentinel; skipping it lets the stacktop handler
    // pop the actual target instance instead of the useless -9.
    //
    // Also skip unconditionally when @@Global@@ pushed the global scope.
    // GMS2.3+ @@Global@@ emits: Call @@Global@@ → PushI -9 → intermediates → VARI access.
    // The intermediates (PushLoc for array index, Push Int64 for enum ID) push values
    // consumed by the eventual Pop.v/Push.v as array indices in the 2D array handler.
    // If -9 is NOT skipped, the 2D handler pops dim2=-9 (treats as self-scope) and
    // the global scope value becomes orphaned or consumed as the wrong operand.
    // Skipping -9 lets the global scope land in the dim2 slot, routing through
    // the "dynamic dim2" branch → setOn(global_scope, field, index, value).
    if matches!(inst.operand, Operand::Int16(-9))
        && inst_idx > 0
        && matches!(
            instructions[inst_idx - 1].opcode,
            Opcode::PushLoc
                | Opcode::PushGlb
                | Opcode::Push
                | Opcode::PushBltn
                | Opcode::PushI
                | Opcode::Call
                | Opcode::CallV
                | Opcode::Break
        )
        && (is_next_stacktop_ref_access(&instructions[inst_idx + 1..]) || *global_scope_on_stack)
    {
        *global_scope_on_stack = false;
        return Ok(());
    }
    // Clear the flag if PushI -9 was NOT skipped (e.g. different operand).
    if *global_scope_on_stack && matches!(inst.operand, Operand::Int16(-9)) {
        *global_scope_on_stack = false;
    }

    // Check if the preceding instruction was a normal Dup (not swap).
    // This signals a compound 2D assignment pattern (e.g. `arr[i,j] += x`):
    //   push dim2, push dim1, Dup(normal), VARI-read (← here), arith, VARI-write
    // The Dup leaves original indices below the copies; after the VARI-read
    // pops the copies, the originals remain for the VARI-write.
    let preceded_by_dup = inst_idx > 0
        && instructions[inst_idx - 1].opcode == Opcode::Dup
        && match instructions[inst_idx - 1].operand {
            Operand::Dup(n) => (n >> 8) & 0xFF == 0, // dup_extra == 0 → normal dup
            _ => true,                               // non-Dup(n) form is always normal
        };

    let depth_before = stack.len();
    translate_push(
        inst,
        &instructions[inst_idx + 1..],
        fb,
        stack,
        locals,
        ctx,
        compound_2d_pending,
        global_arg_count,
        preceded_by_dup,
    )?;
    // Annotate newly pushed value with its GML type size.
    if stack.len() > depth_before {
        if let Some(&val) = stack.last() {
            let units = match &inst.operand {
                Operand::Variable { .. } => 4, // Variable reads → RValue (16 bytes)
                _ => gml_slot_units(inst.type1),
            };
            gml_sizes.insert(val, units);
        }
    }
    Ok(())
}

// ============================================================
// Arithmetic & unary
// ============================================================

/// Handle Add, Sub, Mul, Div, Rem/Mod, Neg, Not.
///
/// The result size on the GML VM stack is the MAXIMUM of the two operand type
/// sizes. For same-type ops like `Add.i` (Int32+Int32), the result is Int32-
/// sized (1 unit). For mixed-type ops like `Add.i[v]` (Int32+Variable), the
/// result is Variable-sized (4 units). This matters for subsequent `Dup`
/// instructions which count backwards by byte units — recording a smaller
/// size causes Dup to over-duplicate items from below the intended top.
fn translate_arithmetic_op(
    inst: &Instruction,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    gml_sizes: &mut HashMap<ValueId, u8>,
) -> Result<(), String> {
    let result_units = gml_slot_units(inst.type1).max(gml_slot_units(inst.type2));
    // Derive the return type from type1 (primary type tag on the instruction).
    let ret_ty = datatype_to_ir_type(inst.type1, fb);

    match inst.opcode {
        Opcode::Add => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            // Use the explicit type suffix from the GML instruction type tag.
            let suffix = type_suffix_for(inst.type1);
            let r = fb.call_named(&arith_callee("add", suffix), &[a, b], ret_ty);
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Sub => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            let suffix = type_suffix_for(inst.type1);
            let r = fb.call_named(&arith_callee("sub", suffix), &[a, b], ret_ty);
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Mul => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            let suffix = type_suffix_for(inst.type1);
            let r = fb.call_named(&arith_callee("mul", suffix), &[a, b], ret_ty);
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Div => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            let suffix = type_suffix_for(inst.type1);
            let r = fb.call_named(&arith_callee("div", suffix), &[a, b], ret_ty);
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Rem | Opcode::Mod => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            let suffix = type_suffix_for(inst.type1);
            let r = fb.call_named(&arith_callee("rem", suffix), &[a, b], ret_ty);
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Neg => {
            let a = pop(stack, inst)?;
            let suffix = type_suffix_for(inst.type1);
            let r = fb.call_named(&arith_callee("neg", suffix), &[a], ret_ty);
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Not => {
            let a = pop(stack, inst)?;
            // GML `!` always operates on Bool.
            let r = fb.call_named("not_bool", &[a], Type::Bool);
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        _ => unreachable!(),
    }
    Ok(())
}

/// Return the type suffix used in builtin names for a given GML DataType.
///
/// `Int32` and `Int16` map to `"f64"` because GML's single numeric type is
/// Real (Float64) at the source level. The VM uses `Int32`/`Int16` opcodes
/// internally, but at source semantics those values are all Reals.
///
fn type_suffix_for(dt: DataType) -> &'static str {
    match dt {
        DataType::Double | DataType::Int32 | DataType::Int16 => "f64",
        DataType::Float => "f32",
        DataType::Int64 => "i64",
        DataType::Bool => "bool",
        DataType::String => "str",
        DataType::Variable => "any",
        _ => "any",
    }
}

/// Build the full callee name for an arithmetic op and data-type suffix.
///
/// All variants (typed and `_any`) follow the `{op}_{suffix}` naming pattern.
/// The `_any` variants have dispatch bodies and are registered in the runtime
/// registry so `FunctionBuilder::add()` (which generates `"add_any"`) resolves
/// them from the core registry.
fn arith_callee(op: &str, suffix: &str) -> String {
    match (op, suffix) {
        // String "add" is concatenation, not arithmetic.
        ("add", "str") => "concat_str".to_string(),
        // All other variants (typed and _any) use op_suffix naming.
        _ => format!("{op}_{suffix}"),
    }
}

/// Emit a binary bitwise builtin (`bitand`, `bitor`, `bitxor`, `shl`, `shr`)
/// using the `_i32` core variant, inserting Float(64) ↔ Int(32) coercions when
/// the GML source type is Real (Float64).
///
/// GML performs bitwise operations on Reals via implicit ToInt32 coercion:
/// `a & b` is semantically `float((int(a)) & (int(b)))`.  The core IR only
/// provides `{op}_i32` (integer semantics); the coercions are
/// GML-specific and belong here in the frontend.
fn emit_gml_bitwise_bin(
    fb: &mut FunctionBuilder,
    op: &str,
    a: ValueId,
    b: ValueId,
    ret_ty: Type,
) -> ValueId {
    let needs_coerce = matches!(ret_ty, Type::Float(64));
    let (a_i, b_i) = if needs_coerce {
        (fb.coerce(a, Type::Int(32)), fb.coerce(b, Type::Int(32)))
    } else {
        (a, b)
    };
    let r_i = fb.call_named(&format!("{op}_i32"), &[a_i, b_i], Type::Int(32));
    if needs_coerce {
        fb.coerce(r_i, Type::Float(64))
    } else {
        r_i
    }
}

// ============================================================
// Bitwise, boolean, comparison
// ============================================================

/// Handle And, Or, Xor, Shl, Shr, Cmp.
///
/// Like arithmetic ops, the result size is max(type1, type2) to correctly
/// model the VM stack slot size for subsequent Dup instructions.
fn translate_bitwise_cmp_op(
    inst: &Instruction,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    gml_sizes: &mut HashMap<ValueId, u8>,
) -> Result<(), String> {
    let result_units = gml_slot_units(inst.type1).max(gml_slot_units(inst.type2));

    match inst.opcode {
        Opcode::And => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            // GML uses one And opcode for both `&&` (Bool operands) and `&` (Int operands).
            let r = if inst.type1 == DataType::Bool {
                fb.call_named("and_bool", &[a, b], Type::Bool)
            } else {
                let ret_ty = datatype_to_ir_type(inst.type1, fb);
                emit_gml_bitwise_bin(fb, "bitand", a, b, ret_ty)
            };
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Or => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            // GML uses one Or opcode for both `||` (Bool operands) and `|` (Int operands).
            let r = if inst.type1 == DataType::Bool {
                fb.call_named("or_bool", &[a, b], Type::Bool)
            } else {
                let ret_ty = datatype_to_ir_type(inst.type1, fb);
                emit_gml_bitwise_bin(fb, "bitor", a, b, ret_ty)
            };
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Xor => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            let ret_ty = datatype_to_ir_type(inst.type1, fb);
            let r = emit_gml_bitwise_bin(fb, "bitxor", a, b, ret_ty);
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Shl => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            let ret_ty = datatype_to_ir_type(inst.type1, fb);
            let r = emit_gml_bitwise_bin(fb, "shl", a, b, ret_ty);
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Shr => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            let ret_ty = datatype_to_ir_type(inst.type1, fb);
            let r = emit_gml_bitwise_bin(fb, "shr", a, b, ret_ty);
            gml_sizes.insert(r, result_units);
            stack.push(r);
        }
        Opcode::Cmp => {
            let b = pop(stack, inst)?;
            let a = pop(stack, inst)?;
            if let Operand::Comparison(kind) = inst.operand {
                let cmp_kind = comparison_to_cmp_kind(kind);
                let r = fb.cmp(cmp_kind, a, b);
                gml_sizes.insert(r, result_units);
                stack.push(r);
            } else {
                return Err(format!(
                    "{:#x}: Cmp without comparison operand",
                    inst.offset
                ));
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

// ============================================================
// Control flow
// ============================================================

/// Handle B, Bt, Bf, Ret, Exit.
#[allow(clippy::too_many_arguments)]
fn translate_control_flow_op(
    inst: &Instruction,
    instructions: &[Instruction],
    inst_idx: usize,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    block_map: &HashMap<usize, BlockId>,
    terminated: &mut bool,
    block_entry_depths: &HashMap<usize, usize>,
) -> Result<(), String> {
    match inst.opcode {
        Opcode::B => {
            if let Operand::Branch(offset) = inst.operand {
                let (target_off, target) = resolve_branch_target(inst, offset, block_map)?;
                let depth = block_entry_depths.get(&target_off).copied().unwrap_or(0);
                let args = get_branch_args(stack, depth);
                fb.br(target, &args);
                *terminated = true;
            }
        }
        Opcode::Bt => {
            translate_conditional_branch(
                inst,
                instructions,
                inst_idx,
                fb,
                stack,
                block_map,
                terminated,
                block_entry_depths,
                true, // branch_on_true
            )?;
        }
        Opcode::Bf => {
            translate_conditional_branch(
                inst,
                instructions,
                inst_idx,
                fb,
                stack,
                block_map,
                terminated,
                block_entry_depths,
                false, // branch_on_false
            )?;
        }
        Opcode::Ret => {
            let val = pop(stack, inst)?;
            fb.ret(Some(val));
            *terminated = true;
        }
        Opcode::Exit => {
            fb.ret(None);
            *terminated = true;
        }
        _ => unreachable!(),
    }
    Ok(())
}

/// Shared logic for Bt and Bf conditional branches.
///
/// When `branch_on_true` is true, the branch target is the "then" arm (Bt semantics).
/// When false, the fall-through is the "then" arm (Bf semantics).
#[allow(clippy::too_many_arguments)]
fn translate_conditional_branch(
    inst: &Instruction,
    instructions: &[Instruction],
    inst_idx: usize,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    block_map: &HashMap<usize, BlockId>,
    terminated: &mut bool,
    block_entry_depths: &HashMap<usize, usize>,
    branch_on_true: bool,
) -> Result<(), String> {
    if let Operand::Branch(offset) = inst.operand {
        let cond = pop(stack, inst)?;
        let branch_target = resolve_branch_target(inst, offset, block_map).ok();
        let fall_target = resolve_fallthrough(instructions, inst_idx, block_map).ok();
        // For Bt: then=branch, else=fall-through.
        // For Bf: then=fall-through, else=branch.
        let (then_target, else_target) = if branch_on_true {
            (branch_target, fall_target)
        } else {
            (fall_target, branch_target)
        };
        match (then_target, else_target) {
            (Some((then_off, then_blk)), Some((else_off, else_blk))) => {
                let then_args = get_branch_args(
                    stack,
                    block_entry_depths.get(&then_off).copied().unwrap_or(0),
                );
                let else_args = get_branch_args(
                    stack,
                    block_entry_depths.get(&else_off).copied().unwrap_or(0),
                );
                fb.br_if(cond, then_blk, &then_args, else_blk, &else_args);
            }
            (Some((off, blk)), None) => {
                let ret_blk = fb.create_block();
                fb.br_if(
                    cond,
                    blk,
                    &get_branch_args(stack, block_entry_depths.get(&off).copied().unwrap_or(0)),
                    ret_blk,
                    &[],
                );
                fb.switch_to_block(ret_blk);
                fb.ret(None);
            }
            (None, Some((off, blk))) => {
                let ret_blk = fb.create_block();
                fb.br_if(
                    cond,
                    ret_blk,
                    &[],
                    blk,
                    &get_branch_args(stack, block_entry_depths.get(&off).copied().unwrap_or(0)),
                );
                fb.switch_to_block(ret_blk);
                fb.ret(None);
            }
            (None, None) => {
                fb.ret(None);
            }
        }
        *terminated = true;
    }
    Ok(())
}

// ============================================================
// Stack management
// ============================================================

/// Handle Popz, Dup.
fn translate_stack_op(
    inst: &Instruction,
    _fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    gml_sizes: &mut HashMap<ValueId, u8>,
    compound_popaf_pending: &mut bool,
) -> Result<(), String> {
    match inst.opcode {
        Opcode::Popz => {
            let _ = pop(stack, inst)?;
        }
        Opcode::Dup => {
            if let Operand::Dup(n) = inst.operand {
                // dup_extra is the high byte of the 16-bit operand. In GMS2.3+ (bc >= 17),
                // a non-zero dup_extra signals swap/no-op modes. On older versions this byte
                // is always zero, so this interpretation is safe unconditionally.
                let dup_extra = (n >> 8) & 0xFF;
                let dup_size = (n & 0xFF) as usize;
                if dup_extra != 0 {
                    // GMS2.3+ extended Dup encoding (DupExtra != 0).  Used in two cases:
                    //
                    // 1. Struct swap marker (type=Variable, dup_size=0): pure no-op in the VM.
                    //
                    // 2. Byte-reorder swap (dup_size > 0): the GML VM uses this before popaf
                    //    in compound array writes (e.g. `arr[i] -= 2`).  The pattern is:
                    //      Dup(normal)  → copies arr+i to top; originals stay below
                    //      pushaf       → pops copies, pushes arr[i]; originals still below
                    //      <arithmetic> → pushes new_value on top
                    //      Dup(swap)    → ← here; stack is [..., arr, i, new_value]
                    //      popaf        → needs (value=top, index=next, array=below)
                    //    At Dup(swap) time the logical stack order is [..., ARRAY, INDEX, VALUE].
                    //    The GML VM reorders bytes because Variable is 16 bytes while the
                    //    computed result may be 4 bytes, so fixed-size popaf needs the bytes
                    //    shuffled.  Our logical ValueId stack has no byte-size issue, BUT the
                    //    compound stack order (ARRAY below INDEX) is the REVERSE of the simple
                    //    write order (INDEX below ARRAY).  Set a flag so popaf knows to swap.
                    if dup_size > 0 {
                        *compound_popaf_pending = true;
                    }
                } else {
                    // Normal dup: duplicate (dup_size + 1) * type_unit units from stack top.
                    let type_unit = gml_slot_units(inst.type1) as usize;
                    let total_units = (dup_size + 1) * type_unit;

                    // Count backwards from stack top to find how many items
                    // correspond to total_units.
                    let mut units_remaining = total_units;
                    let mut item_count = 0;
                    for &v in stack.iter().rev() {
                        if units_remaining == 0 {
                            break;
                        }
                        let item_units = gml_sizes.get(&v).copied().unwrap_or(1) as usize;
                        if item_units > units_remaining {
                            // Item is larger than remaining units — this shouldn't
                            // happen with well-formed bytecode. Include it anyway.
                            item_count += 1;
                            break;
                        }
                        units_remaining -= item_units;
                        item_count += 1;
                    }

                    if stack.len() < item_count {
                        return Err(format!(
                            "{:#x}: Dup({}) on stack of depth {} (need {} items for {} units)",
                            inst.offset,
                            dup_size,
                            stack.len(),
                            item_count,
                            total_units
                        ));
                    }
                    let start = stack.len() - item_count;
                    let to_dup: Vec<ValueId> = stack[start..].to_vec();
                    for &v in &to_dup {
                        stack.push(v);
                    }
                }
            } else {
                if stack.is_empty() {
                    return Err(format!("{:#x}: Dup(0) on stack of depth 0", inst.offset));
                }
                let v = *stack.last().unwrap();
                stack.push(v);
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

// ============================================================
// Function calls
// ============================================================

/// Handle Call, CallV.
fn translate_call_op(
    inst: &Instruction,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    ctx: &TranslateCtx,
    gml_sizes: &mut HashMap<ValueId, u8>,
    global_scope_on_stack: &mut bool,
) -> Result<(), String> {
    match inst.opcode {
        Opcode::Call => {
            if let Operand::Call { function_id, argc } = inst.operand {
                // first_address points to the Call instruction word.
                let abs_addr = ctx.bytecode_offset + inst.offset;
                let func_name = ctx
                    .func_ref_map
                    .get(&abs_addr)
                    .and_then(|&idx| ctx.function_names.get(&(idx as u32)))
                    .cloned()
                    .unwrap_or_else(|| format!("func_unknown_{function_id}"));

                // GMS2.3+ internal built-in functions — resolve to IR values directly.
                // @@This@@ returns the calling instance (self). Replacing it with the
                // self parameter avoids emitting a `this` expression in free functions
                // (which have no implicit `this` binding).
                if func_name == "@@This@@" && argc == 0 {
                    let val = if ctx.has_self {
                        fb.param(0)
                    } else {
                        fb.const_null()
                    };
                    gml_sizes.insert(val, 4);
                    stack.push(val);
                    return Ok(());
                }

                // @@Global@@ returns the global scope. Push the call result
                // (which the backend rewrites to `global`) and set the flag
                // so the PushI -9 skip fires for InstanceType::Stacktop too.
                if func_name == "@@Global@@" && argc == 0 {
                    let ty = fb.fresh_var();
                    let result = fb.call_named("@@Global@@", &[], ty);
                    gml_sizes.insert(result, 4);
                    stack.push(result);
                    *global_scope_on_stack = true;
                    return Ok(());
                }

                let mut args = Vec::with_capacity(argc as usize + 1);
                for _ in 0..argc {
                    args.push(pop(stack, inst)?);
                }
                // Scripts receive the caller's instance as an implicit first arg.
                if ctx.script_names.contains(&func_name) {
                    let self_val = if ctx.has_self {
                        fb.param(0)
                    } else {
                        fb.const_null()
                    };
                    args.insert(0, self_val);
                }
                // instance_destroy() with no explicit target destroys the calling instance.
                // Inject self as the first arg so the runtime knows which instance to remove.
                if func_name == "instance_destroy" && argc == 0 && ctx.has_self {
                    args.push(fb.param(0));
                }
                // @@NewGMLObject@@() creates an anonymous GML struct. In GMS2.3+, anonymous
                // structs are GMLObject instances (no events, not a room instance, but the
                // same base type). Giving it a concrete return type lets type inference produce
                // proper unions (e.g. `GMLObject | boolean | number`) instead of Unknown.
                //
                // @@NewGMLArray@@(v0, v1, ...) creates a GML array literal.  Its function
                // stub has return_ty=Void (default), which would override any TypeVar-based
                // inference and type every array literal as void.  Use Array(Unknown) as the
                // call-site type so the writeback preserves a concrete array type even when
                // the constraint solver tries to reconcile with the Void stub sig.
                let ret_ty = if func_name == "@@NewGMLObject@@" {
                    ctx.instance_types
                        .get("GMLObject")
                        .copied()
                        .map(Type::Instance)
                        .unwrap_or_else(|| fb.fresh_var())
                } else if func_name == "@@NewGMLArray@@" {
                    Type::Array(Box::new(Type::Unknown))
                } else {
                    fb.fresh_var()
                };
                let result = fb.call_named(&func_name, &args, ret_ty);
                gml_sizes.insert(result, 4); // Call returns Variable (16 bytes)
                stack.push(result);
            }
        }
        Opcode::CallV => {
            if let Operand::Call { argc, .. } = inst.operand {
                let callee = pop(stack, inst)?;
                // CallV also pops the instance/receiver below the function ref.
                let _instance = pop(stack, inst)?;
                let mut args = Vec::with_capacity(argc as usize);
                for _ in 0..argc {
                    args.push(pop(stack, inst)?);
                }
                let ty = fb.fresh_var();
                let result = fb.call_indirect(callee, &args, ty);
                gml_sizes.insert(result, 4); // CallV returns Variable (16 bytes)
                stack.push(result);
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

// ============================================================
// Type conversion
// ============================================================

/// Handle Conv (type coercion).
fn translate_conv_op(
    inst: &Instruction,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    gml_sizes: &mut HashMap<ValueId, u8>,
) -> Result<(), String> {
    let val = pop(stack, inst)?;
    // Conv → Variable is a boxing-to-generic-slot operation.  In the IR we
    // have no Variable type — values carry their actual types — so this coerce
    // is a no-op.  Push the source value directly instead of wrapping it in a
    // Coerce(val, Var(fresh)), which would leave the result unresolved → Unknown
    // after constraint solving.
    if inst.type2 == DataType::Variable {
        let val_size = gml_sizes.get(&val).copied().unwrap_or(4);
        gml_sizes.insert(val, val_size);
        stack.push(val);
        return Ok(());
    }
    let target_ty = datatype_to_ir_type(inst.type2, fb);
    let coerced = fb.coerce(val, target_ty);
    gml_sizes.insert(coerced, gml_slot_units(inst.type2));
    stack.push(coerced);
    Ok(())
}

// ============================================================
// PushEnv / PopEnv (with-blocks)
// ============================================================

/// Handle PushEnv, PopEnv.
#[allow(clippy::too_many_arguments)]
fn translate_env_op(
    inst: &Instruction,
    instructions: &[Instruction],
    inst_idx: usize,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    block_map: &HashMap<usize, BlockId>,
    ctx: &TranslateCtx,
    terminated: &mut bool,
    block_entry_depths: &HashMap<usize, usize>,
) -> Result<(), String> {
    match inst.opcode {
        Opcode::PushEnv => {
            if let Operand::Branch(_offset) = inst.operand {
                // Unmatched PushEnv: the matching PopEnv is in a sibling code entry
                // (GMS2.3+ cross-code-entry with-block).  We can't extract the full
                // body as a closure here.  Pop the target object (discarded) and fall
                // through to the body block — this executes the body for `self` only,
                // which is semantically incomplete but produces valid TypeScript.
                let _target_obj = pop(stack, inst)?;
                let (body_off, body_block) =
                    resolve_fallthrough(instructions, inst_idx, block_map)?;
                let args = get_branch_args(
                    stack,
                    block_entry_depths.get(&body_off).copied().unwrap_or(0),
                );
                fb.br(body_block, &args);
                *terminated = true;
            }
        }
        Opcode::PopEnv => {
            if let Operand::Branch(off) = inst.operand {
                if ctx.is_with_body {
                    // Inside a with-body closure, PopEnv is an early-exit signal.
                    //
                    // Case 1: sentinel branch offset (≈ -4194304) — GML "return X inside
                    // with". The value was stored to a local before this PopEnv; the
                    // fall-through block (next instruction in body_insts) loads that local
                    // and emits the actual return. Branch to it so the closure returns
                    // the value and withInstances can propagate it to the outer function.
                    //
                    // Case 2: normal PopEnv (break/continue) — return void from the
                    // closure. withInstances handles loop termination.
                    if ctx.with_body_has_return && off < -1_000_000 {
                        // Exit PopEnv (return X inside with): fall through to the
                        // continuation block that loads the return value and returns it.
                        if let Some(next_inst) = instructions.get(inst_idx + 1) {
                            if let Some(&fall_block) = block_map.get(&next_inst.offset) {
                                let depth = block_entry_depths
                                    .get(&next_inst.offset)
                                    .copied()
                                    .unwrap_or(0);
                                let args = get_branch_args(stack, depth);
                                fb.br(fall_block, &args);
                                *terminated = true;
                                return Ok(());
                            }
                        }
                    }
                    fb.ret(None);
                } else {
                    // Unmatched PopEnv (sibling of an unmatched PushEnv in the same
                    // function, or the loop-back PopEnv of a GMS2.3+ cross-code-entry
                    // with-block).  Fall through to the continuation block.
                    let (fall_off, fall) = resolve_fallthrough(instructions, inst_idx, block_map)?;
                    let args = get_branch_args(
                        stack,
                        block_entry_depths.get(&fall_off).copied().unwrap_or(0),
                    );
                    fb.br(fall, &args);
                }
                *terminated = true;
            }
        }
        _ => unreachable!(),
    }
    Ok(())
}

// ============================================================
// Break (special signals)
// ============================================================

/// Handle Break signal opcodes (pushaf, popaf, pushac, setowner, isstaticok, pushref, etc.).
#[allow(clippy::too_many_arguments)]
fn translate_break_op(
    inst: &Instruction,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    ctx: &TranslateCtx,
    gml_sizes: &mut HashMap<ValueId, u8>,
    pushac_array: &mut Option<ValueId>,
    compound_popaf_pending: &mut bool,
    obj_ref_values: &mut HashMap<ValueId, String>,
) -> Result<(), String> {
    if let Operand::Break { signal, .. } = inst.operand {
        match signal {
            0xFFFF => {} // chkindex — nop for decompilation
            0xFFFE => {
                // pushaf — array element get.
                // Stack: [..., ARRAY, INDEX] with INDEX on top.
                // Strip Conv.v.i32 from the index (VM artifact).
                let index = pop(stack, inst)?;
                let index = fb.try_peel_int_coerce(index);
                let array = pop(stack, inst)?;
                let ty = fb.fresh_var();
                let val = fb.get_index(array, index, ty);
                gml_sizes.insert(val, 4); // Variable (16 bytes)
                stack.push(val);
            }
            0xFFFD => {
                // popaf — array element set.
                //
                // Three stack layouts depending on context:
                //
                // Simple write [..., INDEX, ARRAY, VALUE] (VALUE on top):
                //   pop VALUE, pop ARRAY (2nd), pop INDEX (3rd)
                //   → ARRAY[INDEX] = VALUE
                //
                // Compound write [..., ARRAY, INDEX, VALUE] (after
                //   Dup(normal)+pushaf+arithmetic+Dup(swap)):
                //   pop VALUE, pop INDEX (2nd), pop ARRAY (3rd)
                //   → ARRAY[INDEX] = VALUE
                //   compound_popaf_pending flag (set by Dup(swap)) signals this.
                //
                // pushac case [..., ARRAY, VALUE] (VALUE on top;
                //   INDEX saved by pushac):
                //   pop VALUE, pop ARRAY (2nd), use saved INDEX
                //   → ARRAY[INDEX] = VALUE
                let value = pop(stack, inst)?;
                let (array, index) = if let Some(idx) = pushac_array.take() {
                    let arr = pop(stack, inst)?;
                    (arr, idx)
                } else if *compound_popaf_pending {
                    *compound_popaf_pending = false;
                    let idx = pop(stack, inst)?;
                    let arr = pop(stack, inst).unwrap_or_else(|_| fb.const_int(-6, 64));
                    (arr, idx)
                } else {
                    let arr = pop(stack, inst)?;
                    let idx = pop(stack, inst).unwrap_or_else(|_| fb.const_int(-6, 64));
                    (arr, idx)
                };
                fb.set_index(array, index, value);
            }
            0xFFFC => {
                // pushac — capture the array INDEX for the upcoming popaf.
                // GMS2.3+ uses this to save the index before the array
                // reference and value are pushed onto the stack.  popaf
                // then pops VALUE (top) and ARRAY (second), using the
                // saved index: ARRAY[INDEX] = VALUE.
                //
                // The GML VM often emits Conv.v.i32 before pushac to
                // ensure the index is an integer.  Strip the coerce so
                // the index stays Unknown (JS arrays accept any key).
                let idx = pop(stack, inst)?;
                let idx = fb.try_peel_int_coerce(idx);
                *pushac_array = Some(idx);
            }
            0xFFFB => {
                // setowner — pops the owner instance ID from the stack.
                let owner = pop(stack, inst)?;
                let _ = owner;
            }
            0xFFFA => {
                // isstaticok — static init guard. Pushes true if statics
                // are already initialized; used with Bt to skip init code.
                // For decompilation we push false so the init code is emitted.
                let r = fb.const_bool(false);
                gml_sizes.insert(r, 1); // Boolean (4 bytes)
                stack.push(r);
            }
            0xFFF9 => {} // setstatic — set static scope, nop for decompilation
            0xFFF8 => {} // savearef — save array ref to temp, nop for decompilation
            0xFFF7 => {} // restorearef — restore array ref from temp, nop for decompilation
            0xFFF6 => {
                // chknullish — GMS2.3+ only. Check if top of stack is nullish.
                // Pushes boolean; original value stays on stack below.
                // Used for ?? (nullish coalescing) and ?. (optional chaining).
                if !ctx.bytecode_version.is_gms23_plus() {
                    eprintln!(
                        "[warn] chknullish (Break -10) seen in bytecode_version={}, expected GMS2.3+",
                        ctx.bytecode_version.0
                    );
                }
                let val = *stack
                    .last()
                    .ok_or_else(|| format!("{:#x}: stack underflow on chknullish", inst.offset))?;
                let null_val = fb.const_null();
                let is_null = fb.cmp(CmpKind::Eq, val, null_val);
                gml_sizes.insert(is_null, 1); // Boolean (4 bytes)
                stack.push(is_null);
            }
            0xFFF5 => {
                // pushref — GMS2.3+ only. Push asset reference onto stack.
                // The extra Int32 operand encodes (type_tag << 24) | asset_index.
                if !ctx.bytecode_version.is_gms23_plus() {
                    eprintln!(
                        "[warn] pushref (Break -11) seen in bytecode_version={}, expected GMS2.3+",
                        ctx.bytecode_version.0
                    );
                }
                // Type 0 = OBJT (object), 1 = SPRT, 2 = SOND, 3 = ROOM, etc.
                // All types are resolved via asset_ref_names. Fall back to
                // func_ref_map (for GMS1 compatibility) and then to a placeholder.
                let is_objt = if let Operand::Break {
                    extra: Some(idx), ..
                } = inst.operand
                {
                    (idx as u32) >> 24 == 0
                } else {
                    false
                };
                let func_name = if let Operand::Break {
                    extra: Some(idx), ..
                } = inst.operand
                {
                    let key = idx as u32;
                    ctx.asset_ref_names.get(&key).cloned()
                } else {
                    None
                }
                .or_else(|| {
                    let abs_addr = ctx.bytecode_offset + inst.offset;
                    ctx.func_ref_map
                        .get(&abs_addr)
                        .and_then(|&i| ctx.function_names.get(&(i as u32)))
                        .cloned()
                })
                .unwrap_or_else(|| {
                    let abs_addr = ctx.bytecode_offset + inst.offset;
                    format!("func_ref_unknown_{:#x}", abs_addr)
                });
                // OBJT references are class constructors; everything else
                // (SPRT, SOND, ROOM, SCPT, etc.) is a plain asset index.
                let ref_ty = if is_objt {
                    ctx.classref_types
                        .get(&func_name)
                        .copied()
                        .map(Type::ClassRef)
                        .unwrap_or_else(|| fb.fresh_var())
                } else {
                    fb.fresh_var()
                };
                let val = fb.global_ref(&func_name, ref_ty);
                gml_sizes.insert(val, 4); // Variable (16 bytes)
                stack.push(val);
                // Track OBJT references so with-body closures can be typed.
                if is_objt {
                    obj_ref_values.insert(val, func_name);
                }
            }
            _ => {
                // Unknown break signal, emit as system call.
                let sig_val = fb.const_int(signal as i64, 64);
                fb.call_named("GameMaker.Debug.break", &[sig_val], Type::Void);
            }
        }
    }
    Ok(())
}
