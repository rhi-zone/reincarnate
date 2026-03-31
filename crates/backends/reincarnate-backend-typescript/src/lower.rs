//! Core AST → JS AST mechanical lowering pass.
//!
//! Converts the engine-agnostic `Stmt`/`Expr` tree into `JsStmt`/`JsExpr`
//! with a 1:1 structural mapping. Engine-specific rewrites (scope-lookup
//! resolution, SystemCall → native JS constructs, etc.) are handled by a
//! separate post-lowering rewrite pass (e.g. `rewrites::flash`).

use reincarnate_core::ir::ast::{AstFunction, Expr, Stmt};
use reincarnate_core::ir::{CastKind, CmpKind, Type};

use crate::js_ast::{BinOp, UnaryOp};

use crate::js_ast::{JsExpr, JsFunction, JsStmt};

// ---------------------------------------------------------------------------
// Lowering context
// ---------------------------------------------------------------------------

/// Context for the lowering pass.
pub struct LowerCtx {
    /// Self parameter name — the IR parameter that maps to `this`.
    pub self_param_name: Option<String>,
}

// ---------------------------------------------------------------------------
// Function lowering
// ---------------------------------------------------------------------------

/// Lower an entire function from core AST to JS AST.
pub fn lower_function(ast: &AstFunction, ctx: &LowerCtx) -> JsFunction {
    let body = lower_stmts(&ast.body, ctx);

    JsFunction {
        name: ast.name.clone(),
        params: ast.params.clone(),
        param_defaults: ast.param_defaults.clone(),
        return_ty: ast.return_ty.clone(),
        body,
        is_generator: ast.is_generator,
        visibility: ast.visibility,
        method_kind: ast.method_kind,
        has_rest_param: ast.has_rest_param,
        num_capture_params: ast.num_capture_params,
        capture_modes: ast.capture_modes.clone(),
    }
}

// ---------------------------------------------------------------------------
// Statement lowering
// ---------------------------------------------------------------------------

fn lower_stmts(stmts: &[Stmt], ctx: &LowerCtx) -> Vec<JsStmt> {
    stmts.iter().map(|s| lower_stmt(s, ctx)).collect()
}

/// Lower a single statement.
fn lower_stmt(stmt: &Stmt, ctx: &LowerCtx) -> JsStmt {
    match stmt {
        Stmt::VarDecl {
            name,
            ty,
            init,
            mutable,
        } => JsStmt::VarDecl {
            name: name.clone(),
            ty: ty.clone(),
            init: init.as_ref().map(|e| lower_expr(e, ctx)),
            mutable: *mutable,
        },

        Stmt::Assign { target, value } => JsStmt::Assign {
            target: lower_expr(target, ctx),
            value: lower_expr(value, ctx),
        },

        Stmt::Expr(expr) => JsStmt::Expr(lower_expr(expr, ctx)),

        Stmt::If {
            cond,
            then_body,
            else_body,
        } => JsStmt::If {
            cond: lower_expr(cond, ctx),
            then_body: lower_stmts(then_body, ctx),
            else_body: lower_stmts(else_body, ctx),
        },

        Stmt::While { cond, body } => JsStmt::While {
            cond: lower_expr(cond, ctx),
            body: lower_stmts(body, ctx),
        },

        Stmt::For {
            init,
            cond,
            update,
            body,
        } => JsStmt::For {
            init: lower_stmts(init, ctx),
            cond: lower_expr(cond, ctx),
            update: lower_stmts(update, ctx),
            body: lower_stmts(body, ctx),
        },

        Stmt::Loop { body } => JsStmt::Loop {
            body: lower_stmts(body, ctx),
        },

        Stmt::ForOf {
            binding,
            declare,
            binding_ty,
            iterable,
            body,
        } => JsStmt::ForOf {
            binding: binding.clone(),
            declare: *declare,
            binding_ty: binding_ty.clone(),
            iterable: lower_expr(iterable, ctx),
            body: lower_stmts(body, ctx),
        },

        Stmt::Return(expr) => JsStmt::Return(expr.as_ref().map(|e| lower_expr(e, ctx))),
        Stmt::Break => JsStmt::Break,
        Stmt::Continue => JsStmt::Continue,
        Stmt::LabeledBreak { depth } => JsStmt::LabeledBreak { depth: *depth },

        Stmt::Dispatch { blocks, entry } => JsStmt::Dispatch {
            blocks: blocks
                .iter()
                .map(|(idx, stmts)| (*idx, lower_stmts(stmts, ctx)))
                .collect(),
            entry: *entry,
        },

        Stmt::Switch {
            value,
            cases,
            default_body,
        } => JsStmt::Switch {
            value: lower_expr(value, ctx),
            cases: cases
                .iter()
                .map(|(c, stmts)| (JsExpr::Literal(c.clone()), lower_stmts(stmts, ctx)))
                .collect(),
            default_body: lower_stmts(default_body, ctx),
        },
    }
}

