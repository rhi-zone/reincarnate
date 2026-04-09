use std::collections::HashMap;

use datawin::bytecode::decode::{Instruction, Operand};
use datawin::bytecode::types::InstanceType;
use reincarnate_core::ir::builder::FunctionBuilder;
use reincarnate_core::ir::ty::Type;
use reincarnate_core::ir::value::{Constant, ValueId};

use super::{
    is_2d_array_access, is_cross_obj_2d_read, is_stacktop_ref, parse_argument_index, pop,
    resolve_variable_name, TranslateCtx,
};

/// Coerce a compile-time constant to an `i64` integer value.
///
/// GML integers stored as `Operand::Int32` were historically emitted as
/// `Constant::Int`, but since we now emit them as `Constant::Float` (because
/// GML has only one numeric type — `Real` — at the source level), instance
/// sentinels such as `-9` (self) and `-1` (scalar) arrive as
/// `Constant::Float(-9.0)` / `Constant::Float(-1.0)`.  This helper accepts
/// both forms so that downstream pattern-matching is forward-compatible.
fn const_as_i64(c: &Constant) -> Option<i64> {
    match c {
        Constant::Int(n) => Some(*n),
        Constant::Float(f) => {
            let i = *f as i64;
            if i as f64 == *f {
                Some(i)
            } else {
                None
            }
        }
        _ => None,
    }
}

/// Translate a push instruction.
#[allow(clippy::too_many_arguments)]
pub(super) fn translate_push(
    inst: &Instruction,
    rest: &[Instruction],
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    locals: &mut HashMap<String, ValueId>,
    ctx: &TranslateCtx,
    compound_2d_pending: &mut bool,
    global_arg_count: u16,
    preceded_by_dup: bool,
) -> Result<(), String> {
    match &inst.operand {
        Operand::Int16(v) => stack.push(fb.const_float(*v as f64)),
        Operand::Int32(v) => stack.push(fb.const_float(*v as f64)),
        Operand::Int64(v) => stack.push(fb.const_int(*v, 64)),
        Operand::Double(v) => stack.push(fb.const_float(*v)),
        Operand::Float(v) => stack.push(fb.const_float(*v as f64)),
        Operand::Bool(v) => stack.push(fb.const_bool(*v)), // Push.Bool: valid per spec, not emitted by real GML compilers
        Operand::StringIndex(idx) => {
            let s = ctx.string_table.get(*idx as usize).ok_or_else(|| {
                format!(
                    "string index {} out of range (table size={})",
                    idx,
                    ctx.string_table.len()
                )
            })?;
            stack.push(fb.const_string(s));
        }
        Operand::Variable { var_ref, instance } => {
            translate_push_variable(
                inst,
                rest,
                fb,
                stack,
                locals,
                ctx,
                var_ref,
                *instance,
                compound_2d_pending,
                global_arg_count,
                preceded_by_dup,
            )?;
        }
        _ => {
            return Err(format!(
                "{:#x}: unexpected Push operand {:?}",
                inst.offset, inst.operand
            ));
        }
    }
    Ok(())
}

