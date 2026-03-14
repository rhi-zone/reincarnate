//! GML boolean-in-arithmetic coercion pass.
//!
//! GML treats booleans as integers (0/1) in arithmetic contexts.  A comparison
//! like `this.hp < half_hp` produces a `Bool`-typed value that the game author
//! then uses directly in arithmetic: `speed * half_hp`, `base + half_hp * 10`.
//!
//! This is valid GML semantics — booleans ARE numbers in GML.  TypeScript does
//! not allow `bool * number` (`TS2362`/`TS2365`), so we insert an explicit cast
//! `Bool → Float(64)` before each Bool operand of an arithmetic instruction.
//! The cast prints as `Number(expr)` in TypeScript, which is a no-op at runtime
//! (`true | 0` → `1`) but satisfies the type checker.
//!
//! # Arithmetic ops covered
//! `Add`, `Sub`, `Mul`, `Div`, `Rem`.  Bitwise ops (`BitAnd`, `BitOr`, etc.)
//! are excluded: TypeScript already accepts boolean operands for bitwise.
//!
//! # Fix A — Bool-returning callee in arithmetic (TS2365)
//! When a function returns `Bool` in its sig but ConstraintSolve widens the
//! call-result `value_types` entry to `i64`, the arithmetic pass would miss it.
//! We pre-collect Bool-returning callee names and treat Call-result operands
//! whose callee returns Bool the same as directly-Bool operands.
//!
//! # Fix C — Bool args passed to Int/Float block params via Br/BrIf (TS2322)
//! After Mem2Reg, a pattern like `phase = cmp_result` becomes a Br/BrIf arg
//! passing a Bool value to an Int/Float-typed block parameter.  We insert a
//! `Cast(Bool → param_ty, Coerce)` on the arg side so the types agree.

use std::collections::{HashMap, HashSet};

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::block::BlockId;
use reincarnate_core::ir::inst::{CastKind, CmpKind, Inst, InstId, Op};
use reincarnate_core::ir::module::StructDef;
use reincarnate_core::ir::ty::{parse_type_notation, Type};
use reincarnate_core::ir::value::Constant;
use reincarnate_core::ir::{Function, Module, ValueId};
use reincarnate_core::pipeline::{PureIrPass, Transform, TransformResult};

pub struct GmlBoolArithCoerce;

