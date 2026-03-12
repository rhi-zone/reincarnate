use super::*;
use datawin::bytecode::decode::Instruction;
use datawin::bytecode::encode::encode;
use datawin::bytecode::opcode::Opcode;
use datawin::bytecode::types::{DataType, VariableRef};
use reincarnate_core::ir::inst::Op;

static EMPTY_ASSET_REF_NAMES: std::sync::LazyLock<HashMap<u32, String>> =
    std::sync::LazyLock::new(HashMap::new);

/// Build a minimal `TranslateCtx` for tests.
///
/// `bytecode_offset = 0` so vari_ref_map keys equal decoded instruction offsets.
#[allow(clippy::too_many_arguments)]
fn make_ctx<'a>(
    has_self: bool,
    arg_count: u16,
    variables: &'a [(String, i32)],
    vari_ref_map: &'a HashMap<usize, usize>,
    func_ref_map: &'a HashMap<usize, usize>,
    obj_names: &'a [String],
    function_names: &'a HashMap<u32, String>,
    script_names: &'a HashSet<String>,
) -> TranslateCtx<'a> {
    TranslateCtx {
        function_names,
        asset_ref_names: &EMPTY_ASSET_REF_NAMES,
        variables,
        func_ref_map,
        vari_ref_map,
        bytecode_offset: 0,
        local_names: &[],
        string_table: &[],
        has_self,
        has_other: false,
        arg_count,
        obj_names,
        class_name: None,
        self_object_index: None,
        ancestor_indices: HashSet::new(),
        script_names,
        is_with_body: false,
        with_body_has_return: false,
        // Tests exercise GMS2.3+ bytecode by default (shared blobs, Break signals, etc.).
        bytecode_version: datawin::BytecodeVersion(17),
    }
}

/// Collect all `Op` values from a translated function.
fn collect_ops(func: &reincarnate_core::ir::func::Function) -> Vec<Op> {
    func.insts.values().map(|i| i.op.clone()).collect()
}

/// Build and encode a Push instruction for an Int16 constant.
fn pushi(val: i16) -> Instruction {
    Instruction {
        offset: 0,
        opcode: Opcode::PushI,
        type1: DataType::Int16,
        type2: DataType::Double,
        operand: Operand::Int16(val),
    }
}

/// Build and encode a Push.v.v (variable read) instruction.
fn push_var(instance: i16, ref_type: u8) -> Instruction {
    Instruction {
        offset: 0,
        opcode: Opcode::Push,
        type1: DataType::Variable,
        type2: DataType::Variable,
        operand: Operand::Variable {
            var_ref: VariableRef {
                variable_id: 0,
                ref_type,
            },
            instance,
        },
    }
}

/// Build and encode a Pop.v.v (variable write) instruction.
fn pop_var(instance: i16, ref_type: u8) -> Instruction {
    Instruction {
        offset: 0,
        opcode: Opcode::Pop,
        type1: DataType::Variable,
        type2: DataType::Variable,
        operand: Operand::Variable {
            var_ref: VariableRef {
                variable_id: 0,
                ref_type,
            },
            instance,
        },
    }
}

/// Build an Exit instruction (no-value return).
fn exit_inst() -> Instruction {
    Instruction {
        offset: 0,
        opcode: Opcode::Exit,
        type1: DataType::Double,
        type2: DataType::Double,
        operand: Operand::None,
    }
}

/// Build a Ret instruction (pops value from stack and returns it).
fn ret_inst() -> Instruction {
    Instruction {
        offset: 0,
        opcode: Opcode::Ret,
        type1: DataType::Variable,
        type2: DataType::Double,
        operand: Operand::None,
    }
}

// -----------------------------------------------------------------------
// 2D array write — correct stack pop order
// -----------------------------------------------------------------------

