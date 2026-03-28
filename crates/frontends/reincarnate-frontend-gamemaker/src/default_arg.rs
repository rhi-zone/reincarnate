//! GML default argument recovery pass.
//!
//! GMS2.3+ compiles optional function parameters into the function body as:
//!
//! ```gml
//! if (argument0 === undefined) argument0 = default_value;
//! ```
//!
//! After IR translation and Mem2Reg, this becomes a chain of blocks:
//!
//! ```text
//! entry_block(..., v_arg: arg):
//!     v_undef = get_field self, "undefined"
//!     v_cmp = cmp.eq v_arg, v_undef
//!     br_if v_cmp, default_block, continue_block(v_arg)
//!
//! default_block:
//!     v_default = const <value>
//!     br continue_block(v_default)
//!
//! continue_block(v_resolved):
//!     ... next check or function body ...
//! ```
//!
//! This pass detects that pattern, extracts constant defaults, and sets
//! `FunctionSig.defaults[param_index] = Some(constant)` so the emitted
//! TypeScript has optional parameters, eliminating TS2554/TS2555 errors.
//!
//! The body check is left in place — it's redundant but harmless; removing it
//! would require block rewriting which isn't worth the complexity.

use std::collections::HashSet;

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::func::FuncId;
use reincarnate_core::ir::inst::CmpKind;
use reincarnate_core::ir::inst::Terminator;
use reincarnate_core::ir::ty::Type;
use reincarnate_core::ir::{BlockId, Constant, Function, Module, Op, ValueId};
use reincarnate_core::pipeline::{PureIrPass, Transform, TransformResult};

pub struct GmlDefaultArgRecovery;

impl Transform for GmlDefaultArgRecovery {
    fn name(&self) -> &str {
        "gml-default-arg-recovery"
    }