// ---------------------------------------------------------------------------
// Expression lowering
// ---------------------------------------------------------------------------

/// Lower a single expression from core AST to JS AST.
fn lower_expr(expr: &Expr, ctx: &LowerCtx) -> JsExpr {
    match expr {
        Expr::Literal(c) => JsExpr::Literal(c.clone()),

        Expr::Var(name) => {
            if let Some(ref self_name) = ctx.self_param_name {
                if name == self_name {
                    return JsExpr::This;
                }
            }
            JsExpr::Var(name.clone())
        }

        Expr::Cmp { kind, lhs, rhs } => JsExpr::Cmp {
            kind: *kind,
            lhs: Box::new(lower_expr(lhs, ctx)),
            rhs: Box::new(lower_expr(rhs, ctx)),
        },

        Expr::Field { object, field } => lower_field(object, field, ctx),

        Expr::Index { collection, index } => JsExpr::Index {
            collection: Box::new(lower_expr(collection, ctx)),
            index: Box::new(lower_expr(index, ctx)),
        },

        Expr::Call { func: fname, args } => lower_call(fname, args, ctx),

        Expr::CallIndirect { callee, args } => JsExpr::Call {
            callee: Box::new(lower_expr(callee, ctx)),
            args: lower_exprs(args, ctx),
        },

        Expr::SystemCall {
            system,
            method,
            args,
        } => JsExpr::SystemCall {
            system: system.clone(),
            method: method.clone(),
            args: lower_exprs(args, ctx),
        },

        Expr::MethodCall {
            receiver,
            method,
            args,
        } => JsExpr::Call {
            callee: Box::new(lower_field(receiver, method, ctx)),
            args: lower_exprs(args, ctx),
        },

        Expr::Ternary {
            cond,
            then_val,
            else_val,
        } => JsExpr::Ternary {
            cond: Box::new(lower_expr(cond, ctx)),
            then_val: Box::new(lower_expr(then_val, ctx)),
            else_val: Box::new(lower_expr(else_val, ctx)),
        },

        Expr::LogicalOr { lhs, rhs } => JsExpr::LogicalOr {
            lhs: Box::new(lower_expr(lhs, ctx)),
            rhs: Box::new(lower_expr(rhs, ctx)),
        },

        Expr::LogicalAnd { lhs, rhs } => JsExpr::LogicalAnd {
            lhs: Box::new(lower_expr(lhs, ctx)),
            rhs: Box::new(lower_expr(rhs, ctx)),
        },

        Expr::Cast {
            expr: inner,
            ty,
            kind,
        } => JsExpr::Cast {
            expr: Box::new(lower_expr(inner, ctx)),
            ty: ty.clone(),
            kind: *kind,
        },

        Expr::TypeCheck { expr: inner, ty } => JsExpr::TypeCheck {
            expr: Box::new(lower_expr(inner, ctx)),
            ty: ty.clone(),
            use_instanceof: false,
        },

        Expr::ArrayInit(elems) => JsExpr::ArrayInit(lower_exprs(elems, ctx)),

        Expr::StructInit { name: _, fields } => {
            let pairs: Vec<(String, JsExpr)> = fields
                .iter()
                .map(|(name, val)| (name.clone(), lower_expr(val, ctx)))
                .collect();
            JsExpr::ObjectInit(pairs)
        }

        Expr::TupleInit(elems) => JsExpr::TupleInit(lower_exprs(elems, ctx)),

        Expr::GlobalRef(name) => JsExpr::Var(name.clone()),

        Expr::CoroutineCreate { func: fname, args } => JsExpr::GeneratorCreate {
            func: fname.clone(),
            args: lower_exprs(args, ctx),
        },

        Expr::CoroutineResume(inner) => JsExpr::GeneratorResume(Box::new(lower_expr(inner, ctx))),

        Expr::Yield(v) => JsExpr::Yield(v.as_ref().map(|e| Box::new(lower_expr(e, ctx)))),

        Expr::Not(inner) => JsExpr::Not(Box::new(lower_expr(inner, ctx))),

        Expr::PostIncrement(inner) => JsExpr::PostIncrement(Box::new(lower_expr(inner, ctx))),

        Expr::Spread(inner) => JsExpr::Spread(Box::new(lower_expr(inner, ctx))),

        Expr::MakeClosure { func, captures } => {
            // Lower to a SugarCube.Engine.closure syscall with the function name
            // as the first arg and capture values as subsequent args.  The twine
            // rewrite pass converts this to an IIFE when captures are present,
            // or a plain ArrowFunction when there are none.
            let mut args = Vec::with_capacity(captures.len() + 1);
            args.push(JsExpr::Literal(
                reincarnate_core::ir::value::Constant::String(func.clone()),
            ));
            args.extend(lower_exprs(captures, ctx));
            JsExpr::SystemCall {
                system: "SugarCube.Engine".to_string(),
                method: "closure".to_string(),
                args,
            }
        }
    }
}

