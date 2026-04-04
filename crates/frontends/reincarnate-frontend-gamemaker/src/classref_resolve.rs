//! GML class-reference resolution IR pass.
//!
//! In GMS1 games, object type indices are pushed as plain integer constants —
//! there is no `pushref` (Break -11) instruction.  So a call like
//! `instance_create(x, y, 2)` generates `Op::Const(Int(2))` rather than a
//! `Op::GlobalRef("Enemy", ClassRef)`.  This pass finds those plain integer
//! (or `Cast(Int(N), Unknown, Coerce)`) arguments at positions declared as
//! `"classref"` in `runtime.json`, and replaces them with a fresh
//! `Op::GlobalRef(name)` instruction whose type is `Type::ClassRef(name)`.
//!
//! Every backend then sees already-resolved class references in the IR,
//! instead of having to reimplement the lookup logic per backend.

use std::collections::{HashMap, HashSet};

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::func::FuncId;
use reincarnate_core::ir::inst::{CastKind, Inst, InstId, Op};
use reincarnate_core::ir::ty::{Type, TypeId};
use reincarnate_core::ir::{Constant, Function, Module, ValueId};
use reincarnate_core::pipeline::{PureIrPass, Transform, TransformResult};

/// GML class-reference resolution pass.
///
/// Replaces integer constants at `"classref"`-typed parameter positions in
/// calls to external functions with `Op::GlobalRef(name)` instructions typed
/// `Type::ClassRef(name)`.
pub struct GmlClassRefResolve;

impl Transform for GmlClassRefResolve {
    fn name(&self) -> &str {
        "gml-classref-resolve"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn apply(
        &self,
        mut module: Module,
        dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        if module.object_names.is_empty() || module.external_function_sigs.is_empty() {
            return Ok(TransformResult {
                module,
                changed: false,
                changed_funcs: HashSet::new(),
            });
        }

        // Build a map: FuncId → list of param indices with "classref" type.
        let classref_params: HashMap<FuncId, Vec<usize>> = module
            .external_function_sigs
            .iter()
            .filter_map(|(name, sig)| {
                let indices: Vec<usize> = sig
                    .params
                    .iter()
                    .enumerate()
                    .filter(|(_, p)| p.as_str() == "classref")
                    .map(|(i, _)| i)
                    .collect();
                if indices.is_empty() {
                    return None;
                }
                let fid = *module.runtime_registry.get(name)?;
                Some((fid, indices))
            })
            .collect();

        if classref_params.is_empty() {
            return Ok(TransformResult {
                module,
                changed: false,
                changed_funcs: HashSet::new(),
            });
        }

        let object_names = module.object_names.clone();

        // Pre-intern all object names as ClassRef TypeIds so resolve_function can
        // create Type::ClassRef(TypeId) values without needing the full module.
        let classref_type_ids: Vec<Option<TypeId>> = object_names
            .iter()
            .map(|name| {
                if let Type::ClassRef(id) = module.intern_type_classref(name) {
                    Some(id)
                } else {
                    None
                }
            })
            .collect();

        let mut changed_funcs: HashSet<FuncId> = HashSet::new();

        for func_id in module.functions.keys().collect::<Vec<_>>() {
            if dirty.is_some_and(|d| !d.contains(&func_id)) {
                continue;
            }
            let func = &mut module.functions[func_id];
            if resolve_function(func, &classref_params, &object_names, &classref_type_ids) {
                changed_funcs.insert(func_id);
            }
        }

        let changed = !changed_funcs.is_empty();
        Ok(TransformResult {
            module,
            changed,
            changed_funcs,
        })
    }
}

impl PureIrPass for GmlClassRefResolve {}

/// Process a single function: find call sites with "classref" params whose
/// argument is a plain integer constant (or a Coerce cast thereof), and
/// replace each such argument with a new `Op::GlobalRef` value.
fn resolve_function(
    func: &mut Function,
    classref_params: &HashMap<FuncId, Vec<usize>>,
    object_names: &[String],
    classref_type_ids: &[Option<TypeId>],
) -> bool {
    // Build helper maps: value → integer constant it holds.
    //   const_ints:      result of Op::Const(Int(N))   → N
    //   cast_const_ints: result of Op::Cast(v, Unknown, Coerce) where v is in const_ints → N
    let const_ints: HashMap<ValueId, i64> = func
        .insts
        .iter()
        .filter_map(|(_, inst)| {
            if let Op::Const(Constant::Int(n)) = &inst.op {
                inst.result.map(|v| (v, *n))
            } else {
                None
            }
        })
        .collect();

    let cast_const_ints: HashMap<ValueId, i64> = func
        .insts
        .iter()
        .filter_map(|(_, inst)| {
            if let Op::Cast(inner, Type::Unknown, CastKind::Coerce) = &inst.op {
                let n = const_ints.get(inner).copied()?;
                inst.result.map(|v| (v, n))
            } else {
                None
            }
        })
        .collect();

    // Collect rewrites: (call_inst_id, arg_index, obj_index_into_object_names).
    // We need the block positions to insert the new GlobalRef instructions.
    let mut rewrites: Vec<(InstId, usize, usize)> = Vec::new();

    for (inst_id, inst) in func.insts.iter() {
        let Op::Call {
            func: callee_fid,
            args,
        } = &inst.op
        else {
            continue;
        };
        let Some(indices) = classref_params.get(callee_fid) else {
            continue;
        };
        for &param_idx in indices {
            let Some(&arg_val) = args.get(param_idx) else {
                continue;
            };
            // Resolve the integer value — plain Const or Coerce-cast Const.
            let n = match const_ints
                .get(&arg_val)
                .copied()
                .or_else(|| cast_const_ints.get(&arg_val).copied())
            {
                Some(n) => n,
                None => continue,
            };
            // Skip negative sentinel values (-1 all, -2 noone, etc.).
            if n < 0 {
                continue;
            }
            let obj_idx = n as usize;
            if obj_idx >= object_names.len() {
                continue;
            }
            rewrites.push((inst_id, param_idx, obj_idx));
        }
    }

    if rewrites.is_empty() {
        return false;
    }

    // For each rewrite, allocate a new ValueId + InstId for GlobalRef, then
    // patch the call's arg list and insert the GlobalRef instruction immediately
    // before the call in its block.
    for (call_inst_id, param_idx, obj_idx) in rewrites {
        let obj_name = object_names[obj_idx].clone();
        let classref_ty = classref_type_ids
            .get(obj_idx)
            .and_then(|opt| *opt)
            .map(Type::ClassRef)
            .unwrap_or(Type::Unknown);
        let new_vid = func.value_types.push(classref_ty);
        let new_inst_id = func.insts.push(Inst {
            op: Op::GlobalRef(obj_name),
            result: Some(new_vid),
            span: None,
        });

        // Patch the call's argument list.
        if let Op::Call { args, .. } = &mut func.insts[call_inst_id].op {
            if let Some(arg) = args.get_mut(param_idx) {
                *arg = new_vid;
            }
        }

        // Insert the GlobalRef instruction immediately before the call in
        // whichever block contains the call.
        for block in func.blocks.values_mut() {
            if let Some(pos) = block.insts.iter().position(|&id| id == call_inst_id) {
                block.insts.insert(pos, new_inst_id);
                break;
            }
        }
    }

    true
}