    fn apply(
        &self,
        mut module: Module,
        dirty: Option<&HashSet<FuncId>>,
    ) -> Result<TransformResult, CoreError> {
        let mut changed_funcs: HashSet<FuncId> = HashSet::new();
        for func_id in module.functions.keys().collect::<Vec<_>>() {
            if dirty.is_some_and(|d| !d.contains(&func_id)) {
                continue;
            }
            let func = &mut module.functions[func_id];
            let mut func_changed = recover_defaults(func);
            func_changed |= set_variadic_defaults(func);
            if func_changed {
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

impl PureIrPass for GmlDefaultArgRecovery {}

/// Try to recover default argument values from the entry block chain.
fn recover_defaults(func: &mut Function) -> bool {
    // Need at least 2 params (self + one arg) to have default-check patterns.
    if func.sig.params.len() < 2 {
        return false;
    }

    // Skip if any defaults are already set.
    if func.sig.defaults.iter().any(|d| d.is_some()) {
        return false;
    }

    // Collect recovered defaults: (param_index, constant).
    let matches = scan_default_chain(func);
    if matches.is_empty() {
        return false;
    }

    // Ensure defaults vec is long enough.
    while func.sig.defaults.len() < func.sig.params.len() {
        func.sig.defaults.push(None);
    }

    for m in &matches {
        func.sig.defaults[m.param_idx] = Some(m.constant.clone());
        // Also narrow the param type when it's currently Unknown.  TypeInference
        // runs after this pass and will propagate the concrete type into the body.
        if matches!(func.sig.params[m.param_idx], Type::Unknown) {
            let ty = type_of_constant(&m.constant);
            func.sig.params[m.param_idx] = ty.clone();
            // Update the entry block param as well.
            if let Some(bp) = func.blocks[func.entry].params.get_mut(m.param_idx) {
                func.value_types[bp.value] = ty.clone();
                bp.ty = ty;
            }
        }
    }

    // DCE the undefined-check blocks: replace each BrIf with an unconditional
    // Br to the continue block, passing the original param value.  This
    // prevents later passes (e.g. IntToBoolPromotion) from changing the body
    // constant to a different type than the recovered sig default.
    for m in &matches {
        // Replace the BrIf terminator with unconditional Br to continue block.
        if let Terminator::BrIf {
            else_target,
            else_args,
            ..
        } = &func.blocks[m.check_block].terminator
        {
            let target = *else_target;
            let args = else_args.clone();
            func.blocks[m.check_block].terminator = Terminator::Br { target, args };
        }
    }

    true
}

/// Set type-appropriate defaults on all argument params that don't already have one.
///
/// Applies to all functions — variadic and non-variadic alike. In GML, any argument
/// can be omitted from a call; missing args are `undefined` (GMS2.3+) or 0 (GMS1).
/// This ensures every argument param is optional in the emitted TypeScript.
///
/// Runs after `recover_defaults`, so explicitly-defaulted params (with constant
/// defaults recovered from `arg === undefined` checks) keep their specific values;
/// only the remaining params get the type-appropriate zero sentinel.
fn set_variadic_defaults(func: &mut Function) -> bool {
    // Skip functions that have no params beyond self.
    if func.sig.params.len() < 2 {
        return false;
    }

    // Ensure defaults vec covers all params.
    while func.sig.defaults.len() < func.sig.params.len() {
        func.sig.defaults.push(None);
    }

    let mut changed = false;
    // Skip self/other (they're never optional). For variadic functions, also skip
    // the rest param at the end (it's always optional in TypeScript by its nature).
    let end_idx = if func.sig.has_rest_param {
        func.sig.params.len() - 1 // last param is ...args, skip it
    } else {
        func.sig.params.len()
    };
    for i in 0..end_idx {
        if func.sig.defaults[i].is_some() {
            continue;
        }
        // Skip self/other params — they have Struct or Unknown type but aren't arguments.
        // Argument params are named "argument*" or are the positional args after self/other.
        // Heuristic: self is always index 0 (Struct type or Unknown for untyped),
        // but we can't distinguish self from args by type alone.  Use the entry block
        // param names if available.
        let param_value = func.blocks[func.entry].params.get(i).map(|p| p.value);
        if let Some(val) = param_value {
            if let Some(name) = func.value_names.get(&val) {
                if name == "self" || name == "other" {
                    continue;
                }
            }
        }
        // Use the narrowed type from value_types (set by type inference) for scalar types
        // (Bool/Int/UInt/Float/String) so their defaults are type-appropriate (false, 0, "").
        // For non-scalar types (Struct, Array, Unknown, etc.), fall back to the GML missing-arg
        // sentinel (0.0 / Float(0.0)).  A Struct-typed parameter that accepts the 0.0 sentinel
        // is effectively `any` at the call site — using the narrowed type would produce
        // `argument0: GMLObject = 0.0` which TypeScript rejects.
        let narrowed = func.blocks[func.entry]
            .params
            .get(i)
            .map(|p| &func.value_types[p.value])
            .unwrap_or(&func.sig.params[i]);
        let is_scalar = matches!(
            narrowed,
            Type::Bool | Type::Int(_) | Type::UInt(_) | Type::Float(_) | Type::String
        );
        let ty = if is_scalar { narrowed } else { &Type::Unknown };
        func.sig.defaults[i] = Some(zero_for_type(ty));
        // Widen the param's value_type back to Unknown for non-scalar types so the
        // TypeScript annotation (`any = 0.0`) is consistent with the variadic sentinel.
        // The narrowed type (e.g. GMLObject) is correct for callers that DO pass a value;
        // the variadic sentinel 0.0 means the type annotation must accommodate both.
        if !is_scalar {
            if let Some(pv) = param_value {
                func.value_types[pv] = Type::Unknown;
            }
        }
        changed = true;
    }
    changed
}

/// Return the concrete IR type for a constant value.
fn type_of_constant(c: &Constant) -> Type {
    match c {
        Constant::Bool(_) => Type::Bool,
        Constant::Int(_) => Type::Int(64),
        Constant::UInt(_) => Type::UInt(64),
        Constant::Float(_) => Type::Float(64),
        Constant::String(_) => Type::String,
        Constant::Null => Type::Unknown,
    }
}

/// Return the type-appropriate zero/default constant for a given IR type.
fn zero_for_type(ty: &Type) -> Constant {
    match ty {
        Type::Bool => Constant::Bool(false),
        Type::Int(_) => Constant::Int(0),
        Type::UInt(_) => Constant::UInt(0),
        Type::Float(_) => Constant::Float(0.0),
        Type::String => Constant::String(String::new()),
        // For Unknown, Struct, Array, or anything else, use 0.0 — GML's missing-arg value.
        _ => Constant::Float(0.0),
    }
}

/// Info needed to rewrite a matched default-check block.
struct DefaultCheckMatch {
    param_idx: usize,
    constant: Constant,
    /// The block containing the BrIf.
    check_block: BlockId,
    /// The continue block (else_target of the BrIf) — used for debugging.
    #[allow(dead_code)]
    continue_block: BlockId,
}

/// Walk the entry block chain looking for the `if (arg === undefined) arg = default` pattern.
/// Returns match info for each recovered default.
fn scan_default_chain(func: &Function) -> Vec<DefaultCheckMatch> {
    let mut results = Vec::new();
    let mut current_block = func.entry;

    // Collect entry block param values for identifying which param is being checked.
    let entry_params: Vec<ValueId> = func.blocks[func.entry]
        .params
        .iter()
        .map(|p| p.value)
        .collect();

    // We need at least one param (self) to do GetField on.
    if entry_params.is_empty() {
        return results;
    }

    let self_param = entry_params[0];

    while let Some((param_idx, constant, next_block)) =
        try_match_default_check(func, current_block, self_param, &entry_params)
    {
        results.push(DefaultCheckMatch {
            param_idx,
            constant,
            check_block: current_block,
            continue_block: next_block,
        });
        current_block = next_block;
    }

    results
}

/// Try to match the default-check pattern in a single block.
///
/// Pattern:
///   v_undef = get_field self, "undefined"
///   v_cmp = cmp.eq param[N], v_undef
///   br_if v_cmp, then_block, else_block(param[N])
///
/// then_block:
///   v_default = const <value>
///   br else_block(v_default)
///
/// Returns (param_index, default_constant, else_block_id) on success.
fn try_match_default_check(
    func: &Function,
    block_id: BlockId,
    self_param: ValueId,
    entry_params: &[ValueId],
) -> Option<(usize, Constant, BlockId)> {
    let block = &func.blocks[block_id];
    let insts = &block.insts;

    // We need at least 2-3 instructions: possibly GetField, Cmp, BrIf.
    // The GetField for "undefined" might be in this block or a previous one,
    // so we need to find a Cmp that compares a param with an undefined value.

    // Find any value that is `get_field self, "undefined"` in this block.
    let mut undefined_val = None;
    for &inst_id in insts {
        let inst = &func.insts[inst_id];
        if let Op::GetField { object, field } = &inst.op {
            if *object == self_param && field == "undefined" {
                undefined_val = inst.result;
                break;
            }
        }
    }
    let undefined_val = undefined_val?;

    // Find a Cmp.Eq comparing an entry param with the undefined value.
    let mut cmp_val = None;
    let mut checked_param_idx = None;
    for &inst_id in insts {
        let inst = &func.insts[inst_id];
        if let Op::Cmp(CmpKind::Eq, lhs, rhs) = &inst.op {
            // Check if one side is an entry param and the other is the undefined sentinel.
            if *rhs == undefined_val {
                if let Some(idx) = entry_params.iter().position(|&p| p == *lhs) {
                    cmp_val = inst.result;
                    checked_param_idx = Some(idx);
                    break;
                }
            }
            if *lhs == undefined_val {
                if let Some(idx) = entry_params.iter().position(|&p| p == *rhs) {
                    cmp_val = inst.result;
                    checked_param_idx = Some(idx);
                    break;
                }
            }
        }
    }
    let cmp_val = cmp_val?;
    let checked_param_idx = checked_param_idx?;

    // Check the block's BrIf terminator using this comparison result.
    if let Terminator::BrIf {
        cond,
        then_target,
        then_args,
        else_target,
        else_args,
    } = &func.blocks[block_id].terminator
    {
        if *cond != cmp_val {
            return None;
        }

        // The `then_target` is the default-assignment block (condition is true = arg is undefined).
        // The `else_target` is the continuation block (arg already has a value).
        let default_block = *then_target;
        let continue_block = *else_target;

        // Verify: then_args should be empty (default block doesn't take params from here).
        if !then_args.is_empty() {
            return None;
        }

        // Check the default block: should have a Const + Br to continue_block.
        let default_blk = &func.blocks[default_block];

        // Find the constant in the default block's instructions.
        let mut found_const = None;
        for &dinst_id in &default_blk.insts {
            let dinst = &func.insts[dinst_id];
            if let Op::Const(c) = &dinst.op {
                found_const = Some(c.clone());
            }
        }

        // Check the default block's Br terminator.
        let found_br = if let Terminator::Br { target, args } = &default_blk.terminator {
            *target == continue_block && args.len() == else_args.len()
        } else {
            false
        };

        if let (Some(constant), true) = (found_const, found_br) {
            return Some((checked_param_idx, constant, continue_block));
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use reincarnate_core::ir::builder::FunctionBuilder;
    use reincarnate_core::ir::ty::{FunctionSig, Type};
    use reincarnate_core::ir::ModuleBuilder;
    use reincarnate_core::ir::Visibility;

    fn emit_constant(fb: &mut FunctionBuilder, c: &Constant) -> ValueId {
        match c {
            Constant::Null => fb.const_null(),
            Constant::Bool(b) => fb.const_bool(*b),
            Constant::Int(n) => fb.const_int(*n, 64),
            Constant::UInt(n) => fb.const_uint(*n),
            Constant::Float(f) => fb.const_float(*f),
            Constant::String(s) => fb.const_string(s.as_str()),
        }
    }

    /// Build a function with the GML default-argument pattern:
    ///   if (arg === self.undefined) arg = default;
    fn build_test_function(defaults: &[Constant]) -> Module {
        // sig: (self, arg0, arg1, ...)
        let n_args = defaults.len();
        let mut params = vec![Type::Unknown]; // self
        let mut sig_defaults = vec![None]; // self has no default
        for _ in 0..n_args {
            params.push(Type::Unknown);
            sig_defaults.push(None);
        }
        let sig = FunctionSig {
            params,
            defaults: sig_defaults,
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("test_func", sig, Visibility::Public);

        let self_param = fb.param(0);
        fb.name_value(self_param, "self".to_string());

        // For each argument with a default, build the check chain.
        // Start from entry block.
        for (i, default_val) in defaults.iter().enumerate() {
            let arg_param = fb.param(1 + i);

            // get_field self, "undefined"
            let undef = fb.get_field(self_param, "undefined", Type::Unknown);

            // cmp.eq arg, undef
            let cmp = fb.cmp(CmpKind::Eq, arg_param, undef);

            // Create default block and continue block
            let default_block = fb.create_block();
            let (continue_block, continue_vals) = fb.create_block_with_params(&[Type::Unknown]);

            // br_if cmp, default_block, continue_block(arg_param)
            fb.br_if(cmp, default_block, &[], continue_block, &[arg_param]);

            // default_block: const default_val; br continue_block(const)
            fb.switch_to_block(default_block);
            let const_val = emit_constant(&mut fb, default_val);
            fb.br(continue_block, &[const_val]);

            // continue_block: continue building the chain
            fb.switch_to_block(continue_block);
            // continue_vals[0] is the resolved value — would be used by later code
            let _ = continue_vals;
        }

        // Final: return
        fb.ret(None);

        let func = fb.build();
        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        mb.build()
    }

    #[test]
    fn test_single_default() {
        let module = build_test_function(&[Constant::Float(0.0)]);
        let pass = GmlDefaultArgRecovery;
        let result = pass.apply(module, None).unwrap();
        assert!(result.changed);

        let func = &result.module.functions.values().next().unwrap();
        assert_eq!(func.sig.defaults.len(), 2); // self + arg0
        assert_eq!(func.sig.defaults[0], None); // self
        assert_eq!(func.sig.defaults[1], Some(Constant::Float(0.0))); // arg0
                                                                      // Param type should be narrowed from Unknown to Float(64).
        assert_eq!(func.sig.params[1], Type::Float(64));
    }

    #[test]
    fn test_multiple_defaults() {
        let module = build_test_function(&[
            Constant::String("???".to_string()),
            Constant::Float(1.0),
            Constant::Bool(false),
        ]);
        let pass = GmlDefaultArgRecovery;
        let result = pass.apply(module, None).unwrap();
        assert!(result.changed);

        let func = &result.module.functions.values().next().unwrap();
        assert_eq!(func.sig.defaults.len(), 4); // self + 3 args
        assert_eq!(func.sig.defaults[0], None);
        assert_eq!(
            func.sig.defaults[1],
            Some(Constant::String("???".to_string()))
        );
        assert_eq!(func.sig.defaults[2], Some(Constant::Float(1.0)));
        assert_eq!(func.sig.defaults[3], Some(Constant::Bool(false)));
    }

    #[test]
    fn test_no_pattern_defaults_filled() {
        // Function with no explicit `=== undefined` pattern.
        // set_variadic_defaults still fills arg params with zero defaults.
        let sig = FunctionSig {
            params: vec![Type::Unknown, Type::Unknown],
            defaults: vec![None, None],
            return_ty: Type::Unknown,
            ..Default::default()
        };
        let mut fb = FunctionBuilder::new("plain_func", sig, Visibility::Public);
        let self_param = fb.param(0);
        fb.name_value(self_param, "self".to_string());
        fb.ret(None);
        let func = fb.build();
        let mut mb = ModuleBuilder::new("test");
        mb.add_function(func);
        let module = mb.build();

        let pass = GmlDefaultArgRecovery;
        let result = pass.apply(module, None).unwrap();
        // Even without an explicit pattern, set_variadic_defaults fills arg1 with 0.0.
        assert!(result.changed);
        let func = result.module.functions.values().next().unwrap();
        assert_eq!(func.sig.defaults[0], None); // self — never gets a default
        assert_eq!(func.sig.defaults[1], Some(Constant::Float(0.0))); // arg0 = 0.0
    }
}