/// Lower a slice of expressions.
fn lower_exprs(exprs: &[Expr], ctx: &LowerCtx) -> Vec<JsExpr> {
    exprs.iter().map(|e| lower_expr(e, ctx)).collect()
}

// ---------------------------------------------------------------------------
// Field access lowering
// ---------------------------------------------------------------------------

/// Lower a field access with `::` namespace stripping (IR convention).
fn lower_field(object: &Expr, field: &str, ctx: &LowerCtx) -> JsExpr {
    let effective = if field.contains("::") {
        field.rsplit("::").next().unwrap_or(field)
    } else {
        field
    };

    JsExpr::Field {
        object: Box::new(lower_expr(object, ctx)),
        field: effective.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Call lowering
// ---------------------------------------------------------------------------

/// Lower a Call expression, handling dotted paths and free function calls.
fn lower_call(fname: &str, args: &[Expr], ctx: &LowerCtx) -> JsExpr {
    // Builtin operator expansion — intercept before dotted-path handling.
    if let Some(op_name) = fname.strip_prefix("builtin.") {
        return lower_builtin(op_name, args, ctx);
    }

    // Dotted name (e.g. Math.max) → global function call.
    if fname.contains('.') {
        return JsExpr::Call {
            callee: Box::new(build_dotted_path(fname)),
            args: lower_exprs(args, ctx),
        };
    }

    // Free function call.
    JsExpr::Call {
        callee: Box::new(JsExpr::Var(fname.to_string())),
        args: lower_exprs(args, ctx),
    }
}

/// Build a JsExpr for a dotted path like `"Math.max"` → `Field(Var("Math"), "max")`.
fn build_dotted_path(name: &str) -> JsExpr {
    let mut parts = name.split('.');
    let first = parts.next().unwrap();
    let mut expr = JsExpr::Var(first.to_string());
    for part in parts {
        expr = JsExpr::Field {
            object: Box::new(expr),
            field: part.to_string(),
        };
    }
    expr
}

// ---------------------------------------------------------------------------
// Builtin operator expansion
// ---------------------------------------------------------------------------

/// Helper: binary op from two args.
fn bin_op(op: BinOp, args: &[Expr], ctx: &LowerCtx) -> JsExpr {
    JsExpr::Binary {
        op,
        lhs: Box::new(lower_expr(&args[0], ctx)),
        rhs: Box::new(lower_expr(&args[1], ctx)),
    }
}

/// Helper: unary op from one arg.
fn unary_op(op: UnaryOp, args: &[Expr], ctx: &LowerCtx) -> JsExpr {
    JsExpr::Unary {
        op,
        expr: Box::new(lower_expr(&args[0], ctx)),
    }
}

/// Helper: `Math.method(arg0)`.
fn math_call_1(method: &str, args: &[Expr], ctx: &LowerCtx) -> JsExpr {
    JsExpr::Call {
        callee: Box::new(build_dotted_path(&format!("Math.{method}"))),
        args: vec![lower_expr(&args[0], ctx)],
    }
}

/// Helper: `Math.method(arg0, arg1)`.
fn math_call_2(method: &str, args: &[Expr], ctx: &LowerCtx) -> JsExpr {
    JsExpr::Call {
        callee: Box::new(build_dotted_path(&format!("Math.{method}"))),
        args: vec![lower_expr(&args[0], ctx), lower_expr(&args[1], ctx)],
    }
}

/// Helper: `receiver.method(call_args...)` where receiver and call_args are
/// selected from the lowered args by index.
fn method_call(
    receiver_idx: usize,
    method: &str,
    arg_indices: &[usize],
    args: &[Expr],
    ctx: &LowerCtx,
) -> JsExpr {
    let receiver = lower_expr(&args[receiver_idx], ctx);
    let call_args: Vec<JsExpr> = arg_indices
        .iter()
        .map(|&i| lower_expr(&args[i], ctx))
        .collect();
    JsExpr::Call {
        callee: Box::new(JsExpr::Field {
            object: Box::new(receiver),
            field: method.to_string(),
        }),
        args: call_args,
    }
}

/// Expand a `builtin.{op_name}` call into the appropriate `JsExpr`.
fn lower_builtin(op_name: &str, args: &[Expr], ctx: &LowerCtx) -> JsExpr {
    match op_name {
        // --- Arithmetic binary ---
        "add_f64" | "add_f32" | "add_i32" | "add_i64" | "concat_str" => {
            bin_op(BinOp::Add, args, ctx)
        }
        "sub_f64" | "sub_f32" | "sub_i32" | "sub_i64" => bin_op(BinOp::Sub, args, ctx),
        "mul_f64" | "mul_f32" | "mul_i32" | "mul_i64" => bin_op(BinOp::Mul, args, ctx),
        "div_f64" | "div_f32" | "div_i32" | "div_i64" => bin_op(BinOp::Div, args, ctx),
        "rem_f64" | "rem_f32" | "rem_i32" | "rem_i64" => bin_op(BinOp::Rem, args, ctx),

        // --- Bitwise binary ---
        "shl_i32" => bin_op(BinOp::Shl, args, ctx),
        "shr_i32" => bin_op(BinOp::Shr, args, ctx),
        "bitand_i32" => bin_op(BinOp::BitAnd, args, ctx),
        "bitor_i32" => bin_op(BinOp::BitOr, args, ctx),
        "bitxor_i32" => bin_op(BinOp::BitXor, args, ctx),

        // --- Unary ops ---
        "neg_f64" | "neg_f32" | "neg_i32" | "neg_i64" => unary_op(UnaryOp::Neg, args, ctx),
        "bitnot_i32" => unary_op(UnaryOp::BitNot, args, ctx),

        // --- Boolean NOT ---
        "not_bool" => JsExpr::Not(Box::new(lower_expr(&args[0], ctx))),

        // --- Bitwise bool workarounds (TS2447) ---
        "bitand_bool_i32" => JsExpr::Binary {
            op: BinOp::BitAnd,
            lhs: Box::new(JsExpr::Cast {
                expr: Box::new(lower_expr(&args[0], ctx)),
                ty: Type::Float(64),
                kind: CastKind::Coerce,
            }),
            rhs: Box::new(JsExpr::Cast {
                expr: Box::new(lower_expr(&args[1], ctx)),
                ty: Type::Float(64),
                kind: CastKind::Coerce,
            }),
        },
        "bitor_bool_i32" => JsExpr::Binary {
            op: BinOp::BitOr,
            lhs: Box::new(JsExpr::Cast {
                expr: Box::new(lower_expr(&args[0], ctx)),
                ty: Type::Float(64),
                kind: CastKind::Coerce,
            }),
            rhs: Box::new(JsExpr::Cast {
                expr: Box::new(lower_expr(&args[1], ctx)),
                ty: Type::Float(64),
                kind: CastKind::Coerce,
            }),
        },
        "bitxor_bool_i32" => JsExpr::Cmp {
            kind: CmpKind::Ne,
            lhs: Box::new(lower_expr(&args[0], ctx)),
            rhs: Box::new(lower_expr(&args[1], ctx)),
        },

        // --- Math 1-arg ---
        "sin_f64" => math_call_1("sin", args, ctx),
        "cos_f64" => math_call_1("cos", args, ctx),
        "tan_f64" => math_call_1("tan", args, ctx),
        "asin_f64" => math_call_1("asin", args, ctx),
        "acos_f64" => math_call_1("acos", args, ctx),
        "atan_f64" => math_call_1("atan", args, ctx),
        "sqrt_f64" => math_call_1("sqrt", args, ctx),
        "exp_f64" => math_call_1("exp", args, ctx),
        "ln_f64" => math_call_1("log", args, ctx),
        "log2_f64" => math_call_1("log2", args, ctx),
        "log10_f64" => math_call_1("log10", args, ctx),
        "abs_f64" => math_call_1("abs", args, ctx),
        "floor_f64" => math_call_1("floor", args, ctx),
        "ceil_f64" => math_call_1("ceil", args, ctx),
        "round_f64" => math_call_1("round", args, ctx),
        "trunc_f64" => math_call_1("trunc", args, ctx),
        "sign_f64" => math_call_1("sign", args, ctx),

        // --- Math 2-arg ---
        "atan2_f64" => math_call_2("atan2", args, ctx),
        "pow_f64" => math_call_2("pow", args, ctx),
        "hypot_f64" => math_call_2("hypot", args, ctx),
        "min_f64" => math_call_2("min", args, ctx),
        "max_f64" => math_call_2("max", args, ctx),

        // --- String operations ---
        "string_length_str" => JsExpr::Field {
            object: Box::new(lower_expr(&args[0], ctx)),
            field: "length".to_string(),
        },
        "string_upper_str" => method_call(0, "toUpperCase", &[], args, ctx),
        "string_lower_str" => method_call(0, "toLowerCase", &[], args, ctx),
        "string_char_at_str" => method_call(0, "charAt", &[1], args, ctx),
        "string_index_of_str" => method_call(1, "indexOf", &[0], args, ctx),
        "string_slice_str" => method_call(0, "slice", &[1, 2], args, ctx),
        "string_split_str" => method_call(0, "split", &[1], args, ctx),
        "string_char_code_at_str" => method_call(0, "charCodeAt", &[1], args, ctx),
        "string_repeat_str" => method_call(0, "repeat", &[1], args, ctx),
        "string_replace_first_str" => method_call(0, "replace", &[1, 2], args, ctx),
        "string_trim_str" => method_call(0, "trim", &[], args, ctx),

        // --- Array operations ---
        "array_length_arr" => JsExpr::Field {
            object: Box::new(lower_expr(&args[0], ctx)),
            field: "length".to_string(),
        },
        "array_contains_arr" => method_call(0, "includes", &[1], args, ctx),

        // --- Other ---
        "chr_f64" => JsExpr::Call {
            callee: Box::new(build_dotted_path("String.fromCharCode")),
            args: vec![lower_expr(&args[0], ctx)],
        },

        // --- Fallback: unknown builtin ---
        _ => JsExpr::Call {
            callee: Box::new(build_dotted_path(&format!("builtin.{op_name}"))),
            args: lower_exprs(args, ctx),
        },
    }
}
