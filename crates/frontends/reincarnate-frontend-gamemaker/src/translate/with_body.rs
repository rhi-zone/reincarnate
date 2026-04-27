use std::collections::{HashMap, HashSet};

use datawin::bytecode::decode::{Instruction, Operand};
use datawin::bytecode::opcode::Opcode;
use datawin::bytecode::types::InstanceType;
use reincarnate_core::entity::EntityRef as _;
use reincarnate_core::ir::builder::FunctionBuilder;
use reincarnate_core::ir::func::{CaptureMode, Function, MethodKind, Visibility};
use reincarnate_core::ir::ty::{FunctionSig, Type, TypeVarId};
use reincarnate_core::ir::value::ValueId;

use super::cfg::setup_blocks;
use super::switch::detect_switches;
use super::{
    allocate_locals, parse_argument_index, resolve_variable_name, run_translation_loop,
    TranslateCtx,
};

/// Map each PushEnv instruction index to its corresponding PopEnv index.
///
/// Uses the PushEnv's branch operand to locate the matching PopEnv, handling
/// both GMS1 and GMS2.3+ bytecode conventions:
///
/// - **GMS1**: `PushEnv Branch(N)` where `offset + N = PopEnv.offset`.
/// - **GMS2.3+**: `PushEnv Branch(N)` where `offset + N = continuation.offset`
///   (instruction AFTER PopEnv). Since PopEnv is a 4-byte instruction,
///   PopEnv.offset = continuation - 4. We also try continuation - 8 as a
///   fallback for cases where the continuation is measured differently.
///
/// Stack-based nesting cannot be used because GML emits "early-exit" PopEnv
/// instructions (e.g. `return` inside a `with` body) that would be incorrectly
/// paired with an inner PushEnv, causing the wrong body slice to be extracted.
///
/// PushEnvs whose PopEnv lies in a sibling code entry (GMS2.3+ cross-code-entry
/// with-blocks) are left unmatched. The translate_instruction fallback handles
/// them by executing the body once for `self` only.
pub(super) fn find_with_ranges(instructions: &[Instruction]) -> HashMap<usize, usize> {
    // Build offset → index map for this slice.
    let offset_to_idx: HashMap<usize, usize> = instructions
        .iter()
        .enumerate()
        .map(|(i, inst)| (inst.offset, i))
        .collect();

    let mut result = HashMap::new();

    for (i, inst) in instructions.iter().enumerate() {
        if inst.opcode != Opcode::PushEnv {
            continue;
        }
        let branch_offset = match inst.operand {
            Operand::Branch(off) => off,
            _ => continue,
        };
        // PushEnv Branch(N): the target is either:
        //   GMS1 style — target == PopEnv.offset  (branch jumps to the PopEnv)
        //   GMS2.3+ style — target == PopEnv.offset + sizeof(PopEnv)  (jumps to continuation)
        //
        // PopEnv is a 4-byte instruction, so sizeof(PopEnv) = 4.
        // So in GMS2.3+: PopEnv.offset = branch_target - 4.
        let branch_target = (inst.offset as i64 + branch_offset as i64) as usize;

        // Try GMS1: branch target IS the PopEnv.
        if let Some(&popenv_idx) = offset_to_idx.get(&branch_target) {
            if instructions[popenv_idx].opcode == Opcode::PopEnv {
                result.insert(i, popenv_idx);
                continue;
            }
        }

        // Try GMS2.3+: branch target is the continuation (PopEnv = target - 4).
        if branch_target >= 4 {
            let popenv_off = branch_target - 4;
            if let Some(&popenv_idx) = offset_to_idx.get(&popenv_off) {
                if instructions[popenv_idx].opcode == Opcode::PopEnv {
                    result.insert(i, popenv_idx);
                    continue;
                }
            }
        }

        // Also try target - 8 (some GMS2 versions may encode continuation differently).
        if branch_target >= 8 {
            let popenv_off = branch_target - 8;
            if let Some(&popenv_idx) = offset_to_idx.get(&popenv_off) {
                if instructions[popenv_idx].opcode == Opcode::PopEnv {
                    result.insert(i, popenv_idx);
                    continue;
                }
            }
        }

        // Neither heuristic found the PopEnv: the matching PopEnv is in a sibling
        // code entry (GMS2.3+ cross-code-entry with-block). The unmatched PushEnv
        // falls through to translate_instruction which handles it by discarding the
        // target and falling through to the body — semantically incomplete but valid.
    }

    result
}

