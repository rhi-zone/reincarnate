use std::collections::{HashMap, HashSet};

use datawin::bytecode::decode::{self, Instruction, Operand};
use datawin::bytecode::opcode::Opcode;
use datawin::bytecode::types::{ComparisonKind, DataType, InstanceType, VariableRef};
use reincarnate_core::entity::EntityRef as _;
use reincarnate_core::ir::block::BlockId;
use reincarnate_core::ir::builder::FunctionBuilder;
use reincarnate_core::ir::func::{Function, Visibility};
use reincarnate_core::ir::inst::CmpKind;
use reincarnate_core::ir::ty::{FunctionSig, Type, TypeId, TypeVarId};
use reincarnate_core::ir::value::ValueId;

mod cfg;
mod ops;
pub(crate) mod switch;
#[cfg(test)]
mod tests;
mod variable_access;
pub(crate) mod with_body;

use cfg::{get_branch_args, setup_blocks};
use ops::translate_instruction;
use switch::detect_switches;
use with_body::{
    find_with_ranges, has_exit_popenv, scan_body_argument_indices, scan_body_local_names,
    scan_body_uses_other, translate_with_body, WithBodyCtx,
};

/// Context for translating a single code entry.
pub struct TranslateCtx<'a> {
    /// FUNC function entries: entry_index → resolved name.
    pub function_names: &'a HashMap<u32, String>,
    /// GMS2.3+ pushref asset name map: (type_tag << 24) | asset_idx → raw GML name.
    /// Built from SPRT (type 1), SOND (type 2), BGND (type 3), SCPT (type 5),
    /// FONT (type 6), SHDR (type 8), ROOM (type 9).
    pub asset_ref_names: &'a HashMap<u32, String>,
    /// VARI variable entries: entry_index → (name, instance_type).
    pub variables: &'a [(String, i32)],
    /// FUNC linked-list reference map: absolute bytecode address → func entry index.
    pub func_ref_map: &'a HashMap<usize, usize>,
    /// VARI linked-list reference map: absolute bytecode address → vari entry index.
    pub vari_ref_map: &'a HashMap<usize, usize>,
    /// Absolute file offset where this code entry's bytecode begins.
    pub bytecode_offset: usize,
    /// Pre-resolved local variable names: `(local_index, name)` pairs.
    /// Derived from `CodeLocals` by the caller before constructing `TranslateCtx`.
    /// Empty when no debug info is available.
    pub local_names: &'a [(u32, String)],
    /// Pre-resolved string table (STRG chunk), indexed by string id.
    /// Used for `Push StringIndex(idx)` instructions.
    pub string_table: &'a [String],
    /// Whether this is an instance method (has self param).
    pub has_self: bool,
    /// Whether this is a collision event (has other param).
    pub has_other: bool,
    /// Number of declared arguments.
    pub arg_count: u16,
    /// Object names indexed by object ID (for resolving numeric instance IDs).
    pub obj_names: &'a [String],
    /// Class name for event handlers (used to type the self parameter).
    pub class_name: Option<&'a str>,
    /// Object index of the owning object (for recognizing self-references).
    /// The GameMaker compiler often uses the object's own index as the instance
    /// type instead of -1 (Own). When `instance >= 0` and matches this index,
    /// the access should be treated as `self.field`, not a cross-object reference.
    pub self_object_index: Option<usize>,
    /// Object indices of all ancestors in the parent chain.
    /// When `instance >= 0` matches any ancestor, the access is also self (inherited field).
    pub ancestor_indices: HashSet<usize>,
    /// Set of clean script names (for injecting self at call sites).
    pub script_names: &'a HashSet<String>,
    /// True when translating a with-body closure (extracted from a PushEnv/PopEnv pair).
    /// In this context, a PopEnv instruction is an early-exit signal — the outer with-loop
    /// is managed by `withInstances`, so we do NOT emit `withEnd()` for PopEnv.
    pub is_with_body: bool,
    /// True when the with-body closure uses the "return X inside with" pattern
    /// (the closure has a sentinel-offset exit PopEnv as its *sole* exit path).
    /// When set, sentinel-offset PopEnv inside the closure falls through to the
    /// continuation block (which loads the return value and emits `return v`)
    /// instead of emitting `return void`.
    pub with_body_has_return: bool,
    /// Bytecode version from GEN8. Used to guard version-specific behaviours
    /// (GMS2.3+ Break signals, Dup swap-mode encoding, etc.).
    pub bytecode_version: datawin::BytecodeVersion,
    /// Pre-interned ClassRef TypeIds: object name → TypeId.
    ///
    /// Used by the GMS2.3+ `Break -11` (pushref) instruction translator so it
    /// can emit `Type::ClassRef(TypeId)` without needing direct module access.
    /// Callers pre-populate this by calling `mb.intern_type_classref(name)` for
    /// each object name before translation begins.
    pub classref_types: &'a HashMap<String, TypeId>,
    /// Pre-interned Instance TypeIds: object name → TypeId.
    ///
    /// Used to type `self` parameters and `with`-body `_self` parameters as
    /// `Type::Instance(TypeId)` without needing direct module access.
    /// Callers pre-populate this by calling `mb.intern_type(name)` for each
    /// object name before translation begins.
    pub instance_types: &'a HashMap<String, TypeId>,
}

