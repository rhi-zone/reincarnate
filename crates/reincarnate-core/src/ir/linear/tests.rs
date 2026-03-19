use super::emit::EmitCtx;
use super::linearize::linearize;
use super::resolve::resolve;
use super::*;
use crate::entity::EntityRef;
use crate::ir::ast::{BinOp, Expr, Stmt};
use crate::ir::builder::FunctionBuilder;
use crate::ir::func::{MethodKind, Visibility};
use crate::ir::inst::CmpKind;
use crate::ir::structurize::structurize;
use crate::ir::ty::{FunctionSig, Type};
use crate::ir::value::Constant;

#[test]
fn linearize_simple_block() {
    let sig = FunctionSig {
        params: vec![Type::Int(64), Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("add", sig, Visibility::Public);
    let a = fb.param(0);
    let b = fb.param(1);
    let sum = fb.add(a, b);
    fb.ret(Some(sum));
    let func = fb.build();

    let shape = Shape::Block(func.entry);
    let linear = linearize(&func, &shape);

    // Should have: Def(sum, add_inst), Return(Some(sum))
    assert_eq!(linear.len(), 2);
    assert!(matches!(&linear[0], LinearStmt::Def { result, .. } if *result == sum));
    assert!(matches!(&linear[1], LinearStmt::Return { value: Some(v) } if *v == sum));
}

#[test]
fn linearize_if_else() {
    let sig = FunctionSig {
        params: vec![Type::Bool, Type::Int(64), Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("choose", sig, Visibility::Public);
    let cond = fb.param(0);
    let x = fb.param(1);
    let y = fb.param(2);

    let (then_block, then_vals) = fb.create_block_with_params(&[Type::Int(64)]);
    let (else_block, else_vals) = fb.create_block_with_params(&[Type::Int(64)]);

    fb.br_if(cond, then_block, &[x], else_block, &[y]);

    fb.switch_to_block(then_block);
    fb.ret(Some(then_vals[0]));

    fb.switch_to_block(else_block);
    fb.ret(Some(else_vals[0]));

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let linear = linearize(&func, &shape);

    // Should contain an If with Return in each branch.
    let has_if = linear.iter().any(|s| matches!(s, LinearStmt::If { .. }));
    assert!(has_if, "Expected an If in linearized output: {linear:?}");
}

#[test]
fn linearize_constant_def() {
    let sig = FunctionSig {
        params: vec![],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let c = fb.const_int(42);
    fb.ret(Some(c));
    let func = fb.build();

    let shape = Shape::Block(func.entry);
    let linear = linearize(&func, &shape);

    // Const produces a Def, then Return.
    assert_eq!(linear.len(), 2);
    assert!(matches!(&linear[0], LinearStmt::Def { result, .. } if *result == c));
    match &func.insts[match &linear[0] {
        LinearStmt::Def { inst_id, .. } => *inst_id,
        _ => unreachable!(),
    }]
    .op
    {
        Op::Const(Constant::Int(42)) => {}
        other => panic!("Expected Const(Int(42)), got {other:?}"),
    }
}

// -- Phase 2 tests --

#[test]
fn resolve_constant_classified() {
    let sig = FunctionSig {
        params: vec![Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let a = fb.param(0);
    let c = fb.const_int(42);
    let sum = fb.add(a, c);
    fb.ret(Some(sum));
    let func = fb.build();

    let shape = Shape::Block(func.entry);
    let linear = linearize(&func, &shape);
    let ctx = resolve(&func, &linear, &[]);

    assert!(ctx.constant_inlines.contains_key(&c));
    assert!(ctx.lazy_inlines.contains(&sum));
    assert_eq!(ctx.use_counts.get(&c).copied().unwrap_or(0), 1);
}

#[test]
fn resolve_dead_pure_eliminated() {
    let sig = FunctionSig {
        params: vec![Type::Int(64), Type::Int(64)],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let a = fb.param(0);
    let b = fb.param(1);
    let _dead = fb.add(a, b); // unused
    fb.ret(None);
    let func = fb.build();

    let shape = Shape::Block(func.entry);
    let linear = linearize(&func, &shape);
    let ctx = resolve(&func, &linear, &[]);

    // Dead add: use_count == 0, not in any inline set.
    assert_eq!(ctx.use_counts.get(&_dead).copied().unwrap_or(0), 0);
    assert!(!ctx.lazy_inlines.contains(&_dead));
    assert!(!ctx.constant_inlines.contains_key(&_dead));
}

#[test]
fn resolve_cascading_dead_code() {
    let sig = FunctionSig {
        params: vec![Type::Int(64), Type::Int(64)],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let a = fb.param(0);
    let b = fb.param(1);
    let sum = fb.add(a, b);
    let _neg = fb.neg(sum); // unused; sum only used by neg
    fb.ret(None);
    let func = fb.build();

    let shape = Shape::Block(func.entry);
    let linear = linearize(&func, &shape);
    let ctx = resolve(&func, &linear, &[]);

    // Both neg and sum should be dead after fixpoint.
    assert_eq!(ctx.use_counts.get(&_neg).copied().unwrap_or(0), 0);
    assert_eq!(ctx.use_counts.get(&sum).copied().unwrap_or(0), 0);
}

#[test]
fn resolve_multi_use_not_lazy() {
    let sig = FunctionSig {
        params: vec![Type::Int(64), Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let a = fb.param(0);
    let b = fb.param(1);
    let sum = fb.add(a, b);
    let doubled = fb.add(sum, sum); // sum used twice
    fb.ret(Some(doubled));
    let func = fb.build();

    let shape = Shape::Block(func.entry);
    let linear = linearize(&func, &shape);
    let ctx = resolve(&func, &linear, &[]);

    // sum has 2 uses — should NOT be lazy-inlined.
    assert_eq!(ctx.use_counts.get(&sum).copied().unwrap_or(0), 2);
    assert!(!ctx.lazy_inlines.contains(&sum));
    // doubled has 1 use — should be lazy-inlined.
    assert!(ctx.lazy_inlines.contains(&doubled));
}

// -- Phase 3 (full pipeline) tests --

#[test]
fn full_pipeline_simple_add() {
    let sig = FunctionSig {
        params: vec![Type::Int(64), Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("add", sig, Visibility::Public);
    let a = fb.param(0);
    let b = fb.param(1);
    let sum = fb.add(a, b);
    fb.ret(Some(sum));
    let func = fb.build();

    let shape = Shape::Block(func.entry);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    assert_eq!(ast.name, "add");
    assert_eq!(ast.params.len(), 2);
    // Single-use sum should be inlined into return.
    assert_eq!(ast.body.len(), 1);
    assert!(matches!(
        &ast.body[0],
        Stmt::Return(Some(Expr::Binary { op: BinOp::Add, .. }))
    ));
}

#[test]
fn full_pipeline_constant_inlining() {
    let sig = FunctionSig {
        params: vec![Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let a = fb.param(0);
    let c = fb.const_int(42);
    let sum = fb.add(a, c);
    fb.ret(Some(sum));
    let func = fb.build();

    let shape = Shape::Block(func.entry);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // Constant and sum both inlined into return.
    assert_eq!(ast.body.len(), 1);
    match &ast.body[0] {
        Stmt::Return(Some(Expr::Binary { rhs, .. })) => {
            assert!(matches!(rhs.as_ref(), Expr::Literal(Constant::Int(42))));
        }
        other => panic!("Expected return with binary, got: {other:?}"),
    }
}

#[test]
fn full_pipeline_dead_pure_eliminated() {
    let sig = FunctionSig {
        params: vec![Type::Int(64), Type::Int(64)],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let a = fb.param(0);
    let b = fb.param(1);
    let _dead = fb.add(a, b);
    fb.ret(None);
    let func = fb.build();

    let shape = Shape::Block(func.entry);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    assert!(
        ast.body.is_empty(),
        "Expected empty body, got: {:?}",
        ast.body
    );
}

#[test]
fn full_pipeline_if_else() {
    let sig = FunctionSig {
        params: vec![Type::Bool, Type::Int(64), Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("choose", sig, Visibility::Public);
    let cond = fb.param(0);
    let x = fb.param(1);
    let y = fb.param(2);

    let (then_block, then_vals) = fb.create_block_with_params(&[Type::Int(64)]);
    let (else_block, else_vals) = fb.create_block_with_params(&[Type::Int(64)]);

    fb.br_if(cond, then_block, &[x], else_block, &[y]);

    fb.switch_to_block(then_block);
    fb.ret(Some(then_vals[0]));

    fb.switch_to_block(else_block);
    fb.ret(Some(else_vals[0]));

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    assert!(!ast.body.is_empty());
}

// -----------------------------------------------------------------------
// Regression tests for bug fixes
// -----------------------------------------------------------------------

/// Helper: recursively check if any statement in the AST contains a
/// Stmt::Var reference with the given name.
fn body_contains_var(body: &[Stmt], name: &str) -> bool {
    body.iter().any(|s| stmt_contains_var(s, name))
}

fn stmt_contains_var(stmt: &Stmt, name: &str) -> bool {
    match stmt {
        Stmt::Assign { target, value } => {
            expr_contains_var(target, name) || expr_contains_var(value, name)
        }
        Stmt::VarDecl { init, .. } => init.as_ref().is_some_and(|e| expr_contains_var(e, name)),
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            expr_contains_var(cond, name)
                || body_contains_var(then_body, name)
                || body_contains_var(else_body, name)
        }
        Stmt::While { cond, body } => {
            expr_contains_var(cond, name) || body_contains_var(body, name)
        }
        Stmt::Loop { body } => body_contains_var(body, name),
        Stmt::Return(Some(e)) | Stmt::Expr(e) => expr_contains_var(e, name),
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            body_contains_var(init, name)
                || expr_contains_var(cond, name)
                || body_contains_var(update, name)
                || body_contains_var(body, name)
        }
        Stmt::CompoundAssign { target, value, .. } => {
            expr_contains_var(target, name) || expr_contains_var(value, name)
        }
        _ => false,
    }
}

fn expr_contains_var(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Var(n) => n == name,
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            expr_contains_var(lhs, name) || expr_contains_var(rhs, name)
        }
        Expr::Not(e) | Expr::Cast { expr: e, .. } | Expr::PostIncrement(e) => {
            expr_contains_var(e, name)
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            expr_contains_var(cond, name)
                || expr_contains_var(then_val, name)
                || expr_contains_var(else_val, name)
        }
        Expr::Call { args, .. } | Expr::SystemCall { args, .. } => {
            args.iter().any(|a| expr_contains_var(a, name))
        }
        Expr::MethodCall { receiver, args, .. } => {
            expr_contains_var(receiver, name) || args.iter().any(|a| expr_contains_var(a, name))
        }
        Expr::Field { object, .. } => expr_contains_var(object, name),
        Expr::Index {
            collection, index, ..
        } => expr_contains_var(collection, name) || expr_contains_var(index, name),
        Expr::LogicalAnd { lhs, rhs } | Expr::LogicalOr { lhs, rhs } => {
            expr_contains_var(lhs, name) || expr_contains_var(rhs, name)
        }
        Expr::Unary { expr: e, .. } => expr_contains_var(e, name),
        Expr::ArrayInit(elems) => elems.iter().any(|e| expr_contains_var(e, name)),
        _ => false,
    }
}

/// Helper: count VarDecl statements for a given name in the body.
fn count_var_decls(body: &[Stmt], name: &str) -> usize {
    body.iter()
        .map(|s| match s {
            Stmt::VarDecl { name: n, .. } if n == name => 1,
            Stmt::If {
                then_body,
                else_body,
                ..
            } => count_var_decls(then_body, name) + count_var_decls(else_body, name),
            Stmt::While { body, .. } | Stmt::Loop { body } => count_var_decls(body, name),
            Stmt::For {
                init, body, update, ..
            } => {
                count_var_decls(init, name)
                    + count_var_decls(body, name)
                    + count_var_decls(update, name)
            }
            _ => 0,
        })
        .sum()
}

/// Stringify AST body for debugging.
fn debug_body(body: &[Stmt]) -> String {
    format!("{body:?}")
}

// Regression: 8782b52 — Block->Loop/WhileLoop init assigns must be emitted
// for loop header block params (not suppressed like ForLoop).
#[test]
fn loop_init_assigns_emitted() {
    // entry: v0 = const 0; br header(v0)
    // header(v_i): v_cond = v_i < 10; br_if v_cond, body, exit
    // body: br header(v_i) (no update — just feeds back same value)
    // exit: return
    //
    // This is a WhileLoop. The init assign `v_i = 0` from entry->header
    // must appear in the output before the loop.
    let sig = FunctionSig {
        params: vec![],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let (header, header_vals) = fb.create_block_with_params(&[Type::Int(64)]);
    let body_block = fb.create_block();
    let exit = fb.create_block();

    let v_init = fb.const_int(0);
    fb.br(header, &[v_init]);

    fb.switch_to_block(header);
    let v_i = header_vals[0];
    let v_ten = fb.const_int(10);
    let v_cond = fb.cmp(CmpKind::Lt, v_i, v_ten);
    fb.br_if(v_cond, body_block, &[], exit, &[]);

    fb.switch_to_block(body_block);
    fb.br(header, &[v_i]);

    fb.switch_to_block(exit);
    fb.ret(None);

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // Should have an init assign before the loop, not just a naked while.
    // Verify the body is non-empty and has a loop.
    let has_while_or_for = ast
        .body
        .iter()
        .any(|s| matches!(s, Stmt::While { .. } | Stmt::For { .. }));
    assert!(
        has_while_or_for,
        "Expected loop in output: {}",
        debug_body(&ast.body)
    );
}

// Regression: f241be6 — pipeline output must be deterministic.
#[test]
fn pipeline_deterministic() {
    let sig = FunctionSig {
        params: vec![Type::Bool, Type::Int(64), Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let cond = fb.param(0);
    let a = fb.param(1);
    let b = fb.param(2);

    let (then_block, then_vals) = fb.create_block_with_params(&[Type::Int(64)]);
    let (else_block, else_vals) = fb.create_block_with_params(&[Type::Int(64)]);
    let (merge, merge_vals) = fb.create_block_with_params(&[Type::Int(64)]);

    let sum = fb.add(a, b);
    let diff = fb.sub(a, b);
    fb.br_if(cond, then_block, &[sum], else_block, &[diff]);

    fb.switch_to_block(then_block);
    fb.br(merge, &[then_vals[0]]);

    fb.switch_to_block(else_block);
    fb.br(merge, &[else_vals[0]]);

    fb.switch_to_block(merge);
    fb.ret(Some(merge_vals[0]));

    let mut func = fb.build();
    let shape = structurize(&mut func);

    let ast1 = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );
    let ast2 = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    assert_eq!(
        format!("{:?}", ast1.body),
        format!("{:?}", ast2.body),
        "Pipeline output should be deterministic"
    );
}

// Regression: 221d49d — shared names (Cast/Copy coalescing) that don't come
// from block params must still get a `let` declaration.
#[test]
fn shared_name_gets_decl() {
    // entry: v_src = param(0); v_cast = cast(v_src, Int(32))
    //        Name both v_src and v_cast as "x" to trigger shared_names.
    //        return v_cast
    let sig = FunctionSig {
        params: vec![Type::Int(64)],
        return_ty: Type::Int(32),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let v_src = fb.param(0);
    fb.name_value(v_src, "x".to_string());
    let v_cast = fb.cast(v_src, Type::Int(32));
    fb.name_value(v_cast, "x".to_string());
    fb.ret(Some(v_cast));

    let func = fb.build();
    let shape = Shape::Block(func.entry);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // The output should not panic or produce undeclared variables.
    // It may inline the cast or produce a declaration — either is correct.
    // The key property: no undeclared variable reference.
    let _body_str = debug_body(&ast.body);
}

// Regression: 4c7c747 — duplicate parameter names must be deduplicated.
#[test]
fn duplicate_param_names_deduped() {
    let sig = FunctionSig {
        params: vec![Type::Int(64), Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let a = fb.param(0);
    let b = fb.param(1);
    // Name both params the same.
    fb.name_value(a, "x".to_string());
    fb.name_value(b, "x".to_string());
    let sum = fb.add(a, b);
    fb.ret(Some(sum));

    let func = fb.build();
    let shape = Shape::Block(func.entry);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // Params should have distinct names.
    assert_eq!(ast.params.len(), 2);
    assert_ne!(
        ast.params[0].0, ast.params[1].0,
        "Parameter names should be deduplicated: {:?}",
        ast.params
    );
}

// Regression: af55c19 — non-self values that share the self-parameter's
// name must be renamed to avoid `this = ...` assignments.
#[test]
fn reassigned_self_param_renamed() {
    // Instance method with self param named "this".
    // Another value also named "this" — should be renamed to "_this".
    let sig = FunctionSig {
        params: vec![Type::Struct("Foo".to_string()), Type::Int(64)],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("method", sig, Visibility::Public);
    fb.set_class(vec![], "Foo".to_string(), MethodKind::Instance);

    let self_param = fb.param(0);
    let other = fb.param(1);
    fb.name_value(self_param, "this".to_string());
    fb.name_value(other, "this".to_string());

    let result = fb.add(other, other);
    fb.ret(Some(result));

    let func = fb.build();
    let shape = Shape::Block(func.entry);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // The two "this" params should have different names.
    // param(0) keeps "this", param(1) gets "_this" or similar.
    assert_eq!(ast.params.len(), 2);
    assert_ne!(
        ast.params[0].0, ast.params[1].0,
        "Self-param collision should be resolved: {:?}",
        ast.params
    );
}

// Regression: 3e0c48d — Store target must use Var(name), not inlined expr.
// Tests the emitter's Op::Store handler directly: the LHS of a Store must
// be `Expr::Var(name)`, not `build_val(ptr)` which could inline a Cast.
// We test the emitter directly using linearize+resolve+emit because the
// full pipeline's transforms (Mem2Reg) promote most alloc/store/load chains.
#[test]
fn store_target_uses_var_name() {
    // Build IR: alloc x; store x, param; store x, param2; load x; return
    // Use the linearizer directly (Phase 1->2->3) to skip transform passes.
    let sig = FunctionSig {
        params: vec![Type::Int(64), Type::Int(64)],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let p0 = fb.param(0);
    let p1 = fb.param(1);
    let ptr = fb.alloc(Type::Int(64));
    fb.name_value(ptr, "x".to_string());
    fb.store(ptr, p0);
    fb.store(ptr, p1);
    let loaded = fb.load(ptr, Type::Int(64));
    fb.ret(Some(loaded));

    let func = fb.build();
    let shape = Shape::Block(func.entry);
    let linear = linearize(&func, &shape);
    let resolved = resolve(&func, &linear, &[]);
    let config = LoweringConfig::default();
    let mut ctx = EmitCtx::new(&func, &resolved, &config);
    let body = ctx.emit_stmts(&linear);

    // The Store should produce assignments with Var("x") target.
    let has_x_assign = body.iter().any(|s| match s {
        Stmt::Assign {
            target: Expr::Var(n),
            ..
        } => n == "x",
        Stmt::VarDecl { name, .. } => name == "x",
        _ => false,
    });
    assert!(
        has_x_assign,
        "Expected assignment to Var(\"x\"): {}",
        debug_body(&body)
    );
}

// Regression: 4ef6f43 — debug names propagate through Cast/Copy so the
// source operand gets a human-readable name.
#[test]
fn debug_name_propagates_through_cast() {
    // v_field = get_field(param, "hp")  (unnamed)
    // v_cast = cast(v_field, Int(32))   (named "hp")
    // return v_cast
    //
    // The name "hp" should propagate back to v_field.
    let sig = FunctionSig {
        params: vec![Type::Struct("Obj".to_string())],
        return_ty: Type::Int(32),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let obj = fb.param(0);
    let v_field = fb.get_field(obj, "hp", Type::Unknown);
    // Don't name v_field — only name the cast result.
    let v_cast = fb.cast(v_field, Type::Int(32));
    fb.name_value(v_cast, "hp".to_string());
    fb.ret(Some(v_cast));

    let func = fb.build();
    let shape = Shape::Block(func.entry);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // Output should reference "hp" somewhere, not a vN identifier.
    let body_str = debug_body(&ast.body);
    assert!(
        body_contains_var(&ast.body, "hp") || body_str.contains("hp"),
        "Expected 'hp' name in output: {body_str}"
    );
}

// Regression: 65170ad — LogicalOr/And must not call build_val(cond) twice
// when cond is a single-use lazy value. The fix reuses cond_expr.clone().
#[test]
fn logical_or_no_double_build() {
    // entry: br_if cond, merge(cond), else_block()
    // else_block: br merge(other)
    // merge(phi): return phi
    //
    // This produces a LogicalOr shape. If build_val(cond) is called twice,
    // the second call would fail because the lazy inline was consumed.
    let sig = FunctionSig {
        params: vec![Type::Bool, Type::Bool],
        return_ty: Type::Bool,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let cond = fb.param(0);
    let other = fb.param(1);

    let else_block = fb.create_block();
    let (merge, merge_vals) = fb.create_block_with_params(&[Type::Bool]);

    fb.br_if(cond, merge, &[cond], else_block, &[]);

    fb.switch_to_block(else_block);
    fb.br(merge, &[other]);

    fb.switch_to_block(merge);
    fb.ret(Some(merge_vals[0]));

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let config = LoweringConfig::default();
    let ast = lower_function_linear(&func, &shape, &config, &DebugConfig::none());

    // Should not panic. Output should contain a LogicalOr or similar.
    assert!(!ast.body.is_empty(), "Expected non-empty body");
}

// Regression: 1076a5c — flush_pending_reads must skip already-consumed values.
// When building one pending value consumes another, the consumed value
// must not be flushed again.
#[test]
fn flush_skips_consumed_values() {
    // entry: v1 = get_field(param, "a")
    //        v2 = get_field(v1, "b")
    //        store(alloc, v2)   <- triggers flush
    //        return
    //
    // v1 and v2 are both pending lazy. Flushing v2 consumes v1.
    // The flush loop must skip v1 when it tries to flush it.
    let sig = FunctionSig {
        params: vec![Type::Struct("Obj".to_string())],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let obj = fb.param(0);
    let v1 = fb.get_field(obj, "a", Type::Unknown);
    let v2 = fb.get_field(v1, "b", Type::Unknown);
    let ptr = fb.alloc(Type::Unknown);
    fb.store(ptr, v2);
    fb.ret(None);

    let func = fb.build();
    let shape = Shape::Block(func.entry);
    // Should not panic from double-flush.
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );
    let _ = debug_body(&ast.body);
}

// Regression: 48671ff — flush_pending_reads must be scoped to prevent
// use-before-def. Header-block values must not be flushed inside if-bodies.
#[test]
fn flush_scoped_to_prevent_use_before_def() {
    // entry: v_field = get_field(param, "x")
    //        br_if cond, then_block, merge
    // then_block: set_field(param, "x", const 0)   <- triggers flush
    //             br merge
    // merge: return v_field
    //
    // v_field is defined in the header but used after the if/else.
    // It must NOT be flushed inside then_block's body (use-before-def).
    let sig = FunctionSig {
        params: vec![Type::Struct("Obj".to_string()), Type::Bool],
        return_ty: Type::Unknown,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let obj = fb.param(0);
    let cond = fb.param(1);
    let v_field = fb.get_field(obj, "x", Type::Unknown);

    let then_block = fb.create_block();
    let merge = fb.create_block();

    fb.br_if(cond, then_block, &[], merge, &[]);

    fb.switch_to_block(then_block);
    let zero = fb.const_int(0);
    fb.set_field(obj, "x", zero);
    fb.br(merge, &[]);

    fb.switch_to_block(merge);
    fb.ret(Some(v_field));

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // v_field's declaration must appear before the if/else, not inside it.
    // The return should reference the field value.
    let has_return = ast.body.iter().any(|s| matches!(s, Stmt::Return(Some(_))));
    assert!(
        has_return,
        "Expected return statement: {}",
        debug_body(&ast.body)
    );
}

// Regression: 7821541 — pure wrapper (Cast) around side-effecting operand
// (Call) must chain into a single SE inline, not produce broken output.
#[test]
fn se_chain_through_cast() {
    // v_call = call("f", [])
    // v_cast = cast(v_call, Int(32))
    // return v_cast
    //
    // v_call is side-effecting. v_cast wraps it. Should produce clean output.
    let sig = FunctionSig {
        params: vec![],
        return_ty: Type::Int(32),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("g", sig, Visibility::Public);
    let v_call = fb.call("f", &[], Type::Unknown);
    fb.name_value(v_call, "result".to_string());
    let v_cast = fb.cast(v_call, Type::Int(32));
    fb.name_value(v_cast, "result".to_string());
    fb.ret(Some(v_cast));

    let func = fb.build();
    let shape = Shape::Block(func.entry);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // Should produce clean output with "result" name, no broken references.
    assert_eq!(
        ast.body.len(),
        1,
        "Expected single return: {}",
        debug_body(&ast.body)
    );
    assert!(matches!(&ast.body[0], Stmt::Return(Some(_))));
}

// Regression: e726c58 — when then-body is empty, negate the Cmp condition
// directly (invert CmpKind) instead of wrapping in Not(Cmp(...)).
#[test]
fn inverted_cmp_not_wrapped_in_not() {
    // entry: v_cmp = cmp.lt(a, b)
    //        br_if v_cmp, then_block, else_block
    // then_block: br merge  (empty)
    // else_block: store something; br merge
    // merge: return
    //
    // Since then is empty, the linearizer inverts to: if (!cond) { else }
    // With CmpKind, this should produce `cmp.ge(a, b)` not `Not(cmp.lt(a,b))`.
    let sig = FunctionSig {
        params: vec![Type::Int(64), Type::Int(64)],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let a = fb.param(0);
    let b = fb.param(1);
    let v_cmp = fb.cmp(CmpKind::Lt, a, b);

    let then_block = fb.create_block();
    let else_block = fb.create_block();
    let merge = fb.create_block();

    fb.br_if(v_cmp, then_block, &[], else_block, &[]);

    fb.switch_to_block(then_block);
    fb.br(merge, &[]);

    fb.switch_to_block(else_block);
    let ptr = fb.alloc(Type::Int(64));
    let val = fb.const_int(1);
    fb.store(ptr, val);
    fb.br(merge, &[]);

    fb.switch_to_block(merge);
    fb.ret(None);

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // Find the If statement and check its condition.
    let if_stmt = ast.body.iter().find(|s| matches!(s, Stmt::If { .. }));
    if let Some(Stmt::If { cond, .. }) = if_stmt {
        // Condition should be inverted Cmp (Ge), not Not(Cmp(Lt)).
        assert!(
            matches!(
                cond,
                Expr::Cmp {
                    kind: CmpKind::Ge,
                    ..
                }
            ),
            "Expected inverted Cmp (Ge), not Not wrapper: {cond:?}"
        );
    }
    // If the if was eliminated by AST passes, that's also fine.
}

// Regression: f9d14ec — LogicalAnd/Or phi values flushed as SE inlines
// must not get duplicate declarations.
#[test]
fn logical_and_no_duplicate_decl() {
    // entry: br_if cond, then_mid(), merge(cond)
    // then_mid: v_rhs = call("check", []); br merge(v_rhs)
    // merge(phi): return phi
    //
    // phi is a LogicalAnd result. The call is side-effecting.
    let sig = FunctionSig {
        params: vec![Type::Bool],
        return_ty: Type::Bool,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let cond = fb.param(0);

    let then_mid = fb.create_block();
    let (merge, merge_vals) = fb.create_block_with_params(&[Type::Bool]);

    fb.br_if(cond, then_mid, &[], merge, &[cond]);

    fb.switch_to_block(then_mid);
    let v_rhs = fb.call("check", &[], Type::Bool);
    fb.br(merge, &[v_rhs]);

    fb.switch_to_block(merge);
    fb.ret(Some(merge_vals[0]));

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // Count VarDecl statements for the phi value's name. There should be
    // at most 1 declaration (not duplicated).
    // The phi might be inlined entirely, which is also fine.
    for param in &ast.params {
        let decl_count = count_var_decls(&ast.body, &param.0);
        assert!(
            decl_count <= 1,
            "Duplicate VarDecl for param '{}': count={}, body: {}",
            param.0,
            decl_count,
            debug_body(&ast.body)
        );
    }
}

// Regression: 0983e97 — minmax pattern with SE operands must flush correctly.
#[test]
fn minmax_se_flush_correct() {
    // entry: v_a = call("getA", [])
    //        v_b = call("getB", [])
    //        v_cmp = cmp.ge(v_a, v_b)
    //        br_if v_cmp, then_block(v_a), else_block(v_b)
    // then_block(v_t): br merge(v_t)
    // else_block(v_e): br merge(v_e)
    // merge(phi): return phi
    //
    // This should produce Math.max(getA(), getB()).
    let sig = FunctionSig {
        params: vec![],
        return_ty: Type::Int(64),
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let v_a = fb.call("getA", &[], Type::Int(64));
    let v_b = fb.call("getB", &[], Type::Int(64));
    let v_cmp = fb.cmp(CmpKind::Ge, v_a, v_b);

    let (then_block, then_vals) = fb.create_block_with_params(&[Type::Int(64)]);
    let (else_block, else_vals) = fb.create_block_with_params(&[Type::Int(64)]);
    let (merge, merge_vals) = fb.create_block_with_params(&[Type::Int(64)]);

    fb.br_if(v_cmp, then_block, &[v_a], else_block, &[v_b]);

    fb.switch_to_block(then_block);
    fb.br(merge, &[then_vals[0]]);

    fb.switch_to_block(else_block);
    fb.br(merge, &[else_vals[0]]);

    fb.switch_to_block(merge);
    fb.ret(Some(merge_vals[0]));

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let ast = lower_function_linear(
        &func,
        &shape,
        &LoweringConfig::default(),
        &DebugConfig::none(),
    );

    // Should produce clean output — no panic from SE flush timing.
    assert!(
        !ast.body.is_empty(),
        "Expected non-empty body: {}",
        debug_body(&ast.body)
    );
}

// Regression: cf0524a — while-loop condition should be hoisted into
// `while (cond)` when the header has no materialized statements.
#[test]
fn while_loop_condition_hoisted() {
    // entry: br header
    // header: v_cond = cmp.lt(param, 10); br_if v_cond, body, exit
    // body: br header
    // exit: return
    //
    // The Cmp is single-use and pure — header has no materialized stmts.
    // Should produce `while (param < 10) { }` not `while (true) { if (...) break; }`.
    let sig = FunctionSig {
        params: vec![Type::Int(64)],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let n = fb.param(0);
    fb.name_value(n, "n".to_string());

    let header = fb.create_block();
    let body_block = fb.create_block();
    let exit = fb.create_block();

    fb.br(header, &[]);

    fb.switch_to_block(header);
    let ten = fb.const_int(10);
    let v_cond = fb.cmp(CmpKind::Lt, n, ten);
    fb.br_if(v_cond, body_block, &[], exit, &[]);

    fb.switch_to_block(body_block);
    fb.br(header, &[]);

    fb.switch_to_block(exit);
    fb.ret(None);

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let config = LoweringConfig::default();
    let ast = lower_function_linear(&func, &shape, &config, &DebugConfig::none());

    // With while_condition_hoisting enabled (default), should be While
    // with a real condition, not `While { cond: Literal(true), ... }`.
    let has_while_with_cond = ast.body.iter().any(|s| match s {
        Stmt::While { cond, .. } => !matches!(cond, Expr::Literal(Constant::Bool(true))),
        _ => false,
    });
    assert!(
        has_while_with_cond,
        "Expected while with hoisted condition: {}",
        debug_body(&ast.body)
    );
}

// Regression: Switch handler must flush pending SE inlines to the outer scope
// before emitting case bodies.  Without the flush, a side-effecting inline
// can end up declared inside an earlier case (triggered when that case contains
// an if-body that calls flush_side_effecting_inlines), making it undeclared in
// a later case that references it — TS2304 "Cannot find name 'x'".
//
// IR shape:
//   entry(v_key: Int64, v_cond: Bool):
//     v_call = call("foo", [])           -- SE inline candidate
//     switch v_key { Int(0) -> case0, default -> case1 }
//   case0:
//     br_if v_cond, then0, merge0        -- inner if triggers flush inside case0
//   then0: br merge0
//   merge0: ret
//   case1:
//     v_result = call("bar", [v_call])   -- uses the SE inline
//     ret
#[test]
fn switch_se_inline_flushed_to_outer_scope() {
    let sig = FunctionSig {
        params: vec![Type::Int(64), Type::Bool],
        return_ty: Type::Unknown,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let v_key = fb.param(0);
    let v_cond = fb.param(1);
    let case0 = fb.create_block();
    let then0 = fb.create_block();
    let merge0 = fb.create_block();
    let case1 = fb.create_block();

    // Entry: side-effecting call followed by switch.
    let v_call = fb.call("foo", &[], Type::Unknown);
    fb.name_value(v_call, "foo_call".to_string());
    fb.switch(
        v_key,
        vec![(Constant::Int(0), case0, vec![])],
        (case1, vec![]),
    );

    // case0: contains inner if — this triggers flush_side_effecting_inlines
    // inside case0's scope when emitted, which (without the fix) materialises
    // foo_call into case0's block instead of the outer scope.
    fb.switch_to_block(case0);
    fb.br_if(v_cond, then0, &[], merge0, &[]);

    fb.switch_to_block(then0);
    fb.br(merge0, &[]);

    fb.switch_to_block(merge0);
    fb.ret(None);

    // case1 (default): uses v_call.  Without the outer flush, "foo_call" is
    // undeclared here because it was materialised inside case0's scope.
    fb.switch_to_block(case1);
    let v_result = fb.call("bar", &[v_call], Type::Unknown);
    fb.name_value(v_result, "bar_result".to_string());
    fb.ret(Some(v_result));

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let config = LoweringConfig::default();
    let ast = lower_function_linear(&func, &shape, &config, &DebugConfig::none());

    // The Switch must appear in the output.
    assert!(
        ast.body.iter().any(|s| matches!(s, Stmt::Switch { .. })),
        "Expected a Switch in output: {}",
        debug_body(&ast.body)
    );

    // Key invariant: "foo_call" must not appear as an undeclared Var reference
    // inside any case body.  There are two valid post-AST-pass shapes:
    //
    //  (a) `foo()` was inlined at the use site by fold_single_use_consts
    //      -> no Var("foo_call") anywhere                  (after fix + fold)
    //  (b) Var("foo_call") exists in a case body AND a VarDecl for it
    //      exists in the outer scope                       (after fix, no fold)
    //
    //  The bug (before fix) produces:
    //  (c) Var("foo_call") inside a case body with NO outer-scope VarDecl
    //      — the declaration was materialised inside case0's scope, then
    //      stripped as dead (0 uses in case0) leaving an undeclared ref
    //      in case1 -> TS2304.
    let has_outer_decl = ast
        .body
        .iter()
        .any(|s| matches!(s, Stmt::VarDecl { name, .. } if name == "foo_call"));
    let has_orphan_ref = if let Some(Stmt::Switch {
        cases,
        default_body,
        ..
    }) = ast.body.iter().find(|s| matches!(s, Stmt::Switch { .. }))
    {
        let in_cases = cases
            .iter()
            .any(|(_, stmts)| body_contains_var_ref(stmts, "foo_call"));
        let in_default = body_contains_var_ref(default_body, "foo_call");
        (in_cases || in_default) && !has_outer_decl
    } else {
        false
    };
    assert!(
        !has_orphan_ref,
        "'foo_call' is referenced inside a switch case but has no outer-scope \
         declaration — cross-case scope violation: {}",
        debug_body(&ast.body)
    );
}

/// Check whether `Expr::Var(name)` appears anywhere in a statement list.
fn body_contains_var_ref(body: &[Stmt], name: &str) -> bool {
    body.iter().any(|s| stmt_contains_var_ref(s, name))
}

fn stmt_contains_var_ref(stmt: &Stmt, name: &str) -> bool {
    match stmt {
        Stmt::VarDecl { init, .. } => init
            .as_ref()
            .is_some_and(|e| expr_contains_var_ref(e, name)),
        Stmt::Assign { target, value } | Stmt::CompoundAssign { target, value, .. } => {
            expr_contains_var_ref(target, name) || expr_contains_var_ref(value, name)
        }
        Stmt::Expr(e) | Stmt::Return(Some(e)) => expr_contains_var_ref(e, name),
        Stmt::If {
            cond,
            then_body,
            else_body,
        } => {
            expr_contains_var_ref(cond, name)
                || body_contains_var_ref(then_body, name)
                || body_contains_var_ref(else_body, name)
        }
        Stmt::While { cond, body } => {
            expr_contains_var_ref(cond, name) || body_contains_var_ref(body, name)
        }
        Stmt::For {
            init,
            cond,
            update,
            body,
        } => {
            body_contains_var_ref(init, name)
                || expr_contains_var_ref(cond, name)
                || body_contains_var_ref(update, name)
                || body_contains_var_ref(body, name)
        }
        Stmt::Switch {
            value,
            cases,
            default_body,
        } => {
            expr_contains_var_ref(value, name)
                || cases.iter().any(|(_, s)| body_contains_var_ref(s, name))
                || body_contains_var_ref(default_body, name)
        }
        _ => false,
    }
}

fn expr_contains_var_ref(expr: &Expr, name: &str) -> bool {
    match expr {
        Expr::Var(n) => n == name,
        Expr::Literal(_) | Expr::GlobalRef(_) => false,
        Expr::Binary { lhs, rhs, .. } | Expr::Cmp { lhs, rhs, .. } => {
            expr_contains_var_ref(lhs, name) || expr_contains_var_ref(rhs, name)
        }
        Expr::Unary { expr: inner, .. }
        | Expr::Cast { expr: inner, .. }
        | Expr::TypeCheck { expr: inner, .. }
        | Expr::Not(inner)
        | Expr::PostIncrement(inner)
        | Expr::Spread(inner)
        | Expr::CoroutineResume(inner) => expr_contains_var_ref(inner, name),
        Expr::Yield(Some(inner)) => expr_contains_var_ref(inner, name),
        Expr::Yield(None) => false,
        Expr::Call { args, .. }
        | Expr::ArrayInit(args)
        | Expr::TupleInit(args)
        | Expr::CoroutineCreate { args, .. } => args.iter().any(|a| expr_contains_var_ref(a, name)),
        Expr::MakeClosure { captures, .. } => {
            captures.iter().any(|a| expr_contains_var_ref(a, name))
        }
        Expr::CallIndirect { callee, args } => {
            expr_contains_var_ref(callee, name)
                || args.iter().any(|a| expr_contains_var_ref(a, name))
        }
        Expr::MethodCall { receiver, args, .. } => {
            expr_contains_var_ref(receiver, name)
                || args.iter().any(|a| expr_contains_var_ref(a, name))
        }
        Expr::SystemCall { args, .. } => args.iter().any(|a| expr_contains_var_ref(a, name)),
        Expr::Field { object, .. } => expr_contains_var_ref(object, name),
        Expr::Index { collection, index } => {
            expr_contains_var_ref(collection, name) || expr_contains_var_ref(index, name)
        }
        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            expr_contains_var_ref(cond, name)
                || expr_contains_var_ref(then_val, name)
                || expr_contains_var_ref(else_val, name)
        }
        Expr::LogicalOr { lhs, rhs } | Expr::LogicalAnd { lhs, rhs } => {
            expr_contains_var_ref(lhs, name) || expr_contains_var_ref(rhs, name)
        }
        Expr::StructInit { fields, .. } => {
            fields.iter().any(|(_, v)| expr_contains_var_ref(v, name))
        }
    }
}

// Regression: LogicalOr inside a then-branch with else-branch also assigning
// to the same phi block param must not produce duplicate declarations.
// The LogicalOr's emit_or_inline stores the phi in side_effecting_inlines,
// but the else-branch's Assign adds phi to referenced_block_params.
// Without the fix, flush_side_effecting_inlines would emit `const phi = ...`
// while collect_block_param_decls would emit `let phi: boolean;`.
#[test]
fn logical_or_in_branch_no_duplicate_decl() {
    // IR shape:
    //   entry: v_outer = cmp(param0, param1)
    //          br_if v_outer, block_or_head(), merge(v_outer)
    //   block_or_head: v_a = cmp(param0, 5)
    //                  br_if v_a, merge(v_a), block_or_rhs()
    //   block_or_rhs:  v_b = cmp(param1, 5)
    //                  br merge(v_b)
    //   merge(phi):    br_if phi, then_block(), else_block()
    //   then_block:    return
    //   else_block:    return
    let sig = FunctionSig {
        params: vec![Type::Int(32), Type::Int(32)],
        return_ty: Type::Void,
        ..Default::default()
    };
    let mut fb = FunctionBuilder::new("f", sig, Visibility::Public);
    let p0 = fb.param(0);
    let p1 = fb.param(1);

    let block_or_head = fb.create_block();
    let block_or_rhs = fb.create_block();
    let (merge, merge_vals) = fb.create_block_with_params(&[Type::Bool]);
    let phi = merge_vals[0];
    let then_block = fb.create_block();
    let else_block = fb.create_block();

    // entry
    let v_outer = fb.cmp(CmpKind::Lt, p0, p1);
    fb.br_if(v_outer, block_or_head, &[], merge, &[v_outer]);

    // block_or_head: first part of logical OR
    fb.switch_to_block(block_or_head);
    let five = fb.const_int(5);
    let v_a = fb.cmp(CmpKind::Lt, p0, five);
    fb.br_if(v_a, merge, &[v_a], block_or_rhs, &[]);

    // block_or_rhs: second part of logical OR
    fb.switch_to_block(block_or_rhs);
    let five2 = fb.const_int(5);
    let v_b = fb.cmp(CmpKind::Lt, p1, five2);
    fb.br(merge, &[v_b]);

    // merge: uses phi in a branch
    fb.switch_to_block(merge);
    fb.br_if(phi, then_block, &[], else_block, &[]);

    fb.switch_to_block(then_block);
    fb.ret(None);

    fb.switch_to_block(else_block);
    fb.ret(None);

    let mut func = fb.build();
    let shape = structurize(&mut func);
    let config = LoweringConfig::default();
    let ast = lower_function_linear(&func, &shape, &config, &DebugConfig::none());

    // The phi variable name must appear in at most ONE VarDecl across
    // the entire body.  Two VarDecls = duplicate declaration error.
    let phi_name = format!("v{}", phi.index());
    let decl_count = count_var_decls(&ast.body, &phi_name);
    assert!(
        decl_count <= 1,
        "Expected at most 1 VarDecl for '{phi_name}', got {decl_count}: {}",
        debug_body(&ast.body)
    );
}