/// Returns the set of PushEnv instruction indices (within `with_ranges`) where
/// the with-target is the GML self-sentinel (integer -9). These ranges are
/// inlined directly into the outer function's CFG instead of extracted as closures.
pub(super) fn find_self_with_indices(
    instructions: &[Instruction],
    with_ranges: &HashMap<usize, usize>,
) -> HashSet<usize> {
    let mut result = HashSet::new();
    for &pushenv_idx in with_ranges.keys() {
        if pushenv_idx == 0 {
            continue;
        }
        let prev = &instructions[pushenv_idx - 1];
        let is_neg9 = matches!(
            prev.operand,
            Operand::Int16(-9) | Operand::Int32(-9) | Operand::Int64(-9)
        );
        if is_neg9
            && matches!(
                prev.opcode,
                Opcode::PushI | Opcode::Push | Opcode::PushLoc | Opcode::PushGlb | Opcode::PushBltn
            )
        {
            result.insert(pushenv_idx);
        }
    }
    result
}

/// Returns the set of PushEnv instruction indices (within `with_ranges`) where
/// the with-target is the GML "all instances" sentinel (integer -3). These
/// ranges are inlined as while-loops instead of extracted as closures.
pub(super) fn find_all_with_indices(
    instructions: &[Instruction],
    with_ranges: &HashMap<usize, usize>,
) -> HashSet<usize> {
    let mut result = HashSet::new();
    for &pushenv_idx in with_ranges.keys() {
        if pushenv_idx == 0 {
            continue;
        }
        let prev = &instructions[pushenv_idx - 1];
        let is_neg3 = matches!(
            prev.operand,
            Operand::Int16(-3) | Operand::Int32(-3) | Operand::Int64(-3)
        );
        if is_neg3
            && matches!(
                prev.opcode,
                Opcode::PushI | Opcode::Push | Opcode::PushLoc | Opcode::PushGlb | Opcode::PushBltn
            )
        {
            result.insert(pushenv_idx);
        }
    }
    result
}

/// Returns the set of PushEnv instruction indices (within `with_ranges`) where
/// the with-target is a GMS2.3+ OBJT pushref (class reference). These ranges are
/// inlined as filtered instance loops instead of extracted as closures.
pub(super) fn find_classref_with_indices(
    instructions: &[Instruction],
    with_ranges: &HashMap<usize, usize>,
) -> HashSet<usize> {
    let mut result = HashSet::new();
    for &pushenv_idx in with_ranges.keys() {
        if pushenv_idx == 0 {
            continue;
        }
        let prev = &instructions[pushenv_idx - 1];
        // PushRef OBJT (Break -11 with type_tag==0 in extra) immediately before PushEnv
        if let Operand::Break {
            signal,
            extra: Some(idx),
        } = prev.operand
        {
            if signal == 0xFFF5 && (idx as u32) >> 24 == 0 {
                result.insert(pushenv_idx);
            }
        }
    }
    result
}

/// Find the names of outer local variables accessed in a slice of instructions.
///
/// Used to determine which locals a with-body closure needs to capture.
/// `outer_locals` is the caller's live locals map at the time the `PushEnv`
/// instruction is processed — it contains both CodeLocals allocs and any
/// on-the-fly allocs created during translation up to that point.
pub(super) fn scan_body_local_names(
    body_insts: &[Instruction],
    ctx: &TranslateCtx<'_>,
    outer_locals: &HashMap<String, ValueId>,
) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut names = Vec::new();
    for inst in body_insts {
        if let Operand::Variable { instance, .. } = &inst.operand {
            if matches!(InstanceType::from_i16(*instance), Some(InstanceType::Local)) {
                let name = resolve_variable_name(inst, ctx);
                // Only capture variables that actually have an alloc slot in the outer
                // function. This covers both CodeLocals-declared variables and on-the-fly
                // allocs created during translation (e.g. variables declared with `var` but
                // absent from CodeLocals in obfuscated games like Dead Estate).
                // resolve_variable_name returns "var_unknown_{offset}" on VARI lookup
                // failure; those have no slot and are implicitly excluded here.
                if outer_locals.contains_key(&name) && seen.insert(name.clone()) {
                    names.push(name);
                }
            }
        }
    }
    names
}