/// Non-scalar 2D array write: `myarray[0] = 42`
///
/// GML stack layout before Pop.v.v: `[value=42, dim2=-1, dim1=0]` (dim1 on top).
/// The translator must pop dim1 first (top), then dim2, then value.
/// Expected IR: `SetIndex(GetField(self, "myarray"), dim1_const, 42)`
///
/// This regression guards the fix for the 2D array write stack pop order bug:
/// if dim1 and value are accidentally swapped, a `SetField` (scalar) is emitted
/// instead of a `SetIndex`, or the wrong value is stored.
#[test]
fn test_2d_array_write_nonscalar_emits_set_index() {
    // Instructions (bytecode_offset=0):
    // offset 0:  PushI.i 42   (value)           4 bytes
    // offset 4:  PushI.i -1   (dim2, don't care) 4 bytes
    // offset 8:  PushI.i 0    (dim1 = index)     4 bytes
    // offset 12: Pop.v.v var  (2D array write)   8 bytes
    // offset 20: Exit                             4 bytes
    let instructions = vec![
        pushi(42),
        pushi(-1),
        pushi(0),      // dim1 = 0, non-negative → non-scalar
        pop_var(3, 0), // ref_type=0, instance>=0 → 2D array
        exit_inst(),
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![("myarray".into(), -1)];
    // Pop.v.v is at decoded offset 12 (4+4+4 bytes before it).
    let vari_ref_map: HashMap<usize, usize> = [(12, 0)].into_iter().collect();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec!["Obj0".into(); 4]; // 4 so index 3 is valid
    let script_names: HashSet<String> = HashSet::new();
    let mut ctx = make_ctx(
        true,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );
    // instance=3 in bytecode is the VARI owner (self) — set self_object_index so
    // is_own resolves correctly with the new instance-check logic.
    ctx.self_object_index = Some(3);

    let (func, _) = translate_code_entry(&bytecode, "test_fn", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    let has_set_index = ops.iter().any(|op| matches!(op, Op::SetIndex { .. }));
    let has_set_field = ops.iter().any(|op| matches!(op, Op::SetField { .. }));
    assert!(
        has_set_index,
        "expected SetIndex for non-scalar 2D array write; ops: {ops:?}"
    );
    assert!(
        !has_set_field,
        "unexpected SetField for non-scalar 2D array write; ops: {ops:?}"
    );
}

/// Scalar 2D array write: `myfield[# -1, -1] = 42` (dim1 = -1 → scalar access).
///
/// When dim1 == -1 (the "don't care" sentinel), the translator treats the access
/// as a plain field write: `SetField(self, "myfield", 42)`.
#[test]
fn test_2d_array_write_scalar_emits_set_field() {
    // offset 0:  PushI.i 42   (value)
    // offset 4:  PushI.i -1   (dim2)
    // offset 8:  PushI.i -1   (dim1 = -1 → scalar)
    // offset 12: Pop.v.v var
    // offset 20: Exit
    let instructions = vec![
        pushi(42),
        pushi(-1),
        pushi(-1), // dim1 = -1 → is_scalar = true
        pop_var(3, 0),
        exit_inst(),
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![("myfield".into(), -1)];
    let vari_ref_map: HashMap<usize, usize> = [(12, 0)].into_iter().collect();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec!["Obj0".into(); 4];
    let script_names: HashSet<String> = HashSet::new();
    let mut ctx = make_ctx(
        true,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );
    // instance=3 in bytecode is the VARI owner (self) — set self_object_index so
    // is_own resolves correctly with the new instance-check logic.
    ctx.self_object_index = Some(3);

    let (func, _) = translate_code_entry(&bytecode, "test_fn", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    let has_set_field = ops
        .iter()
        .any(|op| matches!(op, Op::SetField { field, .. } if field == "myfield"));
    let has_set_index = ops.iter().any(|op| matches!(op, Op::SetIndex { .. }));
    assert!(
        has_set_field,
        "expected SetField for scalar 2D array write; ops: {ops:?}"
    );
    assert!(
        !has_set_index,
        "unexpected SetIndex for scalar 2D array write; ops: {ops:?}"
    );
}

// -----------------------------------------------------------------------
// argument[N] variable mapping — 2D array pattern (GMS1)
// -----------------------------------------------------------------------

/// Build a Dup instruction.
fn dup_inst(n: u16, type1: DataType) -> Instruction {
    Instruction {
        offset: 0,
        opcode: Opcode::Dup,
        type1,
        type2: DataType::Double,
        operand: Operand::Dup(n),
    }
}

/// Build an Add instruction (no operand, operates on stack).
fn add_inst(type1: DataType) -> Instruction {
    Instruction {
        offset: 0,
        opcode: Opcode::Add,
        type1,
        type2: DataType::Double,
        operand: Operand::None,
    }
}

// -----------------------------------------------------------------------
// 2D array compound assignment — correct stack layout (value on top)
// -----------------------------------------------------------------------

/// Compound 2D array write: `myarray[5] += 10`
///
/// GML bytecode for compound assignment uses the Dup pattern:
///   push dim2 (artifact), push dim1 (index), Dup, Push.v.v (read), arithmetic, Pop.v.v (write)
///
/// After the Dup+read+arithmetic, the stack is `[dim2, dim1, new_value]` with
/// new_value on TOP — the OPPOSITE of simple assignment `[value, dim2, dim1]`.
/// The `compound_2d_pending` flag, set by translate_push_variable, causes
/// translate_pop to use the reversed pop order: value=top, dim1=next, dim2=bottom.
///
/// This test guards that:
/// 1. A `SetIndex` (not `SetField`) is emitted — index was non-scalar.
/// 2. The index passed to `SetIndex` is the ORIGINAL dim1 constant (5), not
///    the Add result — confirming the new_value is stored, not used as index.
#[test]
fn test_2d_array_compound_write_uses_correct_operands() {
    // Bytecode sequence for `myarray[5] += 10`:
    // offset  0: PushI.i16 3  (dim2 artifact, 1 unit)    → 4 bytes
    // offset  4: PushI.i16 5  (dim1 = array index 5)     → 4 bytes
    // offset  8: Dup.i16 1    (dup top 2 items: 2 units)  → 4 bytes
    // offset 12: Push.v.v     (VARI read: pops dim1_copy+dim2_copy, pushes current) → 8 bytes
    // offset 20: PushI.i16 10 (value to add)              → 4 bytes
    // offset 24: Add.i16      (sum = current + 10)        → 4 bytes
    // offset 28: Pop.v.v      (VARI write, compound)      → 8 bytes
    // offset 36: Exit                                      → 4 bytes
    let instructions = vec![
        pushi(-1),                    // dim2 = -1 (self scope; GMS1 1D self-array sentinel)
        pushi(5),                     // dim1 = index
        dup_inst(1, DataType::Int16), // dup top 2 Int16 items (2 * 1 unit each)
        push_var(3, 0),               // 2D VARI read (ref_type=0, instance=3 ≥ 0)
        pushi(10),                    // value to add
        add_inst(DataType::Int16),    // sum
        pop_var(3, 0),                // 2D VARI write (same variable)
        exit_inst(),
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![("myarray".into(), -1)];
    // Push.v.v is at offset 12, Pop.v.v is at offset 28.
    let vari_ref_map: HashMap<usize, usize> = [(12, 0), (28, 0)].into_iter().collect();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec!["Obj0".into(); 4]; // index 3 valid
    let script_names: HashSet<String> = HashSet::new();
    let ctx = make_ctx(
        true,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );

    let (func, _) =
        translate_code_entry(&bytecode, "test_compound_2d", &ctx).expect("translation failed");

    // Collect (op, result_value_id) pairs to trace operand relationships.
    let insts: Vec<_> = func.insts.values().collect();

    // Find the SetIndex instruction.
    let set_index = insts.iter().find(|i| matches!(i.op, Op::SetIndex { .. }));
    assert!(
        set_index.is_some(),
        "expected SetIndex for compound 2D write; ops: {:?}",
        insts.iter().map(|i| &i.op).collect::<Vec<_>>()
    );

    let Op::SetIndex { index, value, .. } = &set_index.unwrap().op else {
        unreachable!()
    };

    // The index must be the constant 5 (original dim1), NOT the Add result.
    let index_is_const_5 = insts.iter().any(|i| {
        i.result == Some(*index)
            && matches!(i.op, Op::Const(reincarnate_core::ir::Constant::Int(5)))
    });
    assert!(index_is_const_5,
        "SetIndex index should be dim1=const(5), not the Add result; index={index:?}, ops={insts:?}");

    // The value must NOT be the constant 10 or 3 — it should be the Add result.
    let value_is_plain_const = insts
        .iter()
        .any(|i| i.result == Some(*value) && matches!(i.op, Op::Const(_)));
    assert!(
        !value_is_plain_const,
        "SetIndex value should be the Add result (sum), not a plain constant; value={value:?}"
    );
}

// -----------------------------------------------------------------------
// PushEnv / PopEnv — with-block post-continuation must be reachable
// -----------------------------------------------------------------------

/// Post-with code (instructions after PopEnv) must appear in the IR.
///
/// Before the fix, PopEnv's loop-back case emitted `br body_block` (an
/// unconditional loop-back), leaving the fall-through block unreachable.
/// After the fix, PopEnv always falls through to the next instruction so
/// that the post-with code is reachable.
///
/// Bytecode layout:
///   offset  0: PushI.i16 5              — target object
///   offset  4: PushEnv Branch(12)       — skip to PopEnv at offset 16
///   offset  8: PushI.i16 0              — body: push a value
///   offset 12: Popz                     — body: pop it
///   offset 16: PopEnv Branch(-8)        — loop-back sentinel (16-8=8 ≥ 0)
///   offset 20: PushI.i16 99             — post-with sentinel
///   offset 24: Popz                     — discard sentinel
///   offset 28: Ret
#[test]
fn test_popenv_fall_through_reaches_post_with_code() {
    fn popz_inst() -> Instruction {
        Instruction {
            offset: 0,
            opcode: Opcode::Popz,
            type1: DataType::Int16,
            type2: DataType::Double,
            operand: Operand::None,
        }
    }
    fn branch_inst(opcode: Opcode, byte_offset: i32) -> Instruction {
        Instruction {
            offset: 0,
            opcode,
            type1: DataType::Double,
            type2: DataType::Double,
            operand: Operand::Branch(byte_offset),
        }
    }

    // PushEnv Branch(12): skip body, target = offset 4+12=16 (PopEnv).
    // PopEnv Branch(-8): loop-back, sentinel = 16+(-8)=8 ≥ 0.
    let instructions = vec![
        pushi(5),                         // offset  0: push target
        branch_inst(Opcode::PushEnv, 12), // offset  4: skip to offset 16
        pushi(0),                         // offset  8: body push
        popz_inst(),                      // offset 12: body pop
        branch_inst(Opcode::PopEnv, -8),  // offset 16: loop-back to offset 8
        pushi(99),                        // offset 20: post-with sentinel
        popz_inst(),                      // offset 24: pop sentinel
        Instruction {
            // offset 28: Exit (void return)
            offset: 0,
            opcode: Opcode::Exit,
            type1: DataType::Double,
            type2: DataType::Double,
            operand: Operand::None,
        },
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![];
    let vari_ref_map: HashMap<usize, usize> = HashMap::new();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec![];
    let script_names: HashSet<String> = HashSet::new();
    let ctx = make_ctx(
        false,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );

    let (func, extra_funcs) = translate_code_entry(&bytecode, "test_with_continuation", &ctx)
        .expect("translation failed");
    let ops = collect_ops(&func);

    // MakeClosure must appear (PushEnv → closure extraction).
    let has_make_closure = ops.iter().any(|op| matches!(op, Op::MakeClosure { .. }));
    assert!(
        has_make_closure,
        "MakeClosure must appear for with-block; ops: {ops:?}"
    );

    // withInstances syscall must appear.
    let has_with_instances = ops.iter().any(|op| {
        matches!(op, Op::SystemCall { system, method, .. }
            if system == "GameMaker.Instance" && method == "withInstances")
    });
    assert!(
        has_with_instances,
        "withInstances syscall must appear; ops: {ops:?}"
    );

    // An extra closure function must have been extracted.
    assert_eq!(
        extra_funcs.len(),
        1,
        "expected 1 closure function; got {}",
        extra_funcs.len()
    );
    assert_eq!(
        extra_funcs[0].method_kind,
        reincarnate_core::ir::func::MethodKind::Closure,
        "extra function must be MethodKind::Closure"
    );

    // Post-with sentinel (Const 99) must be reachable in the outer function.
    let has_post_with_sentinel = ops.iter().any(|op| {
        matches!(
            op,
            Op::Const(reincarnate_core::ir::value::Constant::Int(99))
        )
    });
    assert!(
        has_post_with_sentinel,
        "post-with code (Const(99) sentinel) must be reachable; ops: {ops:?}"
    );
}

/// `argument[1]` push with 2D array encoding maps to `fb.param(1)`, not a
/// heap allocation or runtime lookup.
///
/// GMS1 encodes `argument[N]` as a 2D array read with `ref_type=0`:
///   PushI -1 (dim2), PushI N (dim1), Push.v.v argument
/// The translator must recognize this and map it to the Nth function parameter,
/// not emit a `SystemCall("GameMaker.Instance", "getField", ...)`.
#[test]
fn test_argument_2d_array_push_maps_to_param() {
    // offset 0: PushI.i -1     (dim2, don't care)   4 bytes
    // offset 4: PushI.i 1      (dim1 = argument[1]) 4 bytes
    // offset 8: Push.v.v arg   (2D array read)       8 bytes
    // offset 16: Ret                                  4 bytes
    let instructions = vec![
        pushi(-1),
        pushi(1),       // dim1 = 1 → argument index 1
        push_var(0, 0), // ref_type=0, instance=0 → 2D array
        ret_inst(),
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![("argument".into(), -2)]; // -2 = Builtin
                                                                  // Push.v.v is at decoded offset 8.
    let vari_ref_map: HashMap<usize, usize> = [(8, 0)].into_iter().collect();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec!["Obj0".into()];
    let script_names: HashSet<String> = HashSet::new();
    // No has_self; arg_count=0 (scan_implicit_args will detect argument[1]).
    let ctx = make_ctx(
        false,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );

    let (func, _) = translate_code_entry(&bytecode, "test_fn", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    // Must NOT fall back to a SystemCall (getField / getOn).
    let has_syscall = ops.iter().any(|op| {
        matches!(op, Op::SystemCall { system, method, .. }
            if system == "GameMaker.Instance" && (method == "getField" || method == "getOn"))
    });
    assert!(
        !has_syscall,
        "argument[N] must map to param, not syscall; ops: {ops:?}"
    );

    // The function must have been given 2 params (argument0, argument1)
    // because scan_implicit_args detected argument[1].
    assert_eq!(
        func.sig.params.len(),
        2,
        "expected 2 params for implicit argument[1]"
    );
}

// -----------------------------------------------------------------------
// Self-field read (Push.v.v with instance=Own)
// -----------------------------------------------------------------------

/// Reading a self-field (instance=-1, Own) produces `GetField(self, "hp")`.
#[test]
fn test_self_field_read_emits_get_field() {
    let instructions = vec![
        pushi(-1),      // dim2 = -1 (self scope)
        pushi(-1),      // dim1 = -1 (scalar)
        push_var(0, 0), // ref_type=0, instance=0 → 2D array
        ret_inst(),
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![("hp".into(), -1)];
    let vari_ref_map: HashMap<usize, usize> = [(8, 0)].into_iter().collect();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec![];
    let script_names: HashSet<String> = HashSet::new();
    let ctx = make_ctx(
        true,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );

    let (func, _) =
        translate_code_entry(&bytecode, "test_self_read", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    let has_get_field = ops
        .iter()
        .any(|op| matches!(op, Op::GetField { field, .. } if field == "hp"));
    assert!(
        has_get_field,
        "self-field read must produce GetField(\"hp\"); ops: {ops:?}"
    );

    let has_return_with_value = ops.iter().any(|op| matches!(op, Op::Return(Some(_))));
    assert!(
        has_return_with_value,
        "Ret must produce Return(Some(_)); ops: {ops:?}"
    );
}

// -----------------------------------------------------------------------
// Self-field write (Pop.v.v with instance=Own)
// -----------------------------------------------------------------------

/// Writing a self-field: `self.hp = 100` emits SetField(self, "hp", 100).
#[test]
fn test_self_field_write_emits_set_field() {
    let instructions = vec![pushi(100), pushi(-1), pushi(-1), pop_var(0, 0), exit_inst()];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![("hp".into(), -1)];
    let vari_ref_map: HashMap<usize, usize> = [(12, 0)].into_iter().collect();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec![];
    let script_names: HashSet<String> = HashSet::new();
    let ctx = make_ctx(
        true,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );

    let (func, _) =
        translate_code_entry(&bytecode, "test_self_write", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    let has_set_field = ops
        .iter()
        .any(|op| matches!(op, Op::SetField { field, .. } if field == "hp"));
    assert!(
        has_set_field,
        "self-field write must produce SetField(\"hp\"); ops: {ops:?}"
    );
}

// -----------------------------------------------------------------------
// Function call (Call opcode)
// -----------------------------------------------------------------------

/// `Call show_debug_message(42)` emits `Op::Call`.
#[test]
fn test_call_opcode_emits_ir_call() {
    let instructions = vec![
        pushi(42),
        Instruction {
            offset: 0,
            opcode: Opcode::Call,
            type1: DataType::Int32,
            type2: DataType::Double,
            operand: Operand::Call {
                function_id: 0,
                argc: 1,
            },
        },
        Instruction {
            offset: 0,
            opcode: Opcode::Popz,
            type1: DataType::Variable,
            type2: DataType::Double,
            operand: Operand::None,
        },
        exit_inst(),
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![];
    let vari_ref_map: HashMap<usize, usize> = HashMap::new();
    let fn_names: HashMap<u32, String> = [(0, "show_debug_message".to_string())]
        .into_iter()
        .collect();
    let func_ref_map: HashMap<usize, usize> = [(4, 0)].into_iter().collect();
    let obj_names: Vec<String> = vec![];
    let script_names: HashSet<String> = HashSet::new();
    let ctx = make_ctx(
        false,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );

    let (func, _) = translate_code_entry(&bytecode, "test_call", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    let has_call = ops.iter().any(|op| {
        matches!(op, Op::Call { func, args } if func == "show_debug_message" && args.len() == 1)
    });
    assert!(
        has_call,
        "Call opcode must produce Op::Call with correct name and 1 arg; ops: {ops:?}"
    );
}

// -----------------------------------------------------------------------
// Arithmetic: Add, Sub, Mul
// -----------------------------------------------------------------------

#[test]
fn test_arithmetic_add_sub_mul() {
    fn sub_inst(type1: DataType) -> Instruction {
        Instruction {
            offset: 0,
            opcode: Opcode::Sub,
            type1,
            type2: DataType::Double,
            operand: Operand::None,
        }
    }
    fn mul_inst(type1: DataType) -> Instruction {
        Instruction {
            offset: 0,
            opcode: Opcode::Mul,
            type1,
            type2: DataType::Double,
            operand: Operand::None,
        }
    }

    let instructions = vec![
        pushi(3),
        pushi(5),
        add_inst(DataType::Int16),
        pushi(10),
        pushi(2),
        sub_inst(DataType::Int16),
        add_inst(DataType::Int16),
        pushi(4),
        pushi(7),
        mul_inst(DataType::Int16),
        add_inst(DataType::Int16),
        ret_inst(),
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![];
    let vari_ref_map: HashMap<usize, usize> = HashMap::new();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec![];
    let script_names: HashSet<String> = HashSet::new();
    let ctx = make_ctx(
        false,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );

    let (func, _) =
        translate_code_entry(&bytecode, "test_arith", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    let add_count = ops.iter().filter(|op| matches!(op, Op::Add(_, _))).count();
    let sub_count = ops.iter().filter(|op| matches!(op, Op::Sub(_, _))).count();
    let mul_count = ops.iter().filter(|op| matches!(op, Op::Mul(_, _))).count();
    assert_eq!(add_count, 3, "expected 3 Add ops; ops: {ops:?}");
    assert_eq!(sub_count, 1, "expected 1 Sub op; ops: {ops:?}");
    assert_eq!(mul_count, 1, "expected 1 Mul op; ops: {ops:?}");
}

// -----------------------------------------------------------------------
// Comparison (Cmp with Equal)
// -----------------------------------------------------------------------

#[test]
fn test_cmp_equal_emits_cmp_eq() {
    use datawin::bytecode::types::ComparisonKind;
    use reincarnate_core::ir::inst::CmpKind;

    let instructions = vec![
        pushi(10),
        pushi(20),
        Instruction {
            offset: 0,
            opcode: Opcode::Cmp,
            type1: DataType::Int16,
            type2: DataType::Int16,
            operand: Operand::Comparison(ComparisonKind::Equal),
        },
        ret_inst(),
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![];
    let vari_ref_map: HashMap<usize, usize> = HashMap::new();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec![];
    let script_names: HashSet<String> = HashSet::new();
    let ctx = make_ctx(
        false,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );

    let (func, _) = translate_code_entry(&bytecode, "test_cmp", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    let has_cmp_eq = ops
        .iter()
        .any(|op| matches!(op, Op::Cmp(CmpKind::Eq, _, _)));
    assert!(
        has_cmp_eq,
        "Cmp with Equal must produce Op::Cmp(Eq, ..); ops: {ops:?}"
    );
}

// -----------------------------------------------------------------------
// Conditional branch (Bf producing BrIf)
// -----------------------------------------------------------------------

#[test]
fn test_bf_produces_br_if() {
    fn branch_inst(opcode: Opcode, byte_offset: i32) -> Instruction {
        Instruction {
            offset: 0,
            opcode,
            type1: DataType::Double,
            type2: DataType::Double,
            operand: Operand::Branch(byte_offset),
        }
    }

    let instructions = vec![
        pushi(1),                    // offset  0: condition
        branch_inst(Opcode::Bf, 16), // offset  4: Bf → offset 20
        pushi(10),                   // offset  8: then value
        ret_inst(),                  // offset 12: return from then
        branch_inst(Opcode::B, 8),   // offset 16: B → offset 24 (skip else)
        pushi(20),                   // offset 20: else value
        ret_inst(),                  // offset 24: return from else
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![];
    let vari_ref_map: HashMap<usize, usize> = HashMap::new();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec![];
    let script_names: HashSet<String> = HashSet::new();
    let ctx = make_ctx(
        false,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );

    let (func, _) = translate_code_entry(&bytecode, "test_bf", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    let has_br_if = ops.iter().any(|op| matches!(op, Op::BrIf { .. }));
    assert!(has_br_if, "Bf must produce BrIf; ops: {ops:?}");

    let return_count = ops
        .iter()
        .filter(|op| matches!(op, Op::Return(Some(_))))
        .count();
    assert!(
        return_count >= 2,
        "both branches must return; got {return_count} returns; ops: {ops:?}"
    );
}

// -----------------------------------------------------------------------
// String constant push
// -----------------------------------------------------------------------

#[test]
fn test_push_string_constant() {
    use reincarnate_core::ir::value::Constant;

    let instructions = vec![
        Instruction {
            offset: 0,
            opcode: Opcode::Push,
            type1: DataType::String,
            type2: DataType::Double,
            operand: Operand::StringIndex(0),
        },
        ret_inst(),
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![];
    let vari_ref_map: HashMap<usize, usize> = HashMap::new();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec![];
    let script_names: HashSet<String> = HashSet::new();
    let string_table = vec!["hello".to_string()];
    let mut ctx = make_ctx(
        false,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );
    ctx.string_table = &string_table;

    let (func, _) =
        translate_code_entry(&bytecode, "test_string", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    let has_hello = ops
        .iter()
        .any(|op| matches!(op, Op::Const(Constant::String(s)) if s == "hello"));
    assert!(
        has_hello,
        "Push.String must produce Const(String(\"hello\")); ops: {ops:?}"
    );
}

// -----------------------------------------------------------------------
// Float constant push
// -----------------------------------------------------------------------

#[test]
fn test_push_double_constant() {
    use reincarnate_core::ir::value::Constant;

    let instructions = vec![
        Instruction {
            offset: 0,
            opcode: Opcode::Push,
            type1: DataType::Double,
            type2: DataType::Double,
            operand: Operand::Double(2.75),
        },
        ret_inst(),
    ];
    let bytecode = encode(&instructions);

    let vars: Vec<(String, i32)> = vec![];
    let vari_ref_map: HashMap<usize, usize> = HashMap::new();
    let fn_names: HashMap<u32, String> = HashMap::new();
    let func_ref_map: HashMap<usize, usize> = HashMap::new();
    let obj_names: Vec<String> = vec![];
    let script_names: HashSet<String> = HashSet::new();
    let ctx = make_ctx(
        false,
        0,
        &vars,
        &vari_ref_map,
        &func_ref_map,
        &obj_names,
        &fn_names,
        &script_names,
    );

    let (func, _) =
        translate_code_entry(&bytecode, "test_double", &ctx).expect("translation failed");
    let ops = collect_ops(&func);

    let has_float = ops
        .iter()
        .any(|op| matches!(op, Op::Const(Constant::Float(v)) if (*v - 2.75).abs() < f64::EPSILON));
    assert!(
        has_float,
        "Push.Double must produce Const(Float(2.75)); ops: {ops:?}"
    );
}