/// Translate a single code entry's bytecode into an IR Function.
///
/// Returns `(main_func, extra_funcs)` where `extra_funcs` are closure
/// functions extracted from `with`-block bodies (one per PushEnv/PopEnv pair).
pub fn translate_code_entry(
    bytecode: &[u8],
    func_name: &str,
    ctx: &TranslateCtx,
) -> Result<(Function, Vec<Function>), String> {
    let all_instructions = decode::decode(bytecode).map_err(|e| format!("{func_name}: {e}"))?;
    if all_instructions.is_empty() {
        let func = build_empty_function(func_name, ctx)?;
        return Ok((func, vec![]));
    }

    // Filter to only instructions reachable from the entry point.
    // In GMS2.3+ shared bytecode blobs, the decoded range may include
    // sibling functions' code beyond this function's Ret/Exit. Without
    // filtering, their branches create spurious block starts that cause
    // stack underflows.
    let instructions = cfg::filter_reachable(&all_instructions);

    // Pre-detect with-block ranges so we can exclude their blocks from the outer CFG.
    let with_ranges = find_with_ranges(&instructions);

    // Pass 1 & 2: Create IR blocks, excluding with-body offsets.
    // Old-style scripts may use argumentN without declaring parameters —
    // scan for implicit argument references to determine true arg count.
    let scan = scan_implicit_args(&instructions, ctx);
    let effective_arg_count = ctx.arg_count.max(scan.count).max(scan.global_arg_count);
    let global_arg_count = scan.global_arg_count;
    let mut sig = build_signature_with_args(ctx, effective_arg_count);
    // Scripts that read `argument_count` or use `argument[dynamic_idx]` are truly
    // variadic — they accept any number of arguments at the call site.  Emit a
    // rest parameter `...args: any[]` so TypeScript call sites are not flagged for
    // passing extra arguments.
    if scan.uses_dynamic_args {
        // Mark as variadic with a rest param.  Type-appropriate defaults for the
        // fixed argument params are set later by GmlDefaultArgRecovery (which runs
        // post-inference and can match defaults to narrowed param types).
        sig.params.push(Type::Array(Box::new(Type::Unknown)));
        sig.defaults.push(None);
        sig.has_rest_param = true;
    }
    let mut fb = FunctionBuilder::new(func_name, sig, Visibility::Public);

    // Name parameters.
    let mut param_idx = 0;
    if ctx.has_self {
        fb.name_value(fb.param(param_idx), "self".to_string());
        param_idx += 1;
    }
    if ctx.has_other {
        fb.name_value(fb.param(param_idx), "other".to_string());
        param_idx += 1;
    }
    for i in 0..effective_arg_count {
        // For declared args, use debug names from code_locals.
        // For implicit args (argumentN pattern), always use argumentN —
        // code_locals indices can collide with unrelated locals.
        let name = if i < ctx.arg_count {
            arg_name(ctx, i).unwrap_or_else(|| format!("argument{i}"))
        } else {
            format!("argument{i}")
        };
        fb.name_value(fb.param(param_idx), name);
        param_idx += 1;
    }
    // If this is a variadic script, record the rest param ValueId in a special
    // locals entry so the translation loop can reference it when it encounters
    // `argument_count` reads or dynamic `argument[N]` accesses.
    let rest_param_id = if scan.uses_dynamic_args {
        let id = fb.param(param_idx);
        fb.name_value(id, "args".to_string());
        Some(id)
    } else {
        None
    };

    let (block_map, block_params, block_entry_depths) = setup_blocks(
        &mut fb,
        &instructions,
        &with_ranges,
        0,
        ctx.function_names,
        ctx.bytecode_offset,
        ctx.func_ref_map,
    );

    // Allocate locals.
    let mut locals = allocate_locals(&mut fb, ctx);
    // Stash the rest param (if any) in locals under the reserved key "_args" so
    // inner translation helpers can look it up without adding extra parameters.
    if let Some(rest_id) = rest_param_id {
        locals.insert("_args".to_string(), rest_id);
    }

    // Pass 3: Translate instructions.
    fb.switch_to_block(fb.entry_block());
    let mut extra_funcs = Vec::new();
    let terminated = run_translation_loop(
        &instructions,
        func_name,
        &mut fb,
        &block_map,
        &block_params,
        &block_entry_depths,
        &with_ranges,
        &mut locals,
        ctx,
        &mut extra_funcs,
        global_arg_count,
    )?;

    // If the last block wasn't terminated, add a void return.
    if !terminated {
        fb.ret(None);
    }

    let mut func = fb.build();
    detect_switches(&mut func);
    Ok((func, extra_funcs))
}