/// Return true if any VARI instruction in `body_insts` uses `InstanceType::Other`.
/// Used to decide whether to capture the outer self for `other.field` access.
pub(super) fn scan_body_uses_other(body_insts: &[Instruction], _ctx: &TranslateCtx<'_>) -> bool {
    body_insts.iter().any(|inst| {
        matches!(&inst.operand,
            Operand::Variable { instance, .. }
                if matches!(InstanceType::from_i16(*instance), Some(InstanceType::Other))
        )
    })
}

/// Find all `argument[N]` indices accessed in a with-body.
///
/// A with-body is compiled as a nested function; the outer function's arguments
/// are not visible as params inside it.  The caller uses this list to capture
/// each needed argument as an extra closure parameter (`_argument{N}`).
pub(super) fn scan_body_argument_indices(
    body_insts: &[Instruction],
    ctx: &TranslateCtx<'_>,
) -> Vec<usize> {
    let mut seen: HashSet<usize> = HashSet::new();
    let mut indices: Vec<usize> = Vec::new();
    for (i, inst) in body_insts.iter().enumerate() {
        if let Operand::Variable { var_ref, instance } = &inst.operand {
            let instance_ty = InstanceType::from_i16(*instance);
            let found: Option<usize> = if matches!(instance_ty, Some(InstanceType::Arg)) {
                // variable_id is the VARI table index, not the argument index —
                // extract the actual index from the variable name ("argument3" → 3).
                parse_argument_index(&resolve_variable_name(inst, ctx))
            } else if matches!(
                instance_ty,
                Some(InstanceType::Own)
                    | Some(InstanceType::Builtin)
                    | Some(InstanceType::Static)
                    | Some(InstanceType::Global)
            ) {
                // Named form: argument0, argument1, ...
                // Static (-15) and Global (-5) are used in GMS2.3+ for argument
                // references within struct/constructor functions.
                parse_argument_index(&resolve_variable_name(inst, ctx))
            } else if var_ref.ref_type == 0 && *instance >= 0 {
                // 2D-array form: `argument[N]`.  dim1 (the argument index) is
                // the value on top of the stack just before this instruction,
                // i.e. pushed by the immediately preceding Push/PushI.
                let var_name = resolve_variable_name(inst, ctx);
                if var_name == "argument" {
                    i.checked_sub(1)
                        .and_then(|j| body_insts.get(j))
                        .and_then(|prev| match prev.operand {
                            Operand::Int16(v) if v >= 0 => Some(v as usize),
                            Operand::Int32(v) if v >= 0 => Some(v as usize),
                            Operand::Int64(v) if v >= 0 => Some(v as usize),
                            _ => None,
                        })
                } else {
                    None
                }
            } else {
                None
            };
            if let Some(idx) = found {
                if seen.insert(idx) {
                    indices.push(idx);
                }
            }
        }
    }
    indices.sort_unstable();
    indices
}

/// Context for translating a `with`-body closure.
///
/// Bundles the parameters needed by [`translate_with_body`] so the function
/// stays under Clippy's 7-argument limit.
pub(super) struct WithBodyCtx<'a> {
    pub body_insts: &'a [Instruction],
    pub inner_name: &'a str,
    pub ctx: &'a TranslateCtx<'a>,
    pub captured_names: &'a [String],
    pub has_outer_self: bool,
    /// Class name of the `with`-target if it was a typed OBJT pushref.
    /// When set, the `_self` parameter and `inner_ctx.class_name` are typed accordingly.
    pub instance_class: Option<&'a str>,
    /// True when `body_insts` contains an exit PopEnv (sentinel Branch offset ≈ -4194304).
    /// This is the GML `return X inside with` pattern: the closure should return Unknown
    /// and the outer function should `return withInstances(...)`.
    pub has_return_in_with: bool,
}

