use std::collections::HashMap;

use reincarnate_core::ir::builder::FunctionBuilder;
use reincarnate_core::ir::func::{FuncId, Function, Visibility};
use reincarnate_core::ir::module::Module;
use reincarnate_core::ir::ty::{FunctionSig, Type};

/// Register polymorphic `_any` arithmetic builtins and their specialization tables.
///
/// These stubs are used by the GML frontend when an arithmetic operand type is not
/// yet known at translation time.  `BuiltinOverloadSelect` replaces each `xxx_any`
/// call with the appropriately-typed variant (`_f64`, `_f32`, `_i32`, `_i64`) once
/// HM inference has resolved the operand types.
///
/// The typed variants (`builtin.add_f64`, etc.) must already be present in
/// `module.runtime_registry` — i.e., this must be called after `Module::new()`.
///
/// This is a GML-specific concern: the GML VM uses `DataType::Variable` for
/// arithmetic whose operand types are not statically tagged.  Other frontends do
/// not need these stubs.
pub fn register_arithmetic_any_builtins(module: &mut Module) {
    let scalar_types = [
        Type::Float(64),
        Type::Float(32),
        Type::Int(32),
        Type::Int(64),
    ];

    let bin_any = FunctionSig {
        params: vec![Type::Unknown, Type::Unknown],
        return_ty: Type::Unknown,
        ..Default::default()
    };
    let un_any = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Unknown,
        ..Default::default()
    };

    for op in &["add", "sub", "mul", "div", "rem"] {
        let mut specs: HashMap<Vec<Type>, FuncId> = scalar_types
            .iter()
            .map(|ty| {
                let suffix = type_suffix(ty);
                let fid = module.runtime_registry[&format!("builtin.{op}_{suffix}")];
                (vec![ty.clone(), ty.clone()], fid)
            })
            .collect();
        if *op == "add" {
            let concat_id = module.runtime_registry["builtin.concat_str"];
            specs.insert(vec![Type::String, Type::String], concat_id);
        }
        let func_name = format!("{op}_any");
        let any_id = module.register_runtime(&func_name, bin_any.clone());

        // Build dispatch chain for binary op: check each specialization type pair.
        // Priority: Float(64) > Float(32) > Int(32) > Int(64), then String for add.
        let mut dispatch_types: Vec<Type> = scalar_types.to_vec();
        if *op == "add" {
            // Insert String after Float(64) — numeric addition is more common.
            dispatch_types.insert(1, Type::String);
        }
        let built = build_binary_any_dispatch(&func_name, &bin_any, &dispatch_types, op);
        module.functions[any_id].blocks = built.blocks;
        module.functions[any_id].insts = built.insts;
        module.functions[any_id].value_types = built.value_types;
        module.functions[any_id].entry = built.entry;
        module.functions[any_id].specializations = specs;
    }

    {
        let specs: HashMap<Vec<Type>, FuncId> = scalar_types
            .iter()
            .map(|ty| {
                let suffix = type_suffix(ty);
                let fid = module.runtime_registry[&format!("builtin.neg_{suffix}")];
                (vec![ty.clone()], fid)
            })
            .collect();
        let func_name = "neg_any";
        let any_id = module.register_runtime(func_name, un_any.clone());

        // Build dispatch chain for unary neg: check each specialization type.
        let dispatch_types: Vec<Type> = scalar_types.to_vec();
        let built = build_unary_any_dispatch(func_name, &un_any, &dispatch_types);
        module.functions[any_id].blocks = built.blocks;
        module.functions[any_id].insts = built.insts;
        module.functions[any_id].value_types = built.value_types;
        module.functions[any_id].entry = built.entry;
        module.functions[any_id].specializations = specs;
    }
}

/// Return the short suffix string for a given type, used in builtin names like
/// `"builtin.add_f64"`.
fn type_suffix(ty: &Type) -> &'static str {
    match ty {
        Type::Float(64) => "f64",
        Type::Float(32) => "f32",
        Type::Int(32) => "i32",
        Type::Int(64) => "i64",
        Type::String => "str",
        other => panic!("type_suffix: unsupported type {other:?}"),
    }
}

