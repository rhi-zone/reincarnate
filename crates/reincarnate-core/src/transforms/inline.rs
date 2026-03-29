//! `Inline` transform pass.
//!
//! Inlines functions marked with [`InlineHint::Always`] at their call sites.
//!
//! # Motivation
//!
//! The GML frontend registers runtime stubs (`string_length`, `dsin`, etc.)
//! and attaches IR bodies to them via `register_runtime_bodies`. Without
//! inlining, every call site emits as `string_length(s)` — a call to the
//! handwritten runtime — rather than the direct expression the body encodes
//! (e.g. `s.length`).
//!
//! # Algorithm
//!
//! For each function (caller) in the module:
//!   For each `Op::Call { func, args }` where the callee:
//!     - is registered in `module.runtime_registry`
//!     - has `inline_hint == InlineHint::Always`
//!     - has exactly one block
//!     - has a `Terminator::Return(Some(v))` (single-block, single-return)
//!   Inline the body:
//!     1. Build a value map: callee param values → caller argument values.
//!     2. Copy each instruction from the callee's block into the caller,
//!        allocating new `ValueId`s and remapping all value references.
//!     3. Substitute all uses of the old call result in subsequent caller
//!        instructions with the remapped callee return value.
//!     4. Remove the `Op::Call` instruction from the caller's block.
//!
//! The pass runs in a fixed-point loop until no more inlining occurs (handles
//! chains where inlined bodies themselves contain inlinable calls).
//!
//! Multi-block callees are skipped — they are logged to stderr and not inlined.
//!
//! # Placement
//!
//! Runs after `builtin-overload-select` (so typed builtins are in place) and
//! before `dead-code-elimination` (so inlined dead code gets eliminated).

use std::collections::{HashMap, HashSet};

use crate::error::CoreError;
use crate::ir::func::{FuncId, InlineHint};
use crate::ir::inst::{Inst, Terminator};
use crate::ir::{Function, InstId, Module, Op, ValueId};
use crate::pipeline::{Transform, TransformResult};

use super::util::substitute_values_in_op;

/// Inline transform — substitutes `InlineHint::Always` callee bodies at call sites.
pub struct Inline;

/// A call site eligible for inlining.
struct InlineSite {
    /// Index into `caller.blocks.keys()` (positional).
    block_idx: usize,
    /// Position of the call instruction within the block's `insts` vec.
    inst_pos: usize,
    /// The function name being called.
    fname: String,
    /// Argument values passed to the call.
    args: Vec<ValueId>,
    /// The `ValueId` produced by the call instruction, if any.
    result_vid: Option<ValueId>,
}