impl Transform for GmlBoolArithCoerce {
    fn name(&self) -> &str {
        "gml-bool-arith-coerce"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(&self, mut module: Module) -> Result<TransformResult, CoreError> {
        // Pre-collect callee names that return Bool (Fix A).
        let bool_returning: HashSet<String> = module
            .functions
            .values()
            .filter(|f| f.sig.return_ty == Type::Bool)
            .map(|f| f.name.clone())
            .collect();

        // Build field type lookup from struct definitions for SetField coercion.
        let struct_field_types = build_struct_field_type_map(&module.structs);

        // Collect numeric field names from external type definitions (e.g.
        // GMLObject.x, GMLObject.depth) so we don't hardcode field names.
        let external_numeric_fields = build_external_numeric_fields(&module);

        // Collect "number | boolean" fields (declared as "*" in external type defs).
        let dynamic_declared_fields = build_dynamic_declared_fields(&module);

        // Pre-collect entry block param types for call-arg coercion (Pass 4).
        // Maps function name → vec of entry block param value_types (skipping
        // the first param which is `self` for instance methods).
        let callee_param_types: HashMap<String, Vec<Type>> = module
            .functions
            .values()
            .map(|f| {
                let entry = &f.blocks[f.entry];
                let tys: Vec<Type> = entry
                    .params
                    .iter()
                    .map(|p| f.value_types[p.value].clone())
                    .collect();
                (f.name.clone(), tys)
            })
            .collect();

        // Pre-collect external (runtime) function param types for call-arg coercion.
        let external_param_types: HashMap<String, Vec<Type>> = module
            .external_function_sigs
            .iter()
            .map(|(name, sig)| {
                let tys: Vec<Type> = sig.params.iter().map(|p| parse_type_notation(p)).collect();
                (name.clone(), tys)
            })
            .collect();

        let mut changed = false;
        for func in module.functions.values_mut() {
            changed |= coerce_bool_arithmetic(func, &bool_returning, &dynamic_declared_fields);
            changed |= coerce_bool_br_args(func);
            changed |= coerce_bool_set_field(func, &struct_field_types, &external_numeric_fields);
            changed |= coerce_bool_call_args(func, &callee_param_types);
            changed |= coerce_call_args_general(func, &callee_param_types, &external_param_types);
            changed |= coerce_bool_cmp_operands(func, &bool_returning, &dynamic_declared_fields);
            changed |= coerce_noone_sentinel(func);
        }
        Ok(TransformResult { module, changed })
    }
}

impl PureIrPass for GmlBoolArithCoerce {}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true if `v` has type `Bool` in `func.value_types`.
fn is_bool(func: &Function, v: ValueId) -> bool {
    matches!(func.value_types.get(v), Some(Type::Bool))
}

/// Returns true if `v` is "effectively boolean" — either directly Bool-typed,
/// or the result of a `Cast(bool_val, Dynamic, Coerce)` from a Bool value.
///
/// GML's `cmp.eq`/`cmp.lt` etc. produce Bool, which then gets coerced to
/// Dynamic before being passed as a call arg.  The emitter strips the coerce,
/// leaving a boolean expression where TypeScript expects a number.
fn is_effectively_bool(func: &Function, v: ValueId) -> bool {
    if is_bool(func, v) {
        return true;
    }
    // Look through Cast(source, _, Coerce) to see if source is Bool.
    let result_map = result_inst_map(func);
    if let Some(&inst_id) = result_map.get(&v) {
        if let Op::Cast(source, _, CastKind::Coerce) = &func.insts[inst_id].op {
            return is_bool(func, *source);
        }
    }
    false
}

/// Returns true if `ty` is an integer or float type (needs coercion from Bool).
fn is_numeric(ty: &Type) -> bool {
    matches!(ty, Type::Int(_) | Type::Float(_))
}

/// Look through a `Cast(src, Dynamic, Coerce)` to recover the underlying type.
///
/// GML translates most call arguments as `coerce val, dyn` before passing them,
/// which hides the real type from the auto-coercion pass.  This helper peels off
/// that wrapper so we can see the original type and insert the right coercion.
fn peel_dynamic_coerce<'a>(
    func: &'a Function,
    v: ValueId,
    result_map: &HashMap<ValueId, InstId>,
) -> &'a Type {
    let ty = &func.value_types[v];
    if !matches!(ty, Type::Dynamic) {
        return ty;
    }
    if let Some(&inst_id) = result_map.get(&v) {
        if let Op::Cast(source, _, CastKind::Coerce) = &func.insts[inst_id].op {
            let src_ty = &func.value_types[*source];
            if !matches!(src_ty, Type::Dynamic) {
                return src_ty;
            }
        }
    }
    ty
}

/// Build a reverse map: ValueId → InstId (the instruction that produces it).
fn result_inst_map(func: &Function) -> HashMap<ValueId, InstId> {
    func.insts
        .iter()
        .filter_map(|(id, inst)| inst.result.map(|v| (v, id)))
        .collect()
}

/// Insert `Cast(v, to_type, Coerce)` before `before_inst_id` in the block
/// that contains it, and return the new ValueId.
fn insert_cast_before(
    func: &mut Function,
    v: ValueId,
    before_inst_id: InstId,
    to_type: Type,
) -> ValueId {
    let cast_vid = func.value_types.push(to_type.clone());
    let cast_inst_id = func.insts.push(Inst {
        op: Op::Cast(v, to_type, CastKind::Coerce),
        result: Some(cast_vid),
        span: None,
    });
    // Find the block containing before_inst_id and insert cast just before it.
    'outer: for block in func.blocks.values_mut() {
        for (pos, &iid) in block.insts.iter().enumerate() {
            if iid == before_inst_id {
                block.insts.insert(pos, cast_inst_id);
                break 'outer;
            }
        }
    }
    cast_vid
}