/// Translate a Push with Variable operand (load from variable).
#[allow(clippy::too_many_arguments)]
pub(super) fn translate_push_variable(
    inst: &Instruction,
    rest: &[Instruction],
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    locals: &mut HashMap<String, ValueId>,
    ctx: &TranslateCtx,
    var_ref: &datawin::bytecode::types::VariableRef,
    instance: i16,
    compound_2d_pending: &mut bool,
    global_arg_count: u16,
    preceded_by_dup: bool,
) -> Result<(), String> {
    let var_name = resolve_variable_name(inst, ctx);

    // Handle stacktop-via-ref_type (ref_type == 0x80 with instance >= 0).
    // The target instance is on the stack (pushed before this instruction).
    // In GMS2.3+, Push Int32(-9) followed by a stacktop variable access is
    // the "self" pattern — resolve to the self parameter directly.
    if is_stacktop_ref(var_ref, instance) {
        let raw_target = pop(stack, inst)?;
        let target = if ctx.has_self {
            if fb
                .try_resolve_const(raw_target)
                .and_then(|c| const_as_i64(&c))
                == Some(-9)
            {
                fb.param(0)
            } else {
                raw_target
            }
        } else {
            raw_target
        };
        if ctx.has_self && target == fb.param(0) {
            let ty = fb.fresh_var();
            let val = fb.get_field(target, &var_name, ty);
            stack.push(val);
        } else if let Some(obj_idx) = fb.try_resolve_const(target).and_then(|c| const_as_i64(&c)) {
            // Constant integer target = object index pushed before stacktop access.
            // Resolve to getOn(objName, field) for clean class-based access.
            if obj_idx >= 0 {
                let obj_id = if let Some(name) = ctx.obj_names.get(obj_idx as usize) {
                    fb.const_string(name)
                } else {
                    fb.const_int(obj_idx, 64)
                };
                let name_val = fb.const_string(&var_name);
                let ty = fb.fresh_var();
                let val = fb.call_named("GameMaker.Instance.getOn", &[obj_id, name_val], ty);
                stack.push(val);
            } else {
                let name_val = fb.const_string(&var_name);
                let ty = fb.fresh_var();
                let val = fb.call_named("GameMaker.Instance.getField", &[target, name_val], ty);
                stack.push(val);
            }
        } else {
            let name_val = fb.const_string(&var_name);
            let ty = fb.fresh_var();
            let val = fb.call_named("GameMaker.Instance.getField", &[target, name_val], ty);
            stack.push(val);
        }
        return Ok(());
    }

    // Handle cross-object 2D array READ (ref_type != 0, ref_type != 0x80,
    // instance >= 0, and the next array-index signal is `pushaf`).
    // E.g. `AceDoll.charSelect[selected]` where the result is then indexed
    // at `[8]` via `pushaf`.  The GML VM pushes dim2 and dim1 before the
    // VARI Push; we consume them here and include dim1 in the getOn call
    // so the result is `charSelect[selected]` (a 1-D slice) ready for the
    // outer pushaf to pick element [8].
    // In a WRITE context (popaf follows) dim1 must remain on the stack for
    // popaf to use as the array index, so we fall through to the normal
    // cross-object dispatch below.
    if is_cross_obj_2d_read(var_ref, instance, rest) {
        let dim1 = pop(stack, inst)?; // first-dimension index (top of stack)
        let _dim2 = pop(stack, inst)?; // orphan dim2 (not used for 1-D slice)
        let is_scalar = fb.try_get_const(dim1).and_then(const_as_i64) == Some(-1);
        let obj_id = if let Some(name) = ctx.obj_names.get(instance as usize) {
            fb.const_string(name)
        } else {
            fb.const_int(instance as i64, 64)
        };
        let name_val = fb.const_string(&var_name);
        let args: Vec<ValueId> = if is_scalar {
            vec![obj_id, name_val]
        } else {
            vec![obj_id, name_val, dim1]
        };
        let ty = fb.fresh_var();
        let val = fb.call_named("GameMaker.Instance.getOn", &args, ty);
        stack.push(val);
        return Ok(());
    }

    // Handle 2D array access (ref_type == 0 with non-negative instance).
    // The instruction pops 2 indices from the stack (dim1, dim2).
    if is_2d_array_access(var_ref, instance) {
        // Stack layout: [dim2, dim1] with dim1 on top.
        // dim1 is the first-dimension array index.
        // dim2 encodes the scope (GMS1 two-argument array encoding):
        //   dim2 == -1  → 1D access (no second dimension, scalar element)
        //   dim2 >= 0   → cross-object access: dim2 is the OBJT index of the
        //                 target object (e.g., obj_stats.advantages[0] pushes
        //                 dim2=3 for Stats's OBJT index, then dim1=0)
        //   dim2 >= 0 and is_scalar(dim1) → cross-object scalar field access
        //               (analogous to obj.field with no array index)
        //
        // Note: `instance` in the Push.v instruction is always 0 for this
        // ref_type and is NOT the object type — dim2 carries the scope.
        let dim1 = pop(stack, inst)?;
        let dim2 = pop(stack, inst)?;
        // A normal Dup immediately before this VARI-read signals a compound
        // assignment pattern (e.g. `arr[i,j] += x`):
        //   push dim2, push dim1, Dup, VARI-read (← here), arithmetic, VARI-write
        // The Dup left original indices below the copies we just popped.
        // The subsequent Pop must use reversed order: new_value=top, dim1, dim2.
        // Only applies to ref_type==0 (self-context 2D arrays) — GMS2.3+ cross-object
        // accesses (ref_type=0x10, 0x90, etc.) use Dup(swap)+popaf instead and must
        // never set this flag.
        if var_ref.ref_type == 0 && preceded_by_dup {
            *compound_2d_pending = true;
        }
        if var_name == "argument" {
            // argument[N] → function parameter (or captured argument in a with-body).
            if let Some(idx) = fb.try_get_const(dim1).and_then(const_as_i64) {
                let n = idx as usize;
                if let Some(&slot) = locals.get(&format!("_argument{n}")) {
                    // Inside a with-body: argument was captured as a local slot.
                    let ty = fb.fresh_var();
                    stack.push(fb.load(slot, ty));
                } else {
                    let param_offset =
                        if ctx.has_self { 1 } else { 0 } + if ctx.has_other { 1 } else { 0 };
                    let param = fb.param(param_offset + n);
                    stack.push(param);
                }
            } else {
                // Unknown index — if the function has a rest param, index into it:
                // `argument[expr]` → `args[expr]`.  Otherwise fall back to getField.
                if let Some(&rest_id) = locals.get("_args") {
                    let ty = fb.fresh_var();
                    let val = fb.get_index(rest_id, dim1, ty);
                    stack.push(val);
                } else {
                    let name_val = fb.const_string(&var_name);
                    let ty = fb.fresh_var();
                    let val = fb.call_named("GameMaker.Instance.getField", &[dim1, name_val], ty);
                    stack.push(val);
                }
            }
        } else if var_ref.ref_type == 0 {
            // In GMS1, dim2 encodes the scope for ref_type==0 array accesses:
            //   dim2 < 0  → self-array access (dim2 == -1 for 1D arrays)
            //   dim2 >= 0 → cross-object access; dim2 is the OBJT index of the
            //               target object
            let dim2_scope = fb.try_get_const(dim2).cloned();
            let is_self_scope = dim2_scope
                .as_ref()
                .and_then(const_as_i64)
                .is_some_and(|n| n < 0);
            let is_scalar = fb.try_get_const(dim1).and_then(const_as_i64) == Some(-1);
            if is_self_scope {
                // Self-array read: check locals first (GML resolves locals
                // before instance fields).
                if let Some(&slot) = locals.get(&var_name) {
                    if is_scalar {
                        let ty = fb.fresh_var();
                        stack.push(fb.load(slot, ty));
                    } else {
                        let ty = fb.fresh_var();
                        let arr = fb.load(slot, ty);
                        let ty = fb.fresh_var();
                        let indexed = fb.get_index(arr, dim1, ty);
                        stack.push(indexed);
                    }
                } else if fb.param_count() > 0 {
                    let self_param = fb.param(0);
                    if is_scalar {
                        let ty = fb.fresh_var();
                        let field_val = fb.get_field(self_param, &var_name, ty);
                        stack.push(field_val);
                    } else {
                        let ty = fb.fresh_var();
                        let field_val = fb.get_field(self_param, &var_name, ty);
                        let ty = fb.fresh_var();
                        let indexed = fb.get_index(field_val, dim1, ty);
                        stack.push(indexed);
                    }
                } else {
                    // No self param (e.g. room creation scripts) — treat as local.
                    let alloc_ty = fb.fresh_var();
                    let slot = *locals
                        .entry(var_name.clone())
                        .or_insert_with(|| fb.alloc(alloc_ty));
                    if is_scalar {
                        let ty = fb.fresh_var();
                        stack.push(fb.load(slot, ty));
                    } else {
                        let ty = fb.fresh_var();
                        let arr = fb.load(slot, ty);
                        let ty = fb.fresh_var();
                        let indexed = fb.get_index(arr, dim1, ty);
                        stack.push(indexed);
                    }
                }
            } else if let Some(obj_idx) = dim2_scope.as_ref().and_then(const_as_i64) {
                // Cross-object: dim2 is the OBJT index of the target object.
                let obj_id = if let Some(name) = ctx.obj_names.get(obj_idx as usize) {
                    fb.const_string(name)
                } else {
                    fb.const_int(obj_idx, 64)
                };
                let name_val = fb.const_string(&var_name);
                let args: Vec<ValueId> = if is_scalar {
                    vec![obj_id, name_val]
                } else {
                    vec![obj_id, name_val, dim1]
                };
                let ty = fb.fresh_var();
                let val = fb.call_named("GameMaker.Instance.getOn", &args, ty);
                stack.push(val);
            } else {
                // Unknown dim2: emit a runtime getOn with the dim2 value as the object.
                let name_val = fb.const_string(&var_name);
                let args: Vec<ValueId> = if is_scalar {
                    vec![dim2, name_val]
                } else {
                    vec![dim2, name_val, dim1]
                };
                let ty = fb.fresh_var();
                let val = fb.call_named("GameMaker.Instance.getOn", &args, ty);
                stack.push(val);
            }
        } else {
            // Cross-object indexed access (ref_type != 0, or script context):
            // the instance field identifies the actual target object.
            // E.g. `AceDoll.charSelect[selected]` uses ref_type=0x10 with
            // instance=AceDoll and dim1=selected pushed on the stack.
            let is_scalar = fb.try_get_const(dim1).and_then(const_as_i64) == Some(-1);
            let obj_id = if let Some(name) = ctx.obj_names.get(instance as usize) {
                fb.const_string(name)
            } else {
                fb.const_int(instance as i64, 64)
            };
            let name_val = fb.const_string(&var_name);
            let args: Vec<ValueId> = if is_scalar {
                vec![obj_id, name_val]
            } else {
                vec![obj_id, name_val, dim1]
            };
            let ty = fb.fresh_var();
            let val = fb.call_named("GameMaker.Instance.getOn", &args, ty);
            stack.push(val);
        }
        return Ok(());
    }

    // The GameMaker compiler sometimes uses the owning object's index (or a
    // parent's index) as the instance type for self-references instead of -1
    // (Own). Normalize here.
    //
    // Exception: inside a with-body closure, param(0) is the iteration target
    // (_self), NOT the outer object. A cross-object access to the outer object
    // (instance == outer_obj_idx) must NOT be normalized to Own here — it
    // belongs to the outer instance and should go through the setOn/getOn path.
    let instance = if instance >= 0
        && ctx.has_self
        && !ctx.is_with_body
        && (ctx.self_object_index == Some(instance as usize)
            || ctx.ancestor_indices.contains(&(instance as usize)))
    {
        -1 // Treat as Own (self)
    } else {
        instance
    };

    match InstanceType::from_i16(instance) {
        Some(InstanceType::Local) => {
            // Local variable: load from alloc slot.
            if let Some(&slot) = locals.get(&var_name) {
                let ty = fb.fresh_var();
                let val = fb.load(slot, ty);
                stack.push(val);
            } else {
                // Fallback: create an on-the-fly alloc and register it for reuse.
                let ty = fb.fresh_var();
                let slot = fb.alloc(ty);
                fb.name_value(slot, var_name.clone());
                locals.insert(var_name, slot);
                let ty = fb.fresh_var();
                let val = fb.load(slot, ty);
                stack.push(val);
            }
        }
        Some(InstanceType::Own) | Some(InstanceType::Builtin) => {
            if let Some(arg_idx) = parse_argument_index(&var_name) {
                // Implicit argument variable → function parameter (or captured slot).
                if let Some(&slot) = locals.get(&format!("_argument{arg_idx}")) {
                    let ty = fb.fresh_var();
                    stack.push(fb.load(slot, ty));
                } else {
                    let param_offset =
                        if ctx.has_self { 1 } else { 0 } + if ctx.has_other { 1 } else { 0 };
                    let param = fb.param(param_offset + arg_idx);
                    stack.push(param);
                }
            } else if var_name == "argument_count" {
                // `argument_count` is a GML built-in that returns the number of
                // arguments passed to the current script call.  When the function
                // was emitted with a rest parameter (stored as "_args" in locals),
                // translate this as `args.length`.  Otherwise fall through to the
                // normal self/global field lookup (best-effort fallback).
                if let Some(&rest_id) = locals.get("_args") {
                    let ty = fb.fresh_var();
                    let val = fb.get_field(rest_id, "length", ty);
                    stack.push(val);
                } else if ctx.has_self {
                    let self_param = fb.param(0);
                    let ty = fb.fresh_var();
                    let val = fb.get_field(self_param, &var_name, ty);
                    stack.push(val);
                } else {
                    let name_val = fb.const_string(&var_name);
                    let ty = fb.fresh_var();
                    let val = fb.call_named("GameMaker.Global.get", &[name_val], ty);
                    stack.push(val);
                }
            } else if ctx.has_self {
                let self_param = fb.param(0);
                let ty = fb.fresh_var();
                let val = fb.get_field(self_param, &var_name, ty);
                stack.push(val);
            } else {
                // Script context without self: variable is a global.
                let name_val = fb.const_string(&var_name);
                let ty = fb.fresh_var();
                let val = fb.call_named("GameMaker.Global.get", &[name_val], ty);
                stack.push(val);
            }
        }
        Some(InstanceType::Global) => {
            // GMS2.3+: global.argumentN references are really this function's
            // formal parameters — the VM copies stack args into globals before
            // entry.  Rewrite to use the formal param directly.
            if global_arg_count > 0 {
                if let Some(arg_idx) = parse_argument_index(&var_name) {
                    // If a previous write created a local alloc, read from that.
                    let local_key = format!("argument{arg_idx}");
                    if let Some(&slot) = locals.get(&local_key) {
                        let ty = fb.fresh_var();
                        stack.push(fb.load(slot, ty));
                    } else {
                        let param_offset =
                            if ctx.has_self { 1 } else { 0 } + if ctx.has_other { 1 } else { 0 };
                        let param = fb.param(param_offset + arg_idx);
                        stack.push(param);
                    }
                    return Ok(());
                } else if var_name == "argument_count" {
                    // Same as Own/Builtin argument_count — use rest param length
                    // or fall through to global lookup.
                    if let Some(&rest_id) = locals.get("_args") {
                        let ty = fb.fresh_var();
                        let val = fb.get_field(rest_id, "length", ty);
                        stack.push(val);
                        return Ok(());
                    }
                }
            }
            let name_val = fb.const_string(&var_name);
            let ty = fb.fresh_var();
            let val = fb.call_named("GameMaker.Global.get", &[name_val], ty);
            stack.push(val);
        }
        Some(InstanceType::Other) => {
            if ctx.has_other {
                let other_idx = if ctx.has_self { 1 } else { 0 };
                let other_param = fb.param(other_idx);
                let ty = fb.fresh_var();
                let val = fb.get_field(other_param, &var_name, ty);
                stack.push(val);
            } else {
                let name_val = fb.const_string(&var_name);
                let ty = fb.fresh_var();
                let val = fb.call_named("GameMaker.Instance.getOther", &[name_val], ty);
                stack.push(val);
            }
        }
        Some(InstanceType::All) => {
            let name_val = fb.const_string(&var_name);
            let ty = fb.fresh_var();
            let val = fb.call_named("GameMaker.Instance.getAll", &[name_val], ty);
            stack.push(val);
        }
        Some(InstanceType::Stacktop) => {
            let raw_target = pop(stack, inst)?;
            // In GMS2.3+ struct methods, `Push Int32(-9)` followed by a Stacktop
            // variable access is the "push current struct self" pattern.
            // Resolve to self parameter instead of emitting the literal -9.
            let target = if ctx.has_self {
                if fb
                    .try_resolve_const(raw_target)
                    .and_then(|c| const_as_i64(&c))
                    == Some(-9)
                {
                    fb.param(0)
                } else {
                    raw_target
                }
            } else {
                raw_target
            };
            if var_name == "argument" {
                // argument[N] → function parameter access
                if let Some(idx) = fb.try_get_const(target).and_then(const_as_i64) {
                    let param_offset =
                        if ctx.has_self { 1 } else { 0 } + if ctx.has_other { 1 } else { 0 };
                    let param = fb.param(param_offset + idx as usize);
                    stack.push(param);
                } else if let Some(&rest_id) = locals.get("_args") {
                    // Unknown index with rest param — `argument[expr]` → `args[expr]`.
                    let ty = fb.fresh_var();
                    let val = fb.get_index(rest_id, target, ty);
                    stack.push(val);
                } else {
                    // Unknown index — fall back to getField
                    let name_val = fb.const_string(&var_name);
                    let ty = fb.fresh_var();
                    let val = fb.call_named("GameMaker.Instance.getField", &[target, name_val], ty);
                    stack.push(val);
                }
            } else if ctx.has_self && target == fb.param(0) {
                // Self-field read in struct method — use get_field for clean output.
                let ty = fb.fresh_var();
                let val = fb.get_field(target, &var_name, ty);
                stack.push(val);
            } else if let Some(obj_idx) =
                fb.try_resolve_const(target).and_then(|c| const_as_i64(&c))
            {
                if obj_idx >= 0 {
                    let obj_id = if let Some(name) = ctx.obj_names.get(obj_idx as usize) {
                        fb.const_string(name)
                    } else {
                        fb.const_int(obj_idx, 64)
                    };
                    let name_val = fb.const_string(&var_name);
                    let ty = fb.fresh_var();
                    let val = fb.call_named("GameMaker.Instance.getOn", &[obj_id, name_val], ty);
                    stack.push(val);
                } else {
                    let name_val = fb.const_string(&var_name);
                    let ty = fb.fresh_var();
                    let val = fb.call_named("GameMaker.Instance.getField", &[target, name_val], ty);
                    stack.push(val);
                }
            } else {
                let name_val = fb.const_string(&var_name);
                let ty = fb.fresh_var();
                let val = fb.call_named("GameMaker.Instance.getField", &[target, name_val], ty);
                stack.push(val);
            }
        }
        Some(InstanceType::Arg) => {
            // Argument variable: map to function parameter (or captured slot).
            // variable_id is the VARI table index, not the argument index —
            // extract the actual index from the variable name ("argument3" → 3).
            let arg_idx = parse_argument_index(&var_name).unwrap_or(var_ref.variable_id as usize);
            if let Some(&slot) = locals.get(&format!("_argument{arg_idx}")) {
                let ty = fb.fresh_var();
                stack.push(fb.load(slot, ty));
            } else {
                let param_offset =
                    if ctx.has_self { 1 } else { 0 } + if ctx.has_other { 1 } else { 0 };
                let idx = param_offset + arg_idx;
                if idx < fb.param_count() {
                    let param = fb.param(idx);
                    stack.push(param);
                } else {
                    // Out-of-range argument access — emit as dynamic lookup.
                    let name_val = fb.const_string(format!("argument{arg_idx}"));
                    let ty = fb.fresh_var();
                    let val = fb.call_named("GameMaker.Argument.get", &[name_val], ty);
                    stack.push(val);
                }
            }
        }
        _ => {
            // Positive value = specific object ID.
            if instance >= 0 {
                let obj_id = if let Some(name) = ctx.obj_names.get(instance as usize) {
                    fb.const_string(name)
                } else {
                    fb.const_int(instance as i64, 64)
                };
                let name_val = fb.const_string(&var_name);
                let ty = fb.fresh_var();
                let val = fb.call_named("GameMaker.Instance.getOn", &[obj_id, name_val], ty);
                stack.push(val);
            } else {
                // GMS2.3+ Static (-15) or other unknown negative instance type.
                // Check for argumentN → formal param or captured slot rewrite.
                if let Some(arg_idx) = parse_argument_index(&var_name) {
                    // With-body: captured outer argument as _argumentN local.
                    let captured_key = format!("_argument{arg_idx}");
                    if let Some(&slot) = locals.get(&captured_key) {
                        let ty = fb.fresh_var();
                        stack.push(fb.load(slot, ty));
                        return Ok(());
                    }
                    // Direct function: rewrite to formal param.
                    if global_arg_count > 0 {
                        let local_key = format!("argument{arg_idx}");
                        if let Some(&slot) = locals.get(&local_key) {
                            let ty = fb.fresh_var();
                            stack.push(fb.load(slot, ty));
                        } else {
                            let param_offset = if ctx.has_self { 1 } else { 0 }
                                + if ctx.has_other { 1 } else { 0 };
                            let param = fb.param(param_offset + arg_idx);
                            stack.push(param);
                        }
                        return Ok(());
                    }
                }
                // Treat as global.
                let name_val = fb.const_string(&var_name);
                let ty = fb.fresh_var();
                let val = fb.call_named("GameMaker.Global.get", &[name_val], ty);
                stack.push(val);
            }
        }
    }
    Ok(())
}

