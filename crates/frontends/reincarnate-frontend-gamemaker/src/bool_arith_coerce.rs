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
use reincarnate_core::ir::inst::{CastKind, Inst, InstId, Op};
use reincarnate_core::ir::module::StructDef;
use reincarnate_core::ir::ty::Type;
use reincarnate_core::ir::{Function, Module, ValueId};
use reincarnate_core::pipeline::{Transform, TransformResult};

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

        let mut changed = false;
        for func in module.functions.values_mut() {
            changed |= coerce_bool_arithmetic(func, &bool_returning);
            changed |= coerce_bool_br_args(func);
            changed |= coerce_bool_set_field(func, &struct_field_types);
        }
        Ok(TransformResult { module, changed })
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Returns true if `v` has type `Bool` in `func.value_types`.
fn is_bool(func: &Function, v: ValueId) -> bool {
    matches!(func.value_types.get(v), Some(Type::Bool))
}

/// Returns true if `ty` is an integer or float type (needs coercion from Bool).
fn is_numeric(ty: &Type) -> bool {
    matches!(ty, Type::Int(_) | Type::Float(_))
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
fn needs_arith_coerce(
    func: &Function,
    v: ValueId,
    result_map: &HashMap<ValueId, InstId>,
    bool_returning: &HashSet<String>,
) -> bool {
    if is_bool(func, v) {
        return true;
    }
    // Fix A: value_types[v] was widened by ConstraintSolve, but the callee
    // sig still says Bool — the emitter will emit a boolean-typed expression.
    if let Some(&inst_id) = result_map.get(&v) {
        if let Op::Call {
            func: callee_name, ..
        } = &func.insts[inst_id].op
        {
            return bool_returning.contains(callee_name);
        }
    }
    false
}

fn coerce_bool_arithmetic(func: &mut Function, bool_returning: &HashSet<String>) -> bool {
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
            let a_coerce = needs_arith_coerce(func, a, &result_map, bool_returning);
            let b_coerce = needs_arith_coerce(func, b, &result_map, bool_returning);
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

/// Returns true if a field name is a known GML built-in numeric property.
/// These are fields on GMLObject that are declared as `number` in the
/// TypeScript runtime (object.ts). Excludes fields that accept boolean
/// values like `visible`, `solid`, `persistent`.
fn is_gml_numeric_field(name: &str) -> bool {
    matches!(
        name,
        "x" | "y"
            | "z"
            | "xstart"
            | "ystart"
            | "xprevious"
            | "yprevious"
            | "image_xscale"
            | "image_yscale"
            | "image_index"
            | "image_alpha"
            | "image_speed"
            | "image_angle"
            | "depth"
            | "speed"
            | "direction"
            | "hspeed"
            | "vspeed"
            | "friction"
            | "gravity"
            | "gravity_direction"
    )
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
) -> bool {
    let targets: Vec<(InstId, ValueId)> = func
        .insts
        .iter()
        .filter_map(|(id, inst)| {
            if let Op::SetField { field, value, .. } = &inst.op {
                if is_bool(func, *value) {
                    // Check built-in GML numeric fields.
                    if is_gml_numeric_field(field) {
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