// ---------------------------------------------------------------------------
// Pass 1 — Arithmetic operands (includes Fix A)
// ---------------------------------------------------------------------------

/// Returns true if `v` needs a Bool→numeric coercion before use in arithmetic.
/// Direct Bool values (value_types == Bool) and Call results from Bool-returning
/// callees (Fix A: ConstraintSolve may have widened the result type) both qualify.
///
/// Also coerces Dynamic-typed GetField results for fields declared as `"*"` in
/// external type definitions (e.g. `visible`, `persistent`).  These fields are
/// `number | boolean` in the TypeScript runtime class; TypeScript rejects
/// `number | boolean` in arithmetic, so we wrap them in `Number()`.
fn needs_arith_coerce(
    func: &Function,
    v: ValueId,
    result_map: &HashMap<ValueId, InstId>,
    bool_returning: &HashSet<String>,
    dynamic_declared_fields: &HashSet<String>,
) -> bool {
    if is_bool(func, v) {
        return true;
    }
    if let Some(&inst_id) = result_map.get(&v) {
        // Fix A: value_types[v] was widened by ConstraintSolve, but the callee
        // sig still says Bool — the emitter will emit a boolean-typed expression.
        if let Op::Call {
            func: callee_name, ..
        } = &func.insts[inst_id].op
        {
            if bool_returning.contains(callee_name) {
                return true;
            }
        }
        // GetField on a "number | boolean" declared field (e.g. visible, solid,
        // persistent) — TypeScript can't do arithmetic on `number | boolean`.
        if let Op::GetField { field, .. } = &func.insts[inst_id].op {
            if dynamic_declared_fields.contains(field.as_str()) {
                return true;
            }
        }
    }
    false
}