// ---------------------------------------------------------------------------
// Signature and helpers
// ---------------------------------------------------------------------------

/// Build function signature from context.
fn build_signature(ctx: &TranslateCtx) -> FunctionSig {
    build_signature_with_args(ctx, ctx.arg_count)
}

fn build_signature_with_args(ctx: &TranslateCtx, arg_count: u16) -> FunctionSig {
    // Pre-builder type variable counter: used before FunctionBuilder is available.
    // The constraint solver ignores TypeVarId numeric values — only is_concrete()
    // matters — so per-invocation collisions are safe.
    let mut tv: u32 = 0;
    let mut fresh = || {
        let ty = Type::Var(TypeVarId::new(tv));
        tv += 1;
        ty
    };

    let mut params = Vec::new();
    let mut defaults = Vec::new();
    if ctx.has_self {
        // Use the declared class type if available; fall back to GMLObject so
        // that script functions (class_name = None) get a typed self rather
        // than unknown, enabling property access type-checking.
        let self_ty = ctx
            .class_name
            .and_then(|name| ctx.instance_types.get(name).copied())
            .or_else(|| ctx.instance_types.get("GMLObject").copied())
            .map(Type::Instance)
            .unwrap_or_else(&mut fresh);
        params.push(self_ty);
        defaults.push(None);
    }
    if ctx.has_other {
        // `other` is always a GML object instance (e.g. collision partner).
        let other_ty = ctx
            .instance_types
            .get("GMLObject")
            .copied()
            .map(Type::Instance)
            .unwrap_or_else(&mut fresh);
        params.push(other_ty);
        defaults.push(None);
    }
    for _ in 0..arg_count {
        params.push(fresh());
        defaults.push(None);
    }
    FunctionSig {
        params,
        defaults,
        return_ty: fresh(),
        ..Default::default()
    }
}

/// Parse `argumentN` variable names and return the index N, if any.
fn parse_argument_index(name: &str) -> Option<usize> {
    name.strip_prefix("argument")
        .and_then(|s| s.parse::<usize>().ok())
}

/// Result of scanning for implicit argument references in a GML script.
struct ImplicitArgScan {
    /// Number of implicit arguments detected via `argumentN` or `argument[K]` with
    /// a constant index (max index + 1), or 0 if none found.
    count: u16,
    /// True when the script uses dynamic argument access: reads `argument_count`
    /// (to determine how many args were passed at runtime) or reads `argument[N]`
    /// with a non-constant index. Scripts with dynamic argument access must be
    /// emitted with a rest parameter (`...args: any[]`) so TypeScript call sites
    /// can pass any number of arguments without TS2554 errors.
    uses_dynamic_args: bool,
    /// Number of arguments detected via `InstanceType::Global` references to
    /// `argumentN` variables.  In GMS2.3+, the VM copies call-stack arguments
    /// into `global.argument0`..`global.argumentN` before entering the function,
    /// so the bytecode reads/writes them as globals rather than using
    /// `InstanceType::Own`/`Builtin`.  When this is non-zero, the translation
    /// loop must rewrite `global.argumentN` accesses to use formal parameters
    /// instead of emitting `GameMaker.Global.get/set` calls.
    global_arg_count: u16,
}