/// Attempt to inline all eligible calls in a single function.
///
/// Returns `true` if at least one call was inlined.
fn inline_function(caller: &mut Function, module: &Module) -> bool {
    // Collect eligible call sites.
    // We need block_idx + inst_pos so we can splice instructions in place.
    let mut sites: Vec<InlineSite> = vec![];

    for (block_idx, block) in caller.blocks.values().enumerate() {
        for (inst_pos, &inst_id) in block.insts.iter().enumerate() {
            let inst = &caller.insts[inst_id];
            if let Op::Call { func: fname, args } = &inst.op {
                let Some(&callee_fid) = module.runtime_registry.get(fname.as_str()) else {
                    continue;
                };
                let callee = &module.functions[callee_fid];
                if callee.inline_hint != InlineHint::Always {
                    continue;
                }
                if callee.blocks.len() != 1 {
                    continue;
                }
                let callee_block = callee.blocks.values().next().unwrap();
                // Only inline single-block callees with a simple Return(Some(v)) terminator.
                if !matches!(callee_block.terminator, Terminator::Return(Some(_))) {
                    continue;
                }
                // Skip inlining if any argument type doesn't exactly match the
                // callee's parameter type.  Mismatched types (e.g. Unknown where
                // String is expected) cause existing single-error call-site
                // diagnostics to multiply into per-operation errors inside the
                // inlined body — a net regression.
                let types_match = args
                    .iter()
                    .zip(callee.sig.params.iter())
                    .all(|(&arg_vid, param_ty)| &caller.value_types[arg_vid] == param_ty);
                if !types_match {
                    continue;
                }
                sites.push(InlineSite {
                    block_idx,
                    inst_pos,
                    fname: fname.clone(),
                    args: args.clone(),
                    result_vid: inst.result,
                });
            }
        }
    }

    if sites.is_empty() {
        return false;
    }

    // Process sites in reverse order within each block so that earlier
    // positions stay valid as we splice/remove instructions.
    // Sort by (block_idx desc, inst_pos desc).
    sites.sort_by(|a, b| {
        b.block_idx
            .cmp(&a.block_idx)
            .then(b.inst_pos.cmp(&a.inst_pos))
    });

    let block_ids: Vec<_> = caller.blocks.keys().collect();

    for site in sites {
        let block_id = block_ids[site.block_idx];

        let Some(&callee_fid) = module.runtime_registry.get(site.fname.as_str()) else {
            continue;
        };
        let callee = &module.functions[callee_fid];

        // Re-check eligibility (another iteration may have changed things).
        if callee.inline_hint != InlineHint::Always || callee.blocks.len() != 1 {
            continue;
        }
        let callee_entry = callee.entry;
        let callee_block = &callee.blocks[callee_entry];
        let Terminator::Return(Some(callee_ret_val)) = callee_block.terminator else {
            continue;
        };

        // Build value map: callee param values → caller argument values.
        let mut value_map: HashMap<ValueId, ValueId> = HashMap::new();
        for (param, &arg) in callee_block.params.iter().zip(site.args.iter()) {
            value_map.insert(param.value, arg);
        }

        // Copy callee instructions into the caller, remapping values.
        // Collect the new instruction ids in order.
        let mut new_inst_ids: Vec<InstId> = Vec::new();
        for &callee_inst_id in &callee_block.insts {
            let callee_inst = &callee.insts[callee_inst_id];
            // Clone the op and remap value references.
            let mut new_op = callee_inst.op.clone();
            substitute_values_in_op(&mut new_op, &value_map);

            // Allocate a new ValueId in the caller for the result (if any).
            let new_result = callee_inst.result.map(|callee_vid| {
                let ty = callee.value_types[callee_vid].clone();
                let new_vid = caller.value_types.push(ty);
                value_map.insert(callee_vid, new_vid);
                new_vid
            });

            let new_inst_id = caller.insts.push(Inst {
                op: new_op,
                result: new_result,
                span: callee_inst.span.clone(),
            });
            new_inst_ids.push(new_inst_id);
        }

        // Determine the caller-side value that maps to the callee's return value.
        let inlined_return_vid: ValueId = *value_map
            .get(&callee_ret_val)
            .expect("callee return value must be in value_map");

        // Remove the call instruction from the block and splice in the new instructions.
        let block_insts = &mut caller.blocks[block_id].insts;
        block_insts.remove(site.inst_pos);
        for (offset, new_id) in new_inst_ids.into_iter().enumerate() {
            block_insts.insert(site.inst_pos + offset, new_id);
        }

        // Substitute all uses of the old call result in the caller.
        // This covers all instructions in all blocks and all terminators.
        if let Some(old_result) = site.result_vid {
            let subst: HashMap<ValueId, ValueId> =
                [(old_result, inlined_return_vid)].into_iter().collect();

            // Walk all live instructions in the caller and substitute.
            let all_inst_ids: Vec<InstId> = caller
                .blocks
                .values()
                .flat_map(|b| b.insts.iter().copied())
                .collect();
            for inst_id in all_inst_ids {
                substitute_values_in_op(&mut caller.insts[inst_id].op, &subst);
            }

            // Also substitute in terminators.
            let all_block_ids: Vec<_> = caller.blocks.keys().collect();
            for bid in all_block_ids {
                substitute_values_in_terminator(&mut caller.blocks[bid].terminator, &subst);
            }
        }
    }

    true
}

/// Substitute ValueIds in a Terminator using a substitution map.
fn substitute_values_in_terminator(term: &mut Terminator, subst: &HashMap<ValueId, ValueId>) {
    let sub = |v: &mut ValueId| {
        if let Some(&new) = subst.get(v) {
            *v = new;
        }
    };
    match term {
        Terminator::Br { args, .. } => {
            for a in args {
                sub(a);
            }
        }
        Terminator::BrIf {
            cond,
            then_args,
            else_args,
            ..
        } => {
            sub(cond);
            for a in then_args {
                sub(a);
            }
            for a in else_args {
                sub(a);
            }
        }
        Terminator::Switch {
            value,
            cases,
            default,
        } => {
            sub(value);
            for (_, _, args) in cases {
                for a in args {
                    sub(a);
                }
            }
            for a in &mut default.1 {
                sub(a);
            }
        }
        Terminator::Return(Some(v)) => sub(v),
        Terminator::Return(None) => {}
    }
}

impl Transform for Inline {
    fn name(&self) -> &str {
        "inline"
    }

    fn run_once(&self) -> bool {
        true
    }

    fn requires(&self) -> &[&str] {
        &["builtin-overload-select"]
    }