/// Translate a Pop instruction (store to variable).
pub(super) fn translate_pop(
    inst: &Instruction,
    fb: &mut FunctionBuilder,
    stack: &mut Vec<ValueId>,
    locals: &mut HashMap<String, ValueId>,
    ctx: &TranslateCtx,
    compound_2d_pending: &mut bool,
    global_arg_count: u16,
) -> Result<(), String> {
    if let Operand::Variable { var_ref, instance } = &inst.operand {
        let var_name = resolve_variable_name(inst, ctx);

        // Handle stacktop-via-ref_type (ref_type == 0x80 with instance >= 0).
        // The target instance is on the stack (top), value to store is below.
        // In GMS2.3+, Push Int32(-9) is the "self" sentinel — resolve to the
        // self parameter directly.
        if is_stacktop_ref(var_ref, *instance) {
            let raw_target = pop(stack, inst)?; // instance (top of stack)
            let value = pop(stack, inst)?; // value to store (below)
            let target = if ctx.has_self {
                if fb
                    .try_resolve_const(raw_target)
                    .and_then(|c| const_as_i64(&c))
                    == Some(-9)
                {
                    fb.param(0)
                } else {
                    raw_target
                }
            } else {
                raw_target
            };
            if ctx.has_self && target == fb.param(0) {
                fb.set_field(target, &var_name, value);
            } else if let Some(obj_idx) =
                fb.try_resolve_const(target).and_then(|c| const_as_i64(&c))
            {
                // Constant integer target = object index pushed before stacktop access.
                // Resolve to setOn(objName, field, value) for clean class-based access.

                if obj_idx >= 0 {
                    let obj_id = if let Some(name) = ctx.obj_names.get(obj_idx as usize) {
                        fb.const_string(name)
                    } else {
                        fb.const_int(obj_idx, 64)
                    };
                    let name_val = fb.const_string(&var_name);
                    fb.call_named(
                        "GameMaker.Instance.setOn",
                        &[obj_id, name_val, value],
                        Type::Void,
                    );
                } else {
                    let name_val = fb.const_string(&var_name);
                    fb.call_named(
                        "GameMaker.Instance.setField",
                        &[target, name_val, value],
                        Type::Void,
                    );
                }
            } else {
                let name_val = fb.const_string(&var_name);
                fb.call_named(
                    "GameMaker.Instance.setField",
                    &[target, name_val, value],
                    Type::Void,
                );
            }
            return Ok(());
        }

        // Handle 2D array access (ref_type == 0 with non-negative instance).
        //
        // Simple assignment: stack is [value, dim2, dim1] with dim1 on top.
        // The value was pushed first, then the indices.
        //
        // Compound assignment (+=, -=, etc.): the compiler Dups the indices
        // before the VARI read, leaving originals below. After the read and
        // arithmetic, the stack becomes [dim2, dim1, new_value] with new_value
        // on top. The `compound_2d_pending` flag is set by translate_push_variable
        // when it detects the originals remaining after the 2D read.
        if is_2d_array_access(var_ref, *instance) {
            let (dim1, dim2, value) = if var_ref.ref_type == 0 && *compound_2d_pending {
                *compound_2d_pending = false;
                // Compound: new_value=top, dim1=next, dim2=bottom
                let value = pop(stack, inst)?;
                let dim1 = pop(stack, inst)?;
                let dim2 = pop(stack, inst)?;
                (dim1, dim2, value)
            } else {
                // Simple: dim1=top, dim2=next, value=bottom
                let dim1 = pop(stack, inst)?;
                let dim2 = pop(stack, inst)?;
                let value = pop(stack, inst)?;
                (dim1, dim2, value)
            };
            if var_name == "argument" {
                // argument[N] = value → store to function parameter slot (or captured slot).
                if let Some(idx) = fb.try_get_const(dim1).and_then(const_as_i64) {
                    if idx >= 0 {
                        let n = idx as usize;
                        if let Some(&slot) = locals.get(&format!("_argument{n}")) {
                            // Inside a with-body: update the captured argument slot.
                            fb.store(slot, value);
                        } else {
                            let param_offset = if ctx.has_self { 1 } else { 0 }
                                + if ctx.has_other { 1 } else { 0 };
                            let abs_idx = param_offset + n;
                            if abs_idx < fb.param_count() {
                                let param = fb.param(abs_idx);
                                let ty = fb.fresh_var();
                                let slot = fb.alloc(ty);
                                fb.store(slot, param);
                                fb.store(slot, value);
                            }
                            // else: OOB (with-body uncaptured arg or invalid game code) — skip
                        }
                    }
                    // Negative index: invalid argument write — skip.
                } else {
                    // Unknown index — fall back to setField
                    let name_val = fb.const_string(&var_name);
                    fb.call_named(
                        "GameMaker.Instance.setField",
                        &[dim1, name_val, value],
                        Type::Void,
                    );
                }
            } else if var_ref.ref_type == 0 {
                // In GMS1, dim2 encodes the scope for ref_type==0 array writes:
                //   dim2 < 0  → self-array write
                //   dim2 >= 0 → cross-object write; dim2 is the OBJT index of
                //               the target object
                let dim2_scope = fb.try_get_const(dim2).cloned();
                let is_self_scope = dim2_scope
                    .as_ref()
                    .and_then(const_as_i64)
                    .is_some_and(|n| n < 0);
                let is_scalar = fb.try_get_const(dim1).and_then(const_as_i64) == Some(-1);
                if is_self_scope {
                    // Self-array write: check locals first (GML resolves
                    // locals before instance fields).
                    if let Some(&slot) = locals.get(&var_name) {
                        if is_scalar {
                            fb.store(slot, value);
                        } else {
                            // Local array write: GML auto-creates an array
                            // when writing to a non-array local via index.
                            let ty = fb.fresh_var();
                            let arr = fb.load(slot, ty);
                            let call_ty = fb.fresh_var();
                            let result =
                                fb.call_named("arrayLocalSet", &[arr, dim1, value], call_ty);
                            fb.store(slot, result);
                        }
                    } else if ctx.has_self {
                        let self_param = fb.param(0);
                        if is_scalar {
                            fb.set_field(self_param, &var_name, value);
                        } else {
                            let ty = fb.fresh_var();
                            let field_val = fb.get_field(self_param, &var_name, ty);
                            fb.set_index(field_val, dim1, value);
                        }
                    } else {
                        // No self param (e.g. room creation code): use setField syscall.
                        let self_id = fb.const_int(-1, 64);
                        let name_val = fb.const_string(&var_name);
                        let args: Vec<ValueId> = if is_scalar {
                            vec![self_id, name_val, value]
                        } else {
                            vec![self_id, name_val, dim1, value]
                        };
                        fb.call_named("GameMaker.Instance.setOn", &args, Type::Void);
                    }
                } else if let Some(obj_idx) = dim2_scope.as_ref().and_then(const_as_i64) {
                    // Cross-object: dim2 is the OBJT index of the target object.
                    let obj_id = if let Some(name) = ctx.obj_names.get(obj_idx as usize) {
                        fb.const_string(name)
                    } else {
                        fb.const_int(obj_idx, 64)
                    };
                    let name_val = fb.const_string(&var_name);
                    let args: Vec<ValueId> = if is_scalar {
                        vec![obj_id, name_val, value]
                    } else {
                        vec![obj_id, name_val, dim1, value]
                    };
                    fb.call_named("GameMaker.Instance.setOn", &args, Type::Void);
                } else {
                    // Unknown dim2: emit a runtime setOn with the dim2 value as the object.
                    let name_val = fb.const_string(&var_name);
                    let args: Vec<ValueId> = if is_scalar {
                        vec![dim2, name_val, value]
                    } else {
                        vec![dim2, name_val, dim1, value]
                    };
                    fb.call_named("GameMaker.Instance.setOn", &args, Type::Void);
                }
            } else {
                // Cross-object indexed write (ref_type != 0, or script context):
                // instance identifies the actual target object.
                let is_scalar = fb.try_get_const(dim1).and_then(const_as_i64) == Some(-1);
                let obj_id = if let Some(name) = ctx.obj_names.get(*instance as usize) {
                    fb.const_string(name)
                } else {
                    fb.const_int(*instance as i64, 64)
                };
                let name_val = fb.const_string(&var_name);
                let args: Vec<ValueId> = if is_scalar {
                    vec![obj_id, name_val, value]
                } else {
                    vec![obj_id, name_val, dim1, value]
                };
                fb.call_named("GameMaker.Instance.setOn", &args, Type::Void);
            }
            return Ok(());
        }

        // Non-2D-array Pop: single value on top of stack.
        let value = pop(stack, inst)?;

        // Normalize self-referencing instance types (see translate_push_variable).
        // Not applied inside with-body closures: param(0) there is the iteration
        // target, not the outer object. Cross-object writes to the outer object
        // must go through setOn, not set_field(param(0)).
        let instance = if *instance >= 0
            && ctx.has_self
            && !ctx.is_with_body
            && (ctx.self_object_index == Some(*instance as usize)
                || ctx.ancestor_indices.contains(&(*instance as usize)))
        {
            -1
        } else {
            *instance
        };

        match InstanceType::from_i16(instance) {
            Some(InstanceType::Local) => {
                if let Some(&slot) = locals.get(&var_name) {
                    fb.store(slot, value);
                } else {
                    // Orphan local — create slot and register for reuse.
                    let ty = fb.fresh_var();
                    let slot = fb.alloc(ty);
                    fb.name_value(slot, var_name.clone());
                    locals.insert(var_name, slot);
                    fb.store(slot, value);
                }
            }
            Some(InstanceType::Own) | Some(InstanceType::Builtin) => {
                if let Some(arg_idx) = parse_argument_index(&var_name) {
                    // Implicit argument variable → store via local slot.
                    let param_offset =
                        if ctx.has_self { 1 } else { 0 } + if ctx.has_other { 1 } else { 0 };
                    if let Some(&slot) = locals.get(&format!("_argument{arg_idx}")) {
                        // Inside a with-body: update the captured argument slot.
                        fb.store(slot, value);
                    } else {
                        let abs_idx = param_offset + arg_idx;
                        if abs_idx < fb.param_count() {
                            let param = fb.param(abs_idx);
                            let ty = fb.fresh_var();
                            let slot = fb.alloc(ty);
                            fb.name_value(slot, var_name.clone());
                            fb.store(slot, param);
                            fb.store(slot, value);
                            locals.insert(var_name, slot);
                        }
                        // else: OOB (with-body uncaptured arg or invalid game code) — skip.
                    }
                } else if ctx.has_self {
                    let self_param = fb.param(0);
                    fb.set_field(self_param, &var_name, value);
                } else {
                    let name_val = fb.const_string(&var_name);
                    fb.call_named("GameMaker.Global.set", &[name_val, value], Type::Void);
                }
            }
            Some(InstanceType::Global) => {
                // GMS2.3+: global.argumentN writes are really writes to this
                // function's formal parameters.  Create an alloc slot so
                // subsequent reads see the updated value.
                if global_arg_count > 0 {
                    if let Some(arg_idx) = parse_argument_index(&var_name) {
                        let param_offset =
                            if ctx.has_self { 1 } else { 0 } + if ctx.has_other { 1 } else { 0 };
                        let abs_idx = param_offset + arg_idx;
                        if abs_idx < fb.param_count() {
                            let param = fb.param(abs_idx);
                            let ty = fb.fresh_var();
                            let slot = fb.alloc(ty);
                            let name = format!("argument{arg_idx}");
                            fb.name_value(slot, name.clone());
                            fb.store(slot, param);
                            fb.store(slot, value);
                            locals.insert(name, slot);
                        }
                        return Ok(());
                    }
                }
                let name_val = fb.const_string(&var_name);
                fb.call_named("GameMaker.Global.set", &[name_val, value], Type::Void);
            }
            Some(InstanceType::Other) => {
                if ctx.has_other {
                    let other_idx = if ctx.has_self { 1 } else { 0 };
                    let other_param = fb.param(other_idx);
                    fb.set_field(other_param, &var_name, value);
                } else {
                    let name_val = fb.const_string(&var_name);
                    fb.call_named(
                        "GameMaker.Instance.setOther",
                        &[name_val, value],
                        Type::Void,
                    );
                }
            }
            Some(InstanceType::All) => {
                let name_val = fb.const_string(&var_name);
                fb.call_named("GameMaker.Instance.setAll", &[name_val, value], Type::Void);
            }
            Some(InstanceType::Stacktop) => {
                let raw_target = pop(stack, inst)?;
                // In GMS2.3+ struct methods, -9 is the self-reference sentinel.
                let target = if ctx.has_self {
                    if fb
                        .try_resolve_const(raw_target)
                        .and_then(|c| const_as_i64(&c))
                        == Some(-9)
                    {
                        fb.param(0)
                    } else {
                        raw_target
                    }
                } else {
                    raw_target
                };
                if var_name == "argument" {
                    // argument[N] = value → store to function parameter slot
                    if let Some(idx) = fb.try_get_const(target).and_then(const_as_i64) {
                        if idx >= 0 {
                            let n = idx as usize;
                            let param_offset = if ctx.has_self { 1 } else { 0 }
                                + if ctx.has_other { 1 } else { 0 };
                            if let Some(&slot) = locals.get(&format!("_argument{n}")) {
                                // Inside a with-body: update the captured argument slot.
                                fb.store(slot, value);
                            } else {
                                let abs_idx = param_offset + n;
                                if abs_idx < fb.param_count() {
                                    let param = fb.param(abs_idx);
                                    let ty = fb.fresh_var();
                                    let slot = fb.alloc(ty);
                                    fb.store(slot, param);
                                    fb.store(slot, value);
                                }
                                // else: OOB — skip.
                            }
                        }
                        // Negative index: invalid argument write — skip.
                    } else {
                        // Unknown index — fall back to setField
                        let name_val = fb.const_string(&var_name);
                        fb.call_named(
                            "GameMaker.Instance.setField",
                            &[target, name_val, value],
                            Type::Void,
                        );
                    }
                } else if ctx.has_self && target == fb.param(0) {
                    // Self-field write in struct method — use set_field for clean output.
                    fb.set_field(target, &var_name, value);
                } else if let Some(obj_idx) =
                    fb.try_resolve_const(target).and_then(|c| const_as_i64(&c))
                {
                    if obj_idx >= 0 {
                        let obj_id = if let Some(name) = ctx.obj_names.get(obj_idx as usize) {
                            fb.const_string(name)
                        } else {
                            fb.const_int(obj_idx, 64)
                        };
                        let name_val = fb.const_string(&var_name);
                        fb.call_named(
                            "GameMaker.Instance.setOn",
                            &[obj_id, name_val, value],
                            Type::Void,
                        );
                    } else {
                        let name_val = fb.const_string(&var_name);
                        fb.call_named(
                            "GameMaker.Instance.setField",
                            &[target, name_val, value],
                            Type::Void,
                        );
                    }
                } else {
                    let name_val = fb.const_string(&var_name);
                    fb.call_named(
                        "GameMaker.Instance.setField",
                        &[target, name_val, value],
                        Type::Void,
                    );
                }
            }
            _ => {
                if instance >= 0 {
                    let obj_id = if let Some(name) = ctx.obj_names.get(instance as usize) {
                        fb.const_string(name)
                    } else {
                        fb.const_int(instance as i64, 64)
                    };
                    let name_val = fb.const_string(&var_name);
                    fb.call_named(
                        "GameMaker.Instance.setOn",
                        &[obj_id, name_val, value],
                        Type::Void,
                    );
                } else {
                    // GMS2.3+ Static (-15) or other unknown negative instance.
                    // Check for argumentN → captured slot or formal param rewrite.
                    if let Some(arg_idx) = parse_argument_index(&var_name) {
                        // With-body: store to captured outer argument slot.
                        let captured_key = format!("_argument{arg_idx}");
                        if let Some(&slot) = locals.get(&captured_key) {
                            fb.store(slot, value);
                            return Ok(());
                        }
                        // Direct function: rewrite to formal param alloc.
                        if global_arg_count > 0 {
                            let param_offset = if ctx.has_self { 1 } else { 0 }
                                + if ctx.has_other { 1 } else { 0 };
                            let abs_idx = param_offset + arg_idx;
                            if abs_idx < fb.param_count() {
                                let param = fb.param(abs_idx);
                                let ty = fb.fresh_var();
                                let slot = fb.alloc(ty);
                                let name = format!("argument{arg_idx}");
                                fb.name_value(slot, name.clone());
                                fb.store(slot, param);
                                fb.store(slot, value);
                                locals.insert(name, slot);
                            }
                            return Ok(());
                        }
                    }
                    let name_val = fb.const_string(&var_name);
                    fb.call_named("GameMaker.Global.set", &[name_val, value], Type::Void);
                }
            }
        }
    } else {
        // Pop without variable destination: just discard.
        let _ = pop(stack, inst)?;
    }
    Ok(())
}