/// Scan instructions for implicit `argument0`..`argumentN` references
/// (variables with Builtin/Own instance type whose name matches `argumentN`)
/// and `argument[N]` references (Stacktop instance type with name "argument"
/// preceded by a constant integer push).
///
/// Also detects dynamic argument access patterns (`argument_count` reads or
/// `argument[N]` with a non-constant index) which require a rest parameter.
fn scan_implicit_args(instructions: &[Instruction], ctx: &TranslateCtx) -> ImplicitArgScan {
    let mut max_idx: Option<usize> = None;
    let mut global_max_idx: Option<usize> = None;
    let mut uses_dynamic_args = false;
    for (i, inst) in instructions.iter().enumerate() {
        if let Operand::Variable { var_ref, instance } = &inst.operand {
            let it = InstanceType::from_i16(*instance);
            if matches!(it, Some(InstanceType::Own) | Some(InstanceType::Builtin)) {
                let name = resolve_variable_name(inst, ctx);
                if let Some(idx) = parse_argument_index(&name) {
                    max_idx = Some(max_idx.map_or(idx, |m: usize| m.max(idx)));
                } else if name == "argument_count" {
                    // Script reads how many arguments were passed — must be variadic.
                    uses_dynamic_args = true;
                }
            } else if matches!(it, Some(InstanceType::Global) | Some(InstanceType::Static)) {
                // GMS2.3+ pattern: arguments are passed via global/static variables.
                // The VM copies call-stack args into global.argument0..N before
                // entering the function.  Static (-15) is used in GMS2.3+ for
                // argument references within struct/constructor functions.
                // Detect these so we can rewrite them to formal parameters in
                // the translation loop.
                let name = resolve_variable_name(inst, ctx);
                if let Some(idx) = parse_argument_index(&name) {
                    global_max_idx = Some(global_max_idx.map_or(idx, |m: usize| m.max(idx)));
                } else if name == "argument_count" {
                    uses_dynamic_args = true;
                }
            } else if matches!(it, Some(InstanceType::Stacktop)) {
                let name = resolve_variable_name(inst, ctx);
                if name == "argument" {
                    if let Some(idx) = preceding_const_int(instructions, i) {
                        // argument[N] pattern (GMS2): preceding instruction pushes the index.
                        let idx = idx as usize;
                        max_idx = Some(max_idx.map_or(idx, |m: usize| m.max(idx)));
                    } else {
                        // Unknown index — script accesses argument[variable].
                        uses_dynamic_args = true;
                    }
                }
            } else if is_2d_array_access(var_ref, *instance) {
                let name = resolve_variable_name(inst, ctx);
                if name == "argument" {
                    // argument[N] pattern (GM:S): 2D array access, dim2 is the index.
                    // Pattern: pushi -1, pushi N, push/pop [obj].argument
                    // dim2 is 1 instruction back from this one.
                    if let Some(idx) = preceding_const_int(instructions, i) {
                        let idx = idx as usize;
                        max_idx = Some(max_idx.map_or(idx, |m: usize| m.max(idx)));
                    } else {
                        // Unknown index — script accesses argument[variable].
                        uses_dynamic_args = true;
                    }
                }
            }
        }
    }
    ImplicitArgScan {
        count: max_idx.map_or(0, |m| (m + 1) as u16),
        uses_dynamic_args,
        global_arg_count: global_max_idx.map_or(0, |m| (m + 1) as u16),
    }
}

/// Extract a constant integer from the instruction preceding `idx`, if any.
fn preceding_const_int(instructions: &[Instruction], idx: usize) -> Option<i64> {
    if idx == 0 {
        return None;
    }
    let prev = &instructions[idx - 1];
    match prev.operand {
        Operand::Int16(v) => Some(v as i64),
        Operand::Int32(v) => Some(v as i64),
        Operand::Int64(v) => Some(v),
        _ => None,
    }
}

/// Get a name for argument index `i`.
fn arg_name(ctx: &TranslateCtx, i: u16) -> Option<String> {
    ctx.local_names
        .iter()
        .find(|(idx, _)| *idx == i as u32)
        .map(|(_, name)| name.clone())
}

/// Allocate local variable slots in the entry block.
fn allocate_locals(fb: &mut FunctionBuilder, ctx: &TranslateCtx) -> HashMap<String, ValueId> {
    let mut locals = HashMap::new();
    for (_, name) in ctx.local_names {
        let ty = fb.fresh_var();
        let slot = fb.alloc(ty);
        fb.name_value(slot, name.clone());
        locals.insert(name.clone(), slot);
    }
    locals
}