/// Build an IR dispatch body for a binary `_any` builtin (e.g. `add_any(a, b)`).
///
/// Produces a chain of nested `br_if` blocks that check `TypeCheck(a, ty)`,
/// then `TypeCheck(b, ty)`, coerces both arguments, calls the typed variant,
/// and returns the result.  Falls through to the next type on mismatch.
/// The final fallback returns `Return(None)`.
fn build_binary_any_dispatch(
    func_name: &str,
    sig: &FunctionSig,
    dispatch_types: &[Type],
    op: &str,
) -> Function {
    let mut fb = FunctionBuilder::new(func_name, sig.clone(), Visibility::Public);
    let a = fb.param(0);
    let b = fb.param(1);

    // For each dispatch type, build: check_a -> check_b -> call -> return
    // On failure, fall through to the next type's check_a block.
    let fallback_block = fb.create_block();

    let mut next_else_block = fallback_block;

    // Build in reverse so we can set `next_else_block` correctly.
    for ty in dispatch_types.iter().rev() {
        let suffix = type_suffix(ty);
        let variant_name = if *ty == Type::String {
            "builtin.concat_str".to_string()
        } else {
            format!("builtin.{op}_{suffix}")
        };

        // Create blocks for this type's dispatch.
        let check_b_block = fb.create_block();
        let call_block = fb.create_block();
        let check_a_block = fb.create_block();

        // check_a_block: TypeCheck(a, ty) -> br_if to check_b or next
        fb.switch_to_block(check_a_block);
        let check_a = fb.type_check(a, ty.clone());
        fb.br_if(check_a, check_b_block, &[], next_else_block, &[]);

        // check_b_block: TypeCheck(b, ty) -> br_if to call or next
        fb.switch_to_block(check_b_block);
        let check_b = fb.type_check(b, ty.clone());
        fb.br_if(check_b, call_block, &[], next_else_block, &[]);

        // call_block: coerce, call, return
        fb.switch_to_block(call_block);
        let a_coerced = fb.coerce(a, ty.clone());
        let b_coerced = fb.coerce(b, ty.clone());
        let result = fb.call(&variant_name, &[a_coerced, b_coerced], ty.clone());
        fb.ret(Some(result));

        next_else_block = check_a_block;
    }

    // Entry block: branch to the first type check.
    let entry = fb.entry_block();
    fb.switch_to_block(entry);
    fb.br(next_else_block, &[]);

    // Fallback block: return None (unreachable in practice).
    fb.switch_to_block(fallback_block);
    fb.ret(None);

    fb.build()
}

/// Build an IR dispatch body for a unary `_any` builtin (e.g. `neg_any(a)`).
///
/// Same structure as [`build_binary_any_dispatch`] but only checks one argument.
fn build_unary_any_dispatch(
    func_name: &str,
    sig: &FunctionSig,
    dispatch_types: &[Type],
) -> Function {
    let mut fb = FunctionBuilder::new(func_name, sig.clone(), Visibility::Public);
    let a = fb.param(0);

    let fallback_block = fb.create_block();
    let mut next_else_block = fallback_block;

    // Build in reverse.
    for ty in dispatch_types.iter().rev() {
        let suffix = type_suffix(ty);
        let variant_name = format!("builtin.neg_{suffix}");

        let call_block = fb.create_block();
        let check_block = fb.create_block();

        // check_block: TypeCheck(a, ty) -> br_if to call or next
        fb.switch_to_block(check_block);
        let check = fb.type_check(a, ty.clone());
        fb.br_if(check, call_block, &[], next_else_block, &[]);

        // call_block: coerce, call, return
        fb.switch_to_block(call_block);
        let a_coerced = fb.coerce(a, ty.clone());
        let result = fb.call(&variant_name, &[a_coerced], ty.clone());
        fb.ret(Some(result));

        next_else_block = check_block;
    }

    // Entry block: branch to first type check.
    let entry = fb.entry_block();
    fb.switch_to_block(entry);
    fb.br(next_else_block, &[]);

    // Fallback block: return None.
    fb.switch_to_block(fallback_block);
    fb.ret(None);

    fb.build()
}