/// Detect GML "return X inside with" pattern.
///
/// True when `body_insts` contains an exit PopEnv (sentinel branch offset ≈ -4194304)
/// AND has no conditional branches (Bt/Bf).  The latter condition ensures the exit
/// PopEnv is the *sole* exit from the body — every code path ends there — so the
/// closure's return type can safely be narrowed from void to Unknown.
///
/// When conditional branches exist (loops, if/else), other code paths may fall
/// through without returning a value.  Changing the closure's return type in that
/// case causes TypeScript TS2366 ("function lacks ending return statement").
pub(super) fn has_exit_popenv(body_insts: &[Instruction]) -> bool {
    let has_sentinel = body_insts.iter().any(|inst| {
        inst.opcode == Opcode::PopEnv
            && matches!(inst.operand, Operand::Branch(off) if off < -1_000_000)
    });
    let has_branches = body_insts
        .iter()
        .any(|inst| matches!(inst.opcode, Opcode::Bt | Opcode::Bf));
    has_sentinel && !has_branches
}

pub(super) fn translate_with_body(
    wctx: &WithBodyCtx<'_>,
    extra_funcs: &mut Vec<Function>,
) -> Result<Function, String> {
    // When the with-target is a known OBJT or self-sentinel, type _self as that class.
    // For unknown targets (variables, `all`, etc.) use a fresh type variable so
    // inference can resolve the type from field accesses rather than locking it to
    // GMLObject (which forecloses access to subclass-specific fields).
    // Pre-builder type variable counter: used for FunctionSig construction before
    // FunctionBuilder is available. The constraint solver ignores TypeVarId values —
    // only is_concrete() matters — so counter collisions across functions are safe.
    let mut pre_tv: u32 = 0;
    let mut pre_fresh = || {
        let ty = Type::Var(TypeVarId::new(pre_tv));
        pre_tv += 1;
        ty
    };

    let instance_types = wctx.ctx.instance_types;
    let self_ty = wctx
        .instance_class
        .and_then(|n| instance_types.get(n).copied())
        .map(Type::Instance)
        .unwrap_or_else(&mut pre_fresh);
    // Use a fresh type variable for the return type when the body contains
    // "return X inside with" (exit PopEnv with sentinel Branch offset).
    let closure_return_ty = if wctx.has_return_in_with {
        pre_fresh()
    } else {
        Type::Void
    };
    let sig = FunctionSig {
        params: vec![self_ty], // _self
        return_ty: closure_return_ty,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new(wctx.inner_name, sig, Visibility::Public);
    fb.set_registry(wctx.ctx.registry.clone());
    fb.name_value(fb.param(0), "_self".to_string());

    // Declare capture parameters (ByValue snapshots of outer locals).
    // ByRef would be more faithful to GML semantics (with-bodies are
    // inline code) but requires outer-scope `let` declarations that
    // Mem2Reg may have eliminated.  ByValue is safe for all cases.
    //
    // `_other` is a special capture: it holds the outer `self` (the enclosing
    // event handler's instance), so it should have the outer self's type rather
    // than Unknown.  The outer self type is the enclosing class (if known) or
    // GMLObject (the universal base class).
    let outer_self_ty = if wctx.has_outer_self {
        Type::Instance(
            wctx.ctx
                .class_name
                .and_then(|n| instance_types.get(n).copied())
                .unwrap_or(wctx.ctx.gml_object_type_id),
        )
    } else {
        fb.fresh_var()
    };
    // Pre-allocate capture types before calling add_capture_params to avoid
    // double-borrowing fb inside the map closure.
    let capture_types: Vec<Type> = wctx
        .captured_names
        .iter()
        .enumerate()
        .map(|(i, _)| {
            if wctx.has_outer_self && i == 0 {
                outer_self_ty.clone()
            } else {
                fb.fresh_var()
            }
        })
        .collect();
    let capture_ids = if wctx.captured_names.is_empty() {
        vec![]
    } else {
        fb.add_capture_params(
            wctx.captured_names
                .iter()
                .zip(capture_types)
                .map(|(n, ty)| (n.clone(), ty, CaptureMode::ByValue))
                .collect(),
        )
    };

    let inner_with_ranges = find_with_ranges(wctx.body_insts);
    let inner_self_with_indices: HashSet<usize> = HashSet::new();
    let inner_all_with_indices: HashSet<usize> = HashSet::new();
    let inner_classref_with_indices: HashSet<usize> = HashSet::new();
    let entry_offset = wctx.body_insts.first().map_or(0, |inst| inst.offset);
    let (block_map, block_params, block_entry_depths) = setup_blocks(
        &mut fb,
        wctx.body_insts,
        &inner_with_ranges,
        &inner_self_with_indices,
        entry_offset,
        wctx.ctx.function_names,
        wctx.ctx.bytecode_offset,
        wctx.ctx.func_ref_map,
        &inner_all_with_indices,
        &inner_classref_with_indices,
    );

    let ctx = wctx.ctx;

    // Allocate outer local variable slots (reusing the outer ctx's local names).
    let mut locals = allocate_locals(&mut fb, ctx);

    // Allocate alloc slots for captured argument variables (_argument0, _argument1, …).
    // These are not GML locals so they aren't in ctx.local_names / allocate_locals.
    for name in wctx.captured_names {
        if name.starts_with("_argument") && !locals.contains_key(name) {
            let ty = fb.fresh_var();
            let slot = fb.alloc(ty);
            locals.insert(name.clone(), slot);
        }
    }

    // Pre-store captured values into their alloc slots so the body can read them.
    // When a local variable has no pre-allocated slot (e.g. no CodeLocals in
    // obfuscated GMS2.3+ games), create the alloc here. Without this, the body's
    // on-the-fly alloc would be disconnected from the capture parameter, causing
    // the captured value to be lost (reads would return the default 0.0/null).
    for (i, name) in wctx.captured_names.iter().enumerate() {
        let slot = if let Some(&s) = locals.get(name) {
            s
        } else {
            let ty = fb.fresh_var();
            let s = fb.alloc(ty);
            fb.name_value(s, name.clone());
            locals.insert(name.clone(), s);
            s
        };
        fb.store(slot, capture_ids[i]);
    }

    // Inner context: same VARI/FUNC tables but no declared args, class-typed self.
    let inner_ctx = TranslateCtx {
        has_self: true,
        has_other: wctx.has_outer_self,
        arg_count: 0,
        // Use the class name from the with-target so field-type lookups work.
        class_name: wctx.instance_class,
        function_names: ctx.function_names,
        asset_ref_names: ctx.asset_ref_names,
        variables: ctx.variables,
        func_ref_map: ctx.func_ref_map,
        vari_ref_map: ctx.vari_ref_map,
        bytecode_offset: ctx.bytecode_offset,
        local_names: ctx.local_names,
        string_table: ctx.string_table,
        obj_names: ctx.obj_names,
        self_object_index: ctx.self_object_index,
        ancestor_indices: ctx.ancestor_indices.clone(),
        script_names: ctx.script_names,
        // With-body closures are not event handlers — they have their own return
        // type inferred from the body.
        is_event_handler: false,
        // This IS a with-body closure — PopEnv inside is an early-exit signal,
        // not a loop-control instruction (the loop is managed by withInstances).
        is_with_body: true,
        with_body_has_return: wctx.has_return_in_with,
        bytecode_version: ctx.bytecode_version,
        classref_types: ctx.classref_types,
        instance_types: ctx.instance_types,
        gml_object_type_id: ctx.gml_object_type_id,
        registry: ctx.registry,
    };

    fb.switch_to_block(fb.entry_block());
    let terminated = run_translation_loop(
        wctx.body_insts,
        wctx.inner_name,
        &mut fb,
        &block_map,
        &block_params,
        &block_entry_depths,
        &inner_with_ranges,
        &inner_self_with_indices,
        &inner_all_with_indices,
        &inner_classref_with_indices,
        &mut locals,
        &inner_ctx,
        extra_funcs,
        0, // with-bodies don't have their own argument params
    )?;

    if !terminated {
        fb.ret(None);
    }

    let mut func = fb.build();
    func.method_kind = MethodKind::Closure;
    detect_switches(&mut func);
    Ok(func)
}