/// Build an empty function with just a void return.
fn build_empty_function(name: &str, ctx: &TranslateCtx) -> Result<Function, String> {
    let sig = build_signature(ctx);
    let mut fb = FunctionBuilder::new(name, sig, Visibility::Public);
    fb.ret(None);
    Ok(fb.build())
}

// ---------------------------------------------------------------------------
// Variable access helpers (shared across sub-modules)
// ---------------------------------------------------------------------------

/// Resolve a variable reference to its name using the VARI linked-list reference map.
///
/// The variable operand word is at `inst.offset + 4` within the code entry's bytecode.
/// We compute the absolute file address and look it up in the pre-built reference map.
fn resolve_variable_name(inst: &Instruction, ctx: &TranslateCtx) -> String {
    // first_address points to the instruction word; lookup by instruction address.
    let abs_addr = ctx.bytecode_offset + inst.offset;
    if let Some(&vari_idx) = ctx.vari_ref_map.get(&abs_addr) {
        if vari_idx < ctx.variables.len() {
            return ctx.variables[vari_idx].0.clone();
        }
    }
    format!("var_unknown_{:x}", inst.offset)
}

/// Map GML DataType to IR Type.
fn datatype_to_ir_type(dt: DataType) -> Type {
    match dt {
        DataType::Double => Type::Float(64),
        DataType::Float => Type::Float(32),
        DataType::Int32 => Type::Int(32),
        DataType::Int64 => Type::Int(64),
        DataType::Bool => Type::Bool,
        DataType::String => Type::String,
        _ => Type::Unknown,
    }
}

/// Map GML ComparisonKind to IR CmpKind.
fn comparison_to_cmp_kind(cmp: ComparisonKind) -> CmpKind {
    match cmp {
        ComparisonKind::Less => CmpKind::Lt,
        ComparisonKind::LessEqual => CmpKind::Le,
        ComparisonKind::Equal => CmpKind::Eq,
        ComparisonKind::NotEqual => CmpKind::Ne,
        ComparisonKind::GreaterEqual => CmpKind::Ge,
        ComparisonKind::Greater => CmpKind::Gt,
    }
}

/// Check if a variable operand represents a 2D array access.
///
/// Two index values (dim1, dim2) are popped from the stack before the
/// variable is accessed. This applies whenever the instance field is a
/// non-negative object ID — which covers:
///
/// - `ref_type == 0x00`: self-context 2D array access (GMS1/GMS2). The
///   `instance` field is the VARI table's scope owner (NOT the actual
///   target); the real target is always `self` in an event handler.
///
/// Cross-object accesses (`ref_type == 0x10`, etc.) with `instance >= 0`
/// are handled separately via `is_cross_obj_2d_read`, which additionally
/// requires that the next array-index signal in the stream is `pushaf` (not
/// `popaf`).  For WRITE context (popaf follows), the VM leaves `dim1` on the
/// stack for `popaf` to consume directly — the VARI Push itself is a plain
/// non-consuming field load.
///
/// `ref_type == 0x80` (stacktop) is excluded because it pops a *target
/// instance* from the stack instead of indices.
/// `ref_type == 0xA0` (singleton/Builtin) always uses `instance < 0`, so
/// it is excluded by the `instance >= 0` check.
fn is_2d_array_access(var_ref: &VariableRef, instance: i16) -> bool {
    instance >= 0 && var_ref.ref_type == 0
}