    fn apply(
        &self,
        mut module: Module,
        _dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        let mut any_changed = false;

        // Fixed-point: repeat until no inlining occurs (handles chains).
        loop {
            let mut changed_this_round = false;
            // Snapshot the set of non-runtime function ids.
            let user_func_ids: Vec<FuncId> = module
                .functions
                .keys()
                .filter(|&fid| !module.runtime_registry.values().any(|&rid| rid == fid))
                .collect();

            for func_id in user_func_ids {
                let mut func = module.functions[func_id].clone();
                if inline_function(&mut func, &module) {
                    module.functions[func_id] = func;
                    changed_this_round = true;
                }
            }
            if !changed_this_round {
                break;
            }
            any_changed = true;
        }

        Ok(TransformResult {
            module,
            changed: any_changed,
            changed_funcs: HashSet::new(), // module-level change; don't track per-func
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ir::builder::{FunctionBuilder, ModuleBuilder};
    use crate::ir::func::InlineHint;
    use crate::ir::ty::FunctionSig;
    use crate::ir::{Module, Type, Visibility};

    /// Build a test module with:
    ///   - A 1-param callee that calls `builtin.abs_f64` on its param and returns
    ///   - A caller that calls the callee
    ///   - The callee marked `InlineHint::Always`
    fn build_test_module() -> (Module, FuncId, FuncId) {
        let mut mb = ModuleBuilder::new("test");
        // Module::new (called by ModuleBuilder::new) already invokes
        // register_core_builtins — no extra call needed.

        // Build callee: fn my_abs(x: f64) -> f64 { return builtin.abs_f64(x) }
        let callee_sig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("my_abs", callee_sig, Visibility::Public);
        let x = fb.param(0);
        let abs_result = fb.call("builtin.abs_f64", &[x], Type::Float(64));
        fb.ret(Some(abs_result));
        let callee_func = fb.build();

        // Build caller: fn caller(v: f64) -> f64 { return my_abs(v) }
        let caller_sig = FunctionSig {
            params: vec![Type::Float(64)],
            return_ty: Type::Float(64),
            ..Default::default()
        };
        let mut fb2 = FunctionBuilder::new("caller", caller_sig, Visibility::Public);
        let v = fb2.param(0);
        let call_result = fb2.call("my_abs", &[v], Type::Float(64));
        fb2.ret(Some(call_result));
        let caller_func = fb2.build();

        let callee_id = mb.add_function(callee_func);
        let caller_id = mb.add_function(caller_func);

        let mut module = mb.build();

        // Register "my_abs" in the runtime registry so the inliner can find it.
        module
            .runtime_registry
            .insert("my_abs".to_string(), callee_id);

        // Mark callee as Always inline.
        module.functions[callee_id].inline_hint = InlineHint::Always;

        (module, callee_id, caller_id)
    }

    /// After inlining, the caller should no longer contain an `Op::Call { func: "my_abs", .. }`.
    #[test]
    fn call_is_inlined() {
        let (module, _callee_id, caller_id) = build_test_module();

        let result = Inline.apply(module, None).unwrap();
        assert!(result.changed, "inliner should have reported a change");

        let caller = &result.module.functions[caller_id];
        let has_my_abs_call =
            caller.blocks.values().flat_map(|b| b.insts.iter()).any(
                |&iid| matches!(&caller.insts[iid].op, Op::Call { func, .. } if func == "my_abs"),
            );

        assert!(
            !has_my_abs_call,
            "Op::Call to 'my_abs' should have been inlined away"
        );
    }

    /// The inlined call's result must still feed the return terminator.
    #[test]
    fn inlined_result_feeds_return() {
        let (module, _callee_id, caller_id) = build_test_module();

        let result = Inline.apply(module, None).unwrap();
        let caller = &result.module.functions[caller_id];

        // The entry block's terminator should be Return(Some(v)) where v is produced
        // by a builtin.abs_f64 call (the inlined body).
        let entry = caller.entry;
        let Terminator::Return(Some(ret_val)) = caller.blocks[entry].terminator else {
            panic!("expected Return(Some(..)) terminator");
        };

        // Find the instruction that produces ret_val.
        let producing_inst = caller
            .blocks
            .values()
            .flat_map(|b| b.insts.iter())
            .find(|&&iid| caller.insts[iid].result == Some(ret_val));

        let producing_inst =
            producing_inst.expect("return value must have a producing instruction");
        assert!(
            matches!(
                &caller.insts[*producing_inst].op,
                Op::Call { func, .. } if func == "builtin.abs_f64"
            ),
            "return value should be produced by builtin.abs_f64"
        );
    }
}