fn coerce_bool_arithmetic(
    func: &mut Function,
    bool_returning: &HashSet<String>,
    dynamic_declared_fields: &HashSet<String>,
) -> bool {
    let result_map = result_inst_map(func);

    // Collect all arithmetic ops where at least one operand needs coercion.
    let targets: Vec<(InstId, ValueId, ValueId, bool, bool)> = func
        .insts
        .iter()
        .filter_map(|(id, inst)| {
            let (a, b) = match &inst.op {
                Op::Add(a, b) | Op::Sub(a, b) | Op::Mul(a, b) | Op::Div(a, b) | Op::Rem(a, b) => {
                    (*a, *b)
                }
                _ => return None,
            };
            let a_coerce = needs_arith_coerce(
                func,
                a,
                &result_map,
                bool_returning,
                dynamic_declared_fields,
            );
            let b_coerce = needs_arith_coerce(
                func,
                b,
                &result_map,
                bool_returning,
                dynamic_declared_fields,
            );
            if a_coerce || b_coerce {
                Some((id, a, b, a_coerce, b_coerce))
            } else {
                None
            }
        })
        .collect();

    if targets.is_empty() {
        return false;
    }

    for (inst_id, lhs, rhs, lhs_coerce, rhs_coerce) in targets {
        let new_lhs = if lhs_coerce {
            insert_cast_before(func, lhs, inst_id, Type::Float(64))
        } else {
            lhs
        };
        let new_rhs = if rhs_coerce {
            insert_cast_before(func, rhs, inst_id, Type::Float(64))
        } else {
            rhs
        };
        match &mut func.insts[inst_id].op {
            Op::Add(a, b) | Op::Sub(a, b) | Op::Mul(a, b) | Op::Div(a, b) | Op::Rem(a, b) => {
                *a = new_lhs;
                *b = new_rhs;
            }
            _ => {}
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Pass 2 — Br/BrIf block-param args (Fix C)
// ---------------------------------------------------------------------------

fn coerce_bool_br_args(func: &mut Function) -> bool {
    // Pre-build: BlockId → Vec<param_ty> for target-block param type lookup.
    // Use value_types[param.value] instead of param.ty — param.ty can be stale
    // (set by Mem2Reg at creation time, not updated by later passes like
    // IntToBoolPromotion which changes value_types but not param.ty).
    let block_param_tys: HashMap<BlockId, Vec<Type>> = func
        .blocks
        .iter()
        .map(|(bid, b)| {
            (
                bid,
                b.params
                    .iter()
                    .map(|p| func.value_types[p.value].clone())
                    .collect(),
            )
        })
        .collect();

    // Collect casts needed: (inst_id, arm, arg_pos, old_v, to_type)
    // arm: 0 = Br, 1 = BrIf then_args, 2 = BrIf else_args
    let mut casts: Vec<(InstId, u8, usize, ValueId, Type)> = Vec::new();

    for (inst_id, inst) in func.insts.iter() {
        match &inst.op {
            Op::Br { target, args } => {
                if let Some(param_tys) = block_param_tys.get(target) {
                    for (i, &v) in args.iter().enumerate() {
                        if let Some(pty) = param_tys.get(i) {
                            if is_bool(func, v) && is_numeric(pty) {
                                // Always cast to Float(64) — the printer emits
                                // Number(expr) for Float, but has no handler for
                                // Int(64) Coerce casts (they fall through as no-op).
                                casts.push((inst_id, 0, i, v, Type::Float(64)));
                            }
                        }
                    }
                }
            }
            Op::BrIf {
                then_target,
                then_args,
                else_target,
                else_args,
                ..
            } => {
                if let Some(param_tys) = block_param_tys.get(then_target) {
                    for (i, &v) in then_args.iter().enumerate() {
                        if let Some(pty) = param_tys.get(i) {
                            if is_bool(func, v) && is_numeric(pty) {
                                casts.push((inst_id, 1, i, v, Type::Float(64)));
                            }
                        }
                    }
                }
                if let Some(param_tys) = block_param_tys.get(else_target) {
                    for (i, &v) in else_args.iter().enumerate() {
                        if let Some(pty) = param_tys.get(i) {
                            if is_bool(func, v) && is_numeric(pty) {
                                casts.push((inst_id, 2, i, v, Type::Float(64)));
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    if casts.is_empty() {
        return false;
    }

    for (inst_id, arm, pos, old_v, to_type) in casts {
        let new_v = insert_cast_before(func, old_v, inst_id, to_type);
        match &mut func.insts[inst_id].op {
            Op::Br { args, .. } if arm == 0 => args[pos] = new_v,
            Op::BrIf { then_args, .. } if arm == 1 => then_args[pos] = new_v,
            Op::BrIf { else_args, .. } if arm == 2 => else_args[pos] = new_v,
            _ => {}
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Pass 3 — Bool values stored via SetField (TS2322)
// ---------------------------------------------------------------------------

/// Build a map of (field_name → Type) from all struct definitions.
/// If multiple structs define the same field with different types, the entry
/// is removed (ambiguous — don't coerce).
fn build_struct_field_type_map(structs: &[StructDef]) -> HashMap<String, Option<Type>> {
    let mut map: HashMap<String, Option<Type>> = HashMap::new();
    for s in structs {
        for field in &s.fields {
            let entry = map
                .entry(field.name.clone())
                .or_insert(Some(field.ty.clone()));
            if let Some(existing) = entry {
                if *existing != field.ty {
                    *entry = None;
                }
            }
        }
    }
    map
}

/// Build a set of numeric field names from external type definitions.
///
/// Reads `module.external_type_defs` and collects all fields whose type
/// notation parses to a numeric type (Int or Float). This replaces a
/// previously hardcoded list of GML built-in property names.
fn build_external_numeric_fields(module: &Module) -> HashSet<String> {
    use reincarnate_core::ir::ty::parse_type_notation;
    let mut result = HashSet::new();
    for ext in module.external_type_defs.values() {
        for (field_name, type_str) in &ext.fields {
            let ty = parse_type_notation(type_str);
            if is_numeric(&ty) {
                result.insert(field_name.clone());
            }
        }
    }
    result
}

/// Build a set of field names declared as `"*"` (Dynamic) in external type
/// definitions.  These are GML properties like `visible`, `solid`,
/// `persistent` that accept both `number` and `boolean`.  The TypeScript
/// runtime class declares them as `number | boolean`, which TypeScript
/// rejects in arithmetic contexts.  We need to coerce these with `Number()`
/// when they appear as operands in arithmetic expressions.
fn build_dynamic_declared_fields(module: &Module) -> HashSet<String> {
    let mut result = HashSet::new();
    for ext in module.external_type_defs.values() {
        for (field_name, type_str) in &ext.fields {
            if type_str == "*" {
                result.insert(field_name.clone());
            }
        }
    }
    result
}

/// Coerce Bool values stored via SetField to Float(64) when the target field
/// is known to be numeric.
///
/// GML treats booleans as numbers (true=1, false=0), so assignments like
/// `this.image_index = (y > threshold)` are valid GML but fail TypeScript
/// (TS2322: "Type 'boolean' is not assignable to type 'number'"). We insert
/// Cast(Bool→Float(64), Coerce) only for fields known to be numeric — either
/// GML built-in numeric properties or fields declared as numeric in structs.
fn coerce_bool_set_field(
    func: &mut Function,
    struct_field_types: &HashMap<String, Option<Type>>,
    external_numeric_fields: &HashSet<String>,
) -> bool {
    let targets: Vec<(InstId, ValueId)> = func
        .insts
        .iter()
        .filter_map(|(id, inst)| {
            if let Op::SetField { field, value, .. } = &inst.op {
                if is_bool(func, *value) {
                    // Check external type definitions (e.g. GMLObject built-ins).
                    if external_numeric_fields.contains(field.as_str()) {
                        return Some((id, *value));
                    }
                    // Check struct-defined numeric fields.
                    if let Some(Some(ty)) = struct_field_types.get(field) {
                        if is_numeric(ty) {
                            return Some((id, *value));
                        }
                    }
                }
            }
            None
        })
        .collect();

    if targets.is_empty() {
        return false;
    }

    for (inst_id, old_v) in targets {
        let new_v = insert_cast_before(func, old_v, inst_id, Type::Float(64));
        if let Op::SetField { value, .. } = &mut func.insts[inst_id].op {
            *value = new_v;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Pass 4 — Bool args passed to numeric callee params via Call (TS2345)
// ---------------------------------------------------------------------------

/// Coerce Bool arguments in function calls when the callee's entry block param
/// is numeric.  GML treats booleans as numbers, so `func(x == y)` is valid GML
/// when `func` expects a number.
///
/// The challenge: at call sites, the Bool value is often coerced to Dynamic
/// (`coerce bool, dyn`) before being passed.  The emitter strips the coerce,
/// exposing the boolean expression to TypeScript.  We look through the coerce
/// to find the underlying Bool and insert `Cast(Bool→Float(64), Coerce)`.
fn coerce_bool_call_args(
    func: &mut Function,
    callee_param_types: &HashMap<String, Vec<Type>>,
) -> bool {
    // Collect: (inst_id, arg_index, old_value)
    let mut casts: Vec<(InstId, usize, ValueId)> = Vec::new();

    for (inst_id, inst) in func.insts.iter() {
        if let Op::Call {
            func: callee_name,
            args,
        } = &inst.op
        {
            if let Some(param_tys) = callee_param_types.get(callee_name) {
                // Call args map directly to entry block params: args[i]
                // corresponds to entry_params[i]. The emitter adds _rt as
                // an extra first argument, but the IR doesn't have it.
                for (i, &arg_v) in args.iter().enumerate() {
                    if let Some(pty) = param_tys.get(i) {
                        if is_numeric(pty) && is_effectively_bool(func, arg_v) {
                            casts.push((inst_id, i, arg_v));
                        }
                    }
                }
            }
        }
    }

    if casts.is_empty() {
        return false;
    }

    for (inst_id, arg_idx, old_v) in casts {
        let new_v = insert_cast_before(func, old_v, inst_id, Type::Float(64));
        if let Op::Call { args, .. } = &mut func.insts[inst_id].op {
            args[arg_idx] = new_v;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Pass 4b — General call-arg type coercion (TS2345)
// ---------------------------------------------------------------------------

/// GML auto-coerces between types at call sites: numbers become strings,
/// strings become numbers, booleans become numbers.  TypeScript doesn't.
/// Insert explicit `Cast(..., Coerce)` when arg type ≠ param type and both
/// are concrete (non-Dynamic).
///
/// Covers: number→string, string→number, bool→string.
/// Does NOT coerce: Dynamic→anything (already compatible), struct→number
/// (instance ID problem — different root cause).
fn coerce_call_args_general(
    func: &mut Function,
    callee_param_types: &HashMap<String, Vec<Type>>,
    external_param_types: &HashMap<String, Vec<Type>>,
) -> bool {
    let result_map = result_inst_map(func);
    let mut casts: Vec<(InstId, usize, ValueId, Type)> = Vec::new();

    for (inst_id, inst) in func.insts.iter() {
        if let Op::Call {
            func: callee_name,
            args,
        } = &inst.op
        {
            // Look up param types from internal functions first, then external.
            let param_tys = callee_param_types
                .get(callee_name)
                .or_else(|| external_param_types.get(callee_name.as_str()));

            if let Some(param_tys) = param_tys {
                for (i, &arg_v) in args.iter().enumerate() {
                    if let Some(pty) = param_tys.get(i) {
                        // Look through coerce-to-Dynamic to recover the real arg type.
                        let arg_ty = peel_dynamic_coerce(func, arg_v, &result_map);
                        if let Some(target) = needs_coerce(arg_ty, pty) {
                            casts.push((inst_id, i, arg_v, target));
                        }
                    }
                }
            }
        }
    }

    if casts.is_empty() {
        return false;
    }

    for (inst_id, arg_idx, old_v, to_type) in casts {
        let new_v = insert_cast_before(func, old_v, inst_id, to_type);
        if let Op::Call { args, .. } = &mut func.insts[inst_id].op {
            args[arg_idx] = new_v;
        }
    }

    true
}

/// Determine if a GML auto-coercion is needed from `arg_ty` to `param_ty`.
/// Returns `Some(target_type)` if a cast should be inserted, `None` otherwise.
fn needs_coerce(arg_ty: &Type, param_ty: &Type) -> Option<Type> {
    // Skip if types already match or either side is Dynamic (compatible with anything).
    if arg_ty == param_ty {
        return None;
    }
    if matches!(arg_ty, Type::Dynamic) || matches!(param_ty, Type::Dynamic) {
        return None;
    }
    // Skip Void args (shouldn't happen but guard).
    if matches!(arg_ty, Type::Void) || matches!(param_ty, Type::Void) {
        return None;
    }

    match (arg_ty, param_ty) {
        // number → string: GML does `string(val)` automatically
        (Type::Float(_) | Type::Int(_) | Type::UInt(_), Type::String) => Some(Type::String),
        // string → number: GML does `real(val)` automatically
        (Type::String, Type::Float(64)) => Some(Type::Float(64)),
        (Type::String, Type::Int(w)) => Some(Type::Int(*w)),
        // bool → string: GML converts to "0"/"1"
        (Type::Bool, Type::String) => Some(Type::String),
        // bool → number: already handled by coerce_bool_call_args,
        // but catch any stragglers
        (Type::Bool, Type::Float(64) | Type::Int(_)) => Some(Type::Float(64)),
        // number → bool: GML treats 0 as false, nonzero as true
        (Type::Float(_) | Type::Int(_), Type::Bool) => Some(Type::Bool),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Pass 5 — Bool operand in Cmp with numeric other side (TS2367)
// ---------------------------------------------------------------------------

/// Try to resolve a ValueId to its compile-time constant (following Copy chains).
fn try_get_const(func: &Function, v: ValueId) -> Option<Constant> {
    for inst in func.insts.values() {
        if inst.result == Some(v) {
            match &inst.op {
                Op::Const(c) => return Some(c.clone()),
                Op::Copy(src) => return try_get_const(func, *src),
                _ => return None,
            }
        }
    }
    None
}

/// Convert a numeric constant to its boolean equivalent (0→false, 1→true).
/// Returns None for non-0/1 values.
fn numeric_const_to_bool(c: &Constant) -> Option<bool> {
    match c {
        Constant::Int(0) | Constant::UInt(0) => Some(false),
        Constant::Int(1) | Constant::UInt(1) => Some(true),
        Constant::Float(f) if *f == 0.0 => Some(false),
        Constant::Float(f) if *f == 1.0 => Some(true),
        _ => None,
    }
}

/// Insert a `Const(Bool(val))` before `before_inst_id` and return the new ValueId.
fn insert_bool_const_before(func: &mut Function, val: bool, before_inst_id: InstId) -> ValueId {
    let vid = func.value_types.push(Type::Bool);
    let iid = func.insts.push(Inst {
        op: Op::Const(Constant::Bool(val)),
        result: Some(vid),
        span: None,
    });
    'outer: for block in func.blocks.values_mut() {
        for (pos, &existing) in block.insts.iter().enumerate() {
            if existing == before_inst_id {
                block.insts.insert(pos, iid);
                break 'outer;
            }
        }
    }
    vid
}

/// Coerce Bool-vs-numeric comparisons so TypeScript doesn't flag TS2367.
///
/// GML treats booleans as numbers, so `(x > y) === 0` is valid GML.
/// Instead of `Number(x > y) === 0` (which is a suppression), we replace
/// the numeric constant with its boolean equivalent:
///   - `bool === 0` → `bool === false`
///   - `bool === 1` → `bool === true`
///   - `bool !== 0` → `bool !== false`
///
/// For non-constant numeric operands or values other than 0/1, falls back
/// to `Cast(Bool→Float(64))` (emits `Number(expr)`).
fn coerce_bool_cmp_operands(
    func: &mut Function,
    bool_returning: &HashSet<String>,
    _dynamic_declared_fields: &HashSet<String>,
) -> bool {
    let result_map = result_inst_map(func);
    // For comparisons, don't coerce Dynamic GetField results — only Bool operands.
    let cmp_fields = &HashSet::new();

    // Each entry: (inst_id, lhs, rhs, lhs_is_bool, rhs_is_bool)
    let targets: Vec<(InstId, ValueId, ValueId, bool, bool)> = func
        .insts
        .iter()
        .filter_map(|(id, inst)| {
            let (a, b) = match &inst.op {
                Op::Cmp(_, a, b) => (*a, *b),
                _ => return None,
            };
            let a_bool = needs_arith_coerce(func, a, &result_map, bool_returning, cmp_fields);
            let b_bool = needs_arith_coerce(func, b, &result_map, bool_returning, cmp_fields);
            let a_num = is_numeric(&func.value_types[a]);
            let b_num = is_numeric(&func.value_types[b]);
            // Only when one side is bool and the other is numeric.
            let coerce_a = a_bool && b_num;
            let coerce_b = b_bool && a_num;
            if coerce_a || coerce_b {
                Some((id, a, b, coerce_a, coerce_b))
            } else {
                None
            }
        })
        .collect();

    if targets.is_empty() {
        return false;
    }

    for (inst_id, lhs, rhs, lhs_is_bool, rhs_is_bool) in targets {
        // Strategy: prefer replacing the numeric side with a bool constant.
        // Fall back to Cast(Bool→Float(64)) only if the numeric side isn't 0 or 1.
        if lhs_is_bool {
            // lhs is Bool, rhs is numeric — try to replace rhs with bool const
            if let Some(bool_val) = try_get_const(func, rhs).and_then(|c| numeric_const_to_bool(&c))
            {
                let new_rhs = insert_bool_const_before(func, bool_val, inst_id);
                if let Op::Cmp(_, _, b) = &mut func.insts[inst_id].op {
                    *b = new_rhs;
                }
            } else {
                // Non-constant or not 0/1 — cast the bool side to number
                let new_lhs = insert_cast_before(func, lhs, inst_id, Type::Float(64));
                if let Op::Cmp(_, a, _) = &mut func.insts[inst_id].op {
                    *a = new_lhs;
                }
            }
        } else if rhs_is_bool {
            // rhs is Bool, lhs is numeric — try to replace lhs with bool const
            if let Some(bool_val) = try_get_const(func, lhs).and_then(|c| numeric_const_to_bool(&c))
            {
                let new_lhs = insert_bool_const_before(func, bool_val, inst_id);
                if let Op::Cmp(_, a, _) = &mut func.insts[inst_id].op {
                    *a = new_lhs;
                }
            } else {
                // Non-constant or not 0/1 — cast the bool side to number
                let new_rhs = insert_cast_before(func, rhs, inst_id, Type::Float(64));
                if let Op::Cmp(_, _, b) = &mut func.insts[inst_id].op {
                    *b = new_rhs;
                }
            }
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Pass 6 — Noone sentinel translation (TS2367)
// ---------------------------------------------------------------------------

/// GML uses `-4` as the `noone` sentinel; our runtime uses `null`.
/// In equality comparisons (`==`/`!=`) where one side is the constant `-4`,
/// replace it with `null` so that:
///   (a) `instance_find() !== null` is semantically correct (our runtime
///       returns null, not -4), and
///   (b) TypeScript doesn't flag TS2367 for comparing an object/null type
///       against a number literal.
///
/// Only fires for equality comparisons where the other operand originates
/// from a function call (which could return an object-or-noone), preventing
/// false positives on genuine `counter === -4` patterns.
fn coerce_noone_sentinel(func: &mut Function) -> bool {
    fn is_noone_const(c: &Constant) -> bool {
        match c {
            Constant::Float(f) => *f == -4.0,
            Constant::Int(n) => *n == -4,
            _ => false,
        }
    }

    /// Check if a value originates from a Call/SystemCall/MethodCall instruction.
    fn is_call_result(func: &Function, v: ValueId) -> bool {
        for inst in func.insts.values() {
            if inst.result == Some(v) {
                return matches!(
                    &inst.op,
                    Op::Call { .. } | Op::SystemCall { .. } | Op::MethodCall { .. }
                );
            }
        }
        false
    }

    let targets: Vec<(InstId, bool, bool)> = func
        .insts
        .iter()
        .filter_map(|(id, inst)| {
            let (kind, a, b) = match &inst.op {
                Op::Cmp(kind, a, b) => (kind, *a, *b),
                _ => return None,
            };
            // Only equality comparisons — noone checks are always == or !=.
            if !matches!(kind, CmpKind::Eq | CmpKind::Ne) {
                return None;
            }
            let a_noone = try_get_const(func, a).is_some_and(|c| is_noone_const(&c));
            let b_noone = try_get_const(func, b).is_some_and(|c| is_noone_const(&c));
            if !a_noone && !b_noone {
                return None;
            }
            // Only replace when the OTHER side comes from a function call.
            let replace_a = a_noone && is_call_result(func, b);
            let replace_b = b_noone && is_call_result(func, a);
            if replace_a || replace_b {
                Some((id, replace_a, replace_b))
            } else {
                None
            }
        })
        .collect();

    if targets.is_empty() {
        return false;
    }

    for (inst_id, replace_a, replace_b) in targets {
        if replace_a {
            let null_vid = func.value_types.push(Type::Option(Box::new(Type::Dynamic)));
            let null_iid = func.insts.push(Inst {
                op: Op::Const(Constant::Null),
                result: Some(null_vid),
                span: None,
            });
            'outer_a: for block in func.blocks.values_mut() {
                for (pos, &existing) in block.insts.iter().enumerate() {
                    if existing == inst_id {
                        block.insts.insert(pos, null_iid);
                        break 'outer_a;
                    }
                }
            }
            if let Op::Cmp(_, a, _) = &mut func.insts[inst_id].op {
                *a = null_vid;
            }
        }
        if replace_b {
            let null_vid = func.value_types.push(Type::Option(Box::new(Type::Dynamic)));
            let null_iid = func.insts.push(Inst {
                op: Op::Const(Constant::Null),
                result: Some(null_vid),
                span: None,
            });
            'outer_b: for block in func.blocks.values_mut() {
                for (pos, &existing) in block.insts.iter().enumerate() {
                    if existing == inst_id {
                        block.insts.insert(pos, null_iid);
                        break 'outer_b;
                    }
                }
            }
            if let Op::Cmp(_, _, b) = &mut func.insts[inst_id].op {
                *b = null_vid;
            }
        }
    }

    true
}