/// Returns `true` when the next pushaf/popaf signal in the remaining
/// instruction stream is `pushaf` (0xFFFE), indicating a READ context.
/// Returns `false` for `popaf` (WRITE context), block boundaries, Dup
/// instructions, or if no array-index signal is found before the end of the
/// basic block.
///
/// Dup triggers an early return of `false` because the only context in which
/// a Dup appears between a cross-object VARI Push and a pushaf is a compound
/// assignment (e.g. `arr[i] += v`).  In that case the VARI Push should NOT
/// consume dim1/dim2 from the translation stack — the compound write path
/// needs those values for the popaf write-back.
fn lookahead_next_af_is_pushaf(rest: &[Instruction]) -> bool {
    for inst in rest {
        match inst.opcode {
            Opcode::B
            | Opcode::Bt
            | Opcode::Bf
            | Opcode::Ret
            | Opcode::Exit
            | Opcode::PushEnv
            | Opcode::PopEnv
            | Opcode::Dup => return false,
            Opcode::Break => {
                if let Operand::Break { signal, .. } = inst.operand {
                    match signal {
                        0xFFFE => return true,  // pushaf = READ
                        0xFFFD => return false, // popaf = WRITE
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Returns `true` when a cross-object variable access (`ref_type != 0`,
/// `ref_type != 0x80`, `instance >= 0`) is used in a 2D array READ context,
/// i.e. the GML VM pushed `dim1` and `dim2` onto the stack before this VARI
/// Push and the next array-index signal is `pushaf`.
///
/// Example: `AceDoll.charSelect[selected]` (ref_type=0x10) where the result
/// is then indexed at `[8]` via `pushaf`.
///
/// In a WRITE context (popaf follows), `dim1` must be left on the stack for
/// `popaf` to consume, so this returns `false` and the VARI Push is treated
/// as a plain field load.
fn is_cross_obj_2d_read(var_ref: &VariableRef, instance: i16, rest: &[Instruction]) -> bool {
    instance >= 0
        && var_ref.ref_type != 0
        && var_ref.ref_type != 0x80
        && lookahead_next_af_is_pushaf(rest)
}

/// Check if a variable operand uses stacktop-via-ref_type encoding.
///
/// In older GameMaker bytecode (pre-GMS2), `ref_type == 0x80` with a
/// non-negative instance type indicates that the target instance is on the
/// operand stack, similar to the GMS2 Stacktop instance type (-9). The
/// `instance` field provides a type hint (which object type to expect) but
/// the actual instance ID is popped from the stack at runtime.
fn is_stacktop_ref(var_ref: &VariableRef, instance: i16) -> bool {
    instance >= 0 && var_ref.ref_type == 0x80
}

/// Check if the next instruction is a stacktop variable access via
/// `ref_type=0x80` only (not `InstanceType::Stacktop`).
///
/// Used by the cross-object sentinel skip in `translate_push_instruction`
/// where `InstanceType::Stacktop` needs a separate, more precise check
/// (@@Global@@ only) to avoid false positives on self-access.
pub(super) fn is_next_stacktop_ref_access(rest: &[Instruction]) -> bool {
    if let Some(next) = rest.first() {
        if matches!(next.opcode, Opcode::Push | Opcode::Pop) {
            if let Operand::Variable { var_ref, instance } = &next.operand {
                return is_stacktop_ref(var_ref, *instance);
            }
        }
    }
    false
}

/// Resolve a branch target offset to (target_offset, BlockId).
fn resolve_branch_target(
    inst: &Instruction,
    offset: i32,
    block_map: &HashMap<usize, BlockId>,
) -> Result<(usize, BlockId), String> {
    let target = (inst.offset as i64 + offset as i64) as usize;
    let block = block_map.get(&target).copied().ok_or_else(|| {
        format!(
            "unresolved branch target at offset {:#x} → {:#x}",
            inst.offset, target
        )
    })?;
    Ok((target, block))
}

/// Pop a value from the operand stack.
fn pop(stack: &mut Vec<ValueId>, inst: &Instruction) -> Result<ValueId, String> {
    stack
        .pop()
        .ok_or_else(|| format!("{:#x}: stack underflow on {:?}", inst.offset, inst.opcode))
}

/// Resolve the fall-through target to (target_offset, BlockId).
fn resolve_fallthrough(
    instructions: &[Instruction],
    inst_idx: usize,
    block_map: &HashMap<usize, BlockId>,
) -> Result<(usize, BlockId), String> {
    let next = instructions
        .get(inst_idx + 1)
        .ok_or_else(|| format!("no fall-through instruction after index {}", inst_idx))?;
    let block = block_map.get(&next.offset).copied().ok_or_else(|| {
        format!(
            "fall-through offset {:#x} is not a block start",
            next.offset
        )
    })?;
    Ok((next.offset, block))
}

// ---------------------------------------------------------------------------
// Core translation loop
// ---------------------------------------------------------------------------

/// Core translation loop shared by [`translate_code_entry`] and [`translate_with_body`].
///
/// Handles the `skip_until` mechanism that skips over with-body instruction ranges
/// (they are extracted into separate closure functions instead of being translated
/// inline).  Returns `true` if the last block was terminated.
#[allow(clippy::too_many_arguments)]
fn run_translation_loop(
    instructions: &[Instruction],
    func_name: &str,
    fb: &mut FunctionBuilder,
    block_map: &HashMap<usize, BlockId>,
    block_params: &HashMap<usize, Vec<ValueId>>,
    block_entry_depths: &HashMap<usize, usize>,
    with_ranges: &HashMap<usize, usize>,
    locals: &mut HashMap<String, ValueId>,
    ctx: &TranslateCtx<'_>,
    extra_funcs: &mut Vec<Function>,
    global_arg_count: u16,
) -> Result<bool, String> {
    let mut stack: Vec<ValueId> = Vec::new();
    // Track GML type sizes (in 4-byte units) for each value on the stack.
    // Used by Dup to compute correct item count when items have different sizes
    // (e.g., Variable = 4 units vs Int16 = 1 unit).
    let mut gml_sizes: HashMap<ValueId, u8> = HashMap::new();
    let mut terminated = false;
    // Set to true when a 2D array VARI read leaves the original dim indices on
    // the stack (compound assignment Dup pattern). The subsequent Pop must use
    // reversed pop order: value on top, dim1 below, _dim2 at bottom.
    let mut compound_2d_pending = false;
    // Array INDEX captured by pushac (0xFFFC) for use by popaf (0xFFFD).
    // pushac saves the index so popaf can pop ARRAY and VALUE from the
    // stack independently, then write ARRAY[INDEX] = VALUE.
    let mut pushac_array: Option<ValueId> = None;
    // Set to true by Dup(swap) (DupExtra != 0 with dup_size > 0) which appears
    // immediately before popaf in compound array element writes (e.g. `arr[i] += x`).
    // In the compound pattern the stack order at popaf time is [..., ARRAY, INDEX, VALUE]
    // (ARRAY below INDEX), which is the REVERSE of the simple-write order
    // [..., INDEX, ARRAY, VALUE].  This flag tells popaf to pop INDEX before ARRAY.
    let mut compound_popaf_pending = false;
    // When Some(n), skip instructions with index < n (with-body instructions
    // that have been extracted into a closure).
    let mut skip_until: Option<usize> = None;
    // Track pushref OBJT values: ValueId → class name. Populated by translate_instruction
    // for signal 0xFFF5 with type_tag==0, consumed at PushEnv to type the with-body _self.
    let mut obj_ref_values: HashMap<ValueId, String> = HashMap::new();
    // Set when Call @@Global@@ pushes the global scope. The PushI -9 skip uses this
    // to also check InstanceType::Stacktop (not just ref_type=0x80).
    let mut global_scope_on_stack = false;

    for (inst_idx, inst) in instructions.iter().enumerate() {
        // Skip instructions that belong to a with-body extracted as a closure.
        if let Some(skip) = skip_until {
            if inst_idx < skip {
                continue;
            }
            skip_until = None;
        }

        // Check if this instruction starts a new block.
        if inst_idx > 0 {
            if let Some(&block) = block_map.get(&inst.offset) {
                // Emit fall-through branch if previous block wasn't terminated.
                if !terminated {
                    let depth = block_entry_depths.get(&inst.offset).copied().unwrap_or(0);
                    let args = get_branch_args(&stack, depth);
                    fb.br(block, &args);
                }
                fb.switch_to_block(block);
                stack.clear();
                compound_2d_pending = false;
                pushac_array = None;
                compound_popaf_pending = false;
                if let Some(params) = block_params.get(&inst.offset) {
                    for &p in params {
                        // Block params are Variable-sized (16 bytes = 4 units).
                        gml_sizes.insert(p, 4);
                    }
                    stack.extend(params.iter().copied());
                }
                terminated = false;
            }
        }

        if terminated {
            continue;
        }

        // Special handling for PushEnv: extract the with-body as a closure.
        if inst.opcode == Opcode::PushEnv {
            if let Some(&popenv_idx) = with_ranges.get(&inst_idx) {
                let target_obj = pop(&mut stack, inst)?;
                let body_insts = &instructions[inst_idx + 1..popenv_idx];

                // Determine which outer locals the body needs to capture.
                let scanned_names = scan_body_local_names(body_insts, ctx, locals);
                // If the outer context has a self and the body accesses `other`, capture
                // the outer self as _other (prepended so it becomes the first capture param).
                let has_outer_self = ctx.has_self && scan_body_uses_other(body_insts, ctx);
                let mut captured_names: Vec<String> = Vec::new();
                let mut capture_vals: Vec<ValueId> = Vec::new();
                if has_outer_self {
                    captured_names.push("_other".to_string());
                    // Outer self is always param 0 (both for regular event handlers and
                    // nested with-bodies where param 0 is the current iterated _self).
                    capture_vals.push(fb.param(0));
                }
                // Capture any argument[N] variables the with-body reads from the outer
                // function.  The inner closure has no argument params of its own, so
                // each outer argument[N] must be passed in as a named capture.
                let outer_arg_offset =
                    if ctx.has_self { 1 } else { 0 } + if ctx.has_other { 1 } else { 0 };
                for n in scan_body_argument_indices(body_insts, ctx) {
                    let captured_key = format!("_argument{n}");
                    // In a nested with-body, the outer argument is already in
                    // locals as `_argumentN` (an alloc slot).  Load from there.
                    if let Some(&slot) = locals.get(&captured_key) {
                        captured_names.push(captured_key);
                        let ty = fb.fresh_var();
                        capture_vals.push(fb.load(slot, ty));
                    } else {
                        // Top-level: argument is a formal param of the outer function.
                        let outer_idx = outer_arg_offset + n;
                        if outer_idx < fb.param_count() {
                            captured_names.push(captured_key);
                            capture_vals.push(fb.param(outer_idx));
                        }
                    }
                }
                for name in &scanned_names {
                    captured_names.push(name.clone());
                    let &slot = locals
                        .get(name)
                        .expect("captured local must have an alloc slot");
                    let ty = fb.fresh_var();
                    capture_vals.push(fb.load(slot, ty));
                }

                // Determine the class of the with-target (if it's a typed OBJT pushref).
                let instance_class: Option<&str> =
                    obj_ref_values.get(&target_obj).map(String::as_str);

                // Detect "return X inside with" pattern: an exit PopEnv with sentinel
                // branch offset exists in body_insts. In this case the closure should
                // return Unknown and the outer function should return withInstances(…).
                let has_return_in_with = has_exit_popenv(body_insts);

                // Build the inner closure function (may recursively extract nested withs).
                let inner_name = format!("{func_name}_with_{:04x}", inst.offset);
                let inner_func = translate_with_body(
                    &WithBodyCtx {
                        body_insts,
                        inner_name: &inner_name,
                        ctx,
                        captured_names: &captured_names,
                        has_outer_self,
                        instance_class,
                        has_return_in_with,
                    },
                    extra_funcs,
                )?;
                extra_funcs.push(inner_func);

                // Emit: withInstances(target, closure).
                // When the closure uses "return X inside with", type the call as Unknown
                // so the outer function can propagate the return value.
                let closure_ty = fb.fresh_var();
                let closure_val = fb.make_closure(&inner_name, &capture_vals, closure_ty);
                let with_return_ty = if has_return_in_with {
                    fb.fresh_var()
                } else {
                    Type::Void
                };
                let with_result = fb.system_call(
                    "GameMaker.Instance",
                    "withInstances",
                    &[target_obj, closure_val],
                    with_return_ty,
                );

                if has_return_in_with {
                    // The closure returns the with-body's return value; propagate it.
                    fb.ret(Some(with_result));
                } else {
                    // Branch unconditionally to the post-with block.
                    let post_with_idx = popenv_idx + 1;
                    if post_with_idx < instructions.len() {
                        let post_with_off = instructions[post_with_idx].offset;
                        let fall_block =
                            block_map.get(&post_with_off).copied().ok_or_else(|| {
                                format!(
                                    "{func_name}: no block at post-with offset {post_with_off:#x}"
                                )
                            })?;
                        let depth = block_entry_depths.get(&post_with_off).copied().unwrap_or(0);
                        let args = get_branch_args(&stack, depth);
                        fb.br(fall_block, &args);
                    } else {
                        fb.ret(None);
                    }
                }
                terminated = true;
                // Skip the body instructions and the PopEnv; resume after PopEnv.
                skip_until = Some(popenv_idx + 1);
                continue;
            }
        }

        translate_instruction(
            inst,
            instructions,
            inst_idx,
            fb,
            &mut stack,
            block_map,
            locals,
            ctx,
            &mut terminated,
            block_entry_depths,
            &mut gml_sizes,
            &mut compound_2d_pending,
            &mut pushac_array,
            &mut compound_popaf_pending,
            global_arg_count,
            &mut obj_ref_values,
            &mut global_scope_on_stack,
        )?;
    }

    Ok(terminated)
}
