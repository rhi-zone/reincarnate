//! Core AST → JS AST mechanical lowering pass.
//!
//! Converts the engine-agnostic `Stmt`/`Expr` tree into `JsStmt`/`JsExpr`
//! with a 1:1 structural mapping. Engine-specific rewrites (scope-lookup
//! resolution, SystemCall → native JS constructs, etc.) are handled by a
//! separate post-lowering rewrite pass (e.g. `rewrites::flash`).

use reincarnate_core::ir::ast::{AstFunction, Expr, Stmt};
use reincarnate_core::ir::module::Module;
use reincarnate_core::ir::{CastKind, CmpKind, Type};

/// Collect the names of all runtime functions whose IR signature begins with
/// `_rt: Type::Instance(module.runtime_type_id)`.
///
/// These are the "stateful" runtime calls: the frontend prepends the runtime
/// handle as the first IR argument, and the backend lowers each call as
/// `arg0.fname(rest_args)` via [`LowerCtx::stateful_names`].
pub fn collect_stateful_runtime_names(module: &Module) -> std::collections::BTreeSet<String> {
    let Some(rt_tid) = module.runtime_type_id else {
        return Default::default();
    };
    let rt_ty = Type::Instance(rt_tid);
    module
        .runtime_registry
        .iter()
        .filter(|(_, &fid)| module.functions[fid].sig.params.first() == Some(&rt_ty))
        .map(|(name, _)| name.clone())
        .collect()
}

use crate::js_ast::{BinOp, UnaryOp};

use crate::js_ast::{JsExpr, JsFunction, JsStmt};

// ---------------------------------------------------------------------------
// Lowering context
// ---------------------------------------------------------------------------

/// Context for the lowering pass.
pub struct LowerCtx {
    /// Self parameter name — the IR parameter that maps to `this`.
    pub self_param_name: Option<String>,
    /// Names of stateful runtime functions whose IR signature begins with
    /// `_rt: GameRuntime`.  Calls to these functions are lowered as
    /// `arg0.fname(rest_args)` (i.e. the first IR arg becomes the receiver).
    pub stateful_names: std::collections::BTreeSet<String>,
    /// When true, `Expr::Var("_rt")` is lowered to `this._rt` rather than the
    /// bare `_rt` identifier.  Set by class-method emit paths where the runtime
    /// handle lives as an instance field instead of an explicit parameter.
    pub rt_via_this: bool,
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
        overloads: vec![],
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
            // Class methods carry the runtime as an instance field rather than
            // a JS parameter, so the IR `_rt` value must be lowered as
            // `this._rt` to keep the reference well-formed.
            if ctx.rt_via_this && name == "_rt" {
                return JsExpr::Field {
                    object: Box::new(JsExpr::This),
                    field: "_rt".to_string(),
                };
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
    // Core builtin operator expansion — try direct name dispatch first.
    if let Some(expr) = lower_builtin_opt(fname, args, ctx) {
        return expr;
    }

    // Stateful runtime call — emit as `arg0.fname(rest_args)`.
    // The frontend prepends the runtime handle (`_rt` or `this._rt`) as the
    // first IR argument for these calls; we surface that as the JS receiver.
    if ctx.stateful_names.contains(fname) {
        let mut lowered_args = lower_exprs(args, ctx);
        if !lowered_args.is_empty() {
            let receiver = lowered_args.remove(0);
            return JsExpr::Call {
                callee: Box::new(JsExpr::Field {
                    object: Box::new(receiver),
                    field: fname.to_string(),
                }),
                args: lowered_args,
            };
        }
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

/// Recursively collect args for chained calls of `func_name`, flattening them
/// into a single flat argument list.  For example, `max_f64(max_f64(a, b), c)`
/// becomes `[a, b, c]` so the backend can emit `Math.max(a, b, c)`.
fn collect_chained_args(func_name: &str, args: &[Expr], ctx: &LowerCtx) -> Vec<JsExpr> {
    let mut result = Vec::new();
    for arg in args {
        if let Expr::Call { func, args: inner } = arg {
            if func == func_name {
                result.extend(collect_chained_args(func_name, inner, ctx));
                continue;
            }
        }
        result.push(lower_expr(arg, ctx));
    }
    result
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

/// Try to expand a core builtin name into the appropriate `JsExpr`.
/// Returns `None` if the name is not a known core builtin.
fn lower_builtin_opt(op_name: &str, args: &[Expr], ctx: &LowerCtx) -> Option<JsExpr> {
    match op_name {
        // --- Arithmetic binary ---
        "add_f64" | "add_f32" | "add_i32" | "add_i64" | "concat_str" => {
            Some(bin_op(BinOp::Add, args, ctx))
        }
        "sub_f64" | "sub_f32" | "sub_i32" | "sub_i64" => Some(bin_op(BinOp::Sub, args, ctx)),
        "mul_f64" | "mul_f32" | "mul_i32" | "mul_i64" => Some(bin_op(BinOp::Mul, args, ctx)),
        "div_f64" | "div_f32" | "div_i32" | "div_i64" => Some(bin_op(BinOp::Div, args, ctx)),
        "rem_f64" | "rem_f32" | "rem_i32" | "rem_i64" => Some(bin_op(BinOp::Rem, args, ctx)),

        // --- Bitwise binary ---
        "shl_i32" => Some(bin_op(BinOp::Shl, args, ctx)),
        "shr_i32" => Some(bin_op(BinOp::Shr, args, ctx)),
        "bitand_i32" => Some(bin_op(BinOp::BitAnd, args, ctx)),
        "bitor_i32" => Some(bin_op(BinOp::BitOr, args, ctx)),
        "bitxor_i32" => Some(bin_op(BinOp::BitXor, args, ctx)),

        // --- Unary ops ---
        "neg_f64" | "neg_f32" | "neg_i32" | "neg_i64" => Some(unary_op(UnaryOp::Neg, args, ctx)),
        "bitnot_i32" => Some(unary_op(UnaryOp::BitNot, args, ctx)),

        // --- Boolean NOT ---
        "not_bool" => Some(JsExpr::Not(Box::new(lower_expr(&args[0], ctx)))),

        // --- Bitwise bool workarounds (TS2447) ---
        "bitand_bool_i32" => Some(JsExpr::Binary {
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
        }),
        "bitor_bool_i32" => Some(JsExpr::Binary {
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
        }),
        "bitxor_bool_i32" => Some(JsExpr::Cmp {
            kind: CmpKind::Ne,
            lhs: Box::new(lower_expr(&args[0], ctx)),
            rhs: Box::new(lower_expr(&args[1], ctx)),
        }),

        // --- Math 1-arg ---
        "sin_f64" => Some(math_call_1("sin", args, ctx)),
        "cos_f64" => Some(math_call_1("cos", args, ctx)),
        "tan_f64" => Some(math_call_1("tan", args, ctx)),
        "asin_f64" => Some(math_call_1("asin", args, ctx)),
        "acos_f64" => Some(math_call_1("acos", args, ctx)),
        "atan_f64" => Some(math_call_1("atan", args, ctx)),
        "sqrt_f64" => Some(math_call_1("sqrt", args, ctx)),
        "exp_f64" => Some(math_call_1("exp", args, ctx)),
        "ln_f64" => Some(math_call_1("log", args, ctx)),
        "log2_f64" => Some(math_call_1("log2", args, ctx)),
        "log10_f64" => Some(math_call_1("log10", args, ctx)),
        "abs_f64" => Some(math_call_1("abs", args, ctx)),
        "floor_f64" => Some(math_call_1("floor", args, ctx)),
        "ceil_f64" => Some(math_call_1("ceil", args, ctx)),
        "round_f64" => Some(math_call_1("round", args, ctx)),
        "trunc_f64" => Some(math_call_1("trunc", args, ctx)),
        "sign_f64" => Some(math_call_1("sign", args, ctx)),

        // --- Math 2-arg ---
        "atan2_f64" => Some(math_call_2("atan2", args, ctx)),
        "pow_f64" => Some(math_call_2("pow", args, ctx)),
        "hypot_f64" => Some(math_call_2("hypot", args, ctx)),
        "min_f64" => Some(JsExpr::Call {
            callee: Box::new(build_dotted_path("Math.min")),
            args: collect_chained_args("min_f64", args, ctx),
        }),
        "max_f64" => Some(JsExpr::Call {
            callee: Box::new(build_dotted_path("Math.max")),
            args: collect_chained_args("max_f64", args, ctx),
        }),

        // --- String operations ---
        "string_length_str" => Some(JsExpr::Field {
            object: Box::new(lower_expr(&args[0], ctx)),
            field: "length".to_string(),
        }),
        "string_upper_str" => Some(method_call(0, "toUpperCase", &[], args, ctx)),
        "string_lower_str" => Some(method_call(0, "toLowerCase", &[], args, ctx)),
        "string_char_at_str" => Some(method_call(0, "charAt", &[1], args, ctx)),
        "string_index_of_str" => Some(method_call(1, "indexOf", &[0], args, ctx)),
        "string_slice_str" => Some(method_call(0, "slice", &[1, 2], args, ctx)),
        "string_split_str" => Some(method_call(0, "split", &[1], args, ctx)),
        "string_join_arr" => Some(method_call(0, "join", &[1], args, ctx)),
        "string_char_code_at_str" => Some(method_call(0, "charCodeAt", &[1], args, ctx)),
        // string_byte_at_rt(str, index0) -> str.charCodeAt(index0) || 0
        // GML string_byte_at returns 0 for out-of-range; charCodeAt returns NaN which is falsy.
        "string_byte_at_rt" => {
            let char_code = method_call(0, "charCodeAt", &[1], args, ctx);
            Some(JsExpr::LogicalOr {
                lhs: Box::new(char_code),
                rhs: Box::new(JsExpr::Literal(
                    reincarnate_core::ir::value::Constant::Float(0.0),
                )),
            })
        }
        "string_repeat_str" => Some(method_call(0, "repeat", &[1], args, ctx)),
        "string_replace_first_str" => Some(method_call(0, "replace", &[1, 2], args, ctx)),
        "string_trim_str" => Some(method_call(0, "trim", &[], args, ctx)),

        // --- Array operations ---
        "array_length_arr" => Some(JsExpr::Field {
            object: Box::new(lower_expr(&args[0], ctx)),
            field: "length".to_string(),
        }),
        "array_contains_arr" => Some(method_call(0, "includes", &[1], args, ctx)),
        // array_pop_arr(arr) -> arr.pop()
        "array_pop_arr" => Some(method_call(0, "pop", &[], args, ctx)),
        // array_delete_arr(arr, index, count) -> arr.splice(index, count)
        "array_delete_arr" => Some(method_call(0, "splice", &[1, 2], args, ctx)),
        // array_insert_arr(arr, index, val) -> arr.splice(index, 0, val)
        "array_insert_arr" => {
            let receiver = lower_expr(&args[0], ctx);
            let index = lower_expr(&args[1], ctx);
            let zero = JsExpr::Literal(reincarnate_core::ir::value::Constant::Float(0.0));
            let val = lower_expr(&args[2], ctx);
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Field {
                    object: Box::new(receiver),
                    field: "splice".to_string(),
                }),
                args: vec![index, zero, val],
            })
        }
        // array_resize_arr(arr, newSize) -> arr.length = newSize
        "array_resize_arr" => Some(JsExpr::Assign {
            lhs: Box::new(JsExpr::Field {
                object: Box::new(lower_expr(&args[0], ctx)),
                field: "length".to_string(),
            }),
            rhs: Box::new(lower_expr(&args[1], ctx)),
        }),
        // array_get_index_arr(arr, value) -> arr.indexOf(value)
        "array_get_index_arr" => Some(method_call(0, "indexOf", &[1], args, ctx)),
        // array_sort_arr(arr, ascending) -> arr.sort((a, b) => ascending ? a - b : b - a)
        // .sort() returns the sorted array (same ref); GML array_sort returns void so the
        // result is discarded, but the call expression is well-typed either way.
        "array_sort_arr" => {
            let arr = lower_expr(&args[0], ctx);
            let ascending = lower_expr(&args[1], ctx);
            let a_minus_b = JsExpr::Binary {
                op: BinOp::Sub,
                lhs: Box::new(JsExpr::Var("a".to_string())),
                rhs: Box::new(JsExpr::Var("b".to_string())),
            };
            let b_minus_a = JsExpr::Binary {
                op: BinOp::Sub,
                lhs: Box::new(JsExpr::Var("b".to_string())),
                rhs: Box::new(JsExpr::Var("a".to_string())),
            };
            let comparator = JsExpr::ArrowFunction {
                params: vec![
                    ("a".to_string(), Type::Unknown),
                    ("b".to_string(), Type::Unknown),
                ],
                return_ty: Type::Float(64),
                body: vec![JsStmt::Return(Some(JsExpr::Ternary {
                    cond: Box::new(ascending),
                    then_val: Box::new(a_minus_b),
                    else_val: Box::new(b_minus_a),
                }))],
                has_rest_param: false,
                cast_as: None,
                infer_param_types: true,
            };
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Field {
                    object: Box::new(arr),
                    field: "sort".to_string(),
                }),
                args: vec![comparator],
            })
        }
        // array_unique_arr(arr) -> [...new Set(arr)]
        "array_unique_arr" => {
            let arr = lower_expr(&args[0], ctx);
            Some(JsExpr::ArrayInit(vec![JsExpr::Spread(Box::new(
                JsExpr::New {
                    callee: Box::new(JsExpr::Var("Set".to_string())),
                    args: vec![arr],
                },
            ))]))
        }

        // --- Other ---
        "chr_f64" => Some(JsExpr::Call {
            callee: Box::new(build_dotted_path("String.fromCharCode")),
            args: vec![lower_expr(&args[0], ctx)],
        }),
        "to_number_unknown" | "to_number_str" => Some(JsExpr::Call {
            callee: Box::new(JsExpr::Var("Number".to_string())),
            args: vec![lower_expr(&args[0], ctx)],
        }),
        "to_string_unknown" => Some(JsExpr::Call {
            callee: Box::new(JsExpr::Var("String".to_string())),
            args: vec![lower_expr(&args[0], ctx)],
        }),
        "to_i32_f64" => Some(JsExpr::Binary {
            op: BinOp::BitOr,
            lhs: Box::new(lower_expr(&args[0], ctx)),
            rhs: Box::new(JsExpr::Literal(
                reincarnate_core::ir::value::Constant::Float(0.0),
            )),
        }),
        "to_u32_f64" => Some(JsExpr::Binary {
            op: BinOp::Ushr,
            lhs: Box::new(lower_expr(&args[0], ctx)),
            rhs: Box::new(JsExpr::Literal(
                reincarnate_core::ir::value::Constant::Float(0.0),
            )),
        }),

        // is_nan_f64: (Float64) -> Bool
        // emit as: Number.isNaN(x)
        "is_nan_f64" => Some(JsExpr::Call {
            callee: Box::new(build_dotted_path("Number.isNaN")),
            args: vec![lower_expr(&args[0], ctx)],
        }),

        // is_infinite_f64: (Float64) -> Bool
        // emit as: !Number.isFinite(x) && !Number.isNaN(x)
        "is_infinite_f64" => {
            let arg = lower_expr(&args[0], ctx);
            let not_finite = JsExpr::Not(Box::new(JsExpr::Call {
                callee: Box::new(build_dotted_path("Number.isFinite")),
                args: vec![arg.clone()],
            }));
            let not_nan = JsExpr::Not(Box::new(JsExpr::Call {
                callee: Box::new(build_dotted_path("Number.isNaN")),
                args: vec![arg],
            }));
            Some(JsExpr::LogicalAnd {
                lhs: Box::new(not_finite),
                rhs: Box::new(not_nan),
            })
        }

        // is_struct_unknown: (Unknown) -> Bool
        // emit as: typeof x === "object" && x != null && !Array.isArray(x)
        "is_struct_unknown" => {
            let arg = lower_expr(&args[0], ctx);
            let is_object = JsExpr::Cmp {
                kind: CmpKind::Eq,
                lhs: Box::new(JsExpr::TypeOf(Box::new(arg.clone()))),
                rhs: Box::new(JsExpr::Literal(
                    reincarnate_core::ir::value::Constant::String("object".to_string()),
                )),
            };
            let not_null = JsExpr::Cmp {
                kind: CmpKind::Ne,
                lhs: Box::new(arg.clone()),
                rhs: Box::new(JsExpr::Literal(reincarnate_core::ir::value::Constant::Null)),
            };
            let not_array = JsExpr::Not(Box::new(JsExpr::Call {
                callee: Box::new(build_dotted_path("Array.isArray")),
                args: vec![arg],
            }));
            Some(JsExpr::LogicalAnd {
                lhs: Box::new(JsExpr::LogicalAnd {
                    lhs: Box::new(is_object),
                    rhs: Box::new(not_null),
                }),
                rhs: Box::new(not_array),
            })
        }

        // is_numeric_unknown: (Unknown) -> Bool
        // emit as: !isNaN(Number(val))
        "is_numeric_unknown" => {
            let arg = lower_expr(&args[0], ctx);
            let number_call = JsExpr::Call {
                callee: Box::new(JsExpr::Var("Number".to_string())),
                args: vec![arg],
            };
            let is_nan_call = JsExpr::Call {
                callee: Box::new(build_dotted_path("Number.isNaN")),
                args: vec![number_call],
            };
            Some(JsExpr::Not(Box::new(is_nan_call)))
        }

        // typeof_gml: (Unknown) -> String
        // GML typeof() — returns the GML type name for a value.
        // emit as: val === undefined ? "undefined" : val === null ? "null" :
        //   typeof val === "boolean" ? "bool" : typeof val === "string" ? "string" :
        //   Array.isArray(val) ? "array" : typeof val === "number" ? "number" :
        //   typeof val === "object" ? "struct" : "unknown"
        "typeof_gml" => {
            let arg = lower_expr(&args[0], ctx);
            let str_lit = |s: &str| {
                Box::new(JsExpr::Literal(
                    reincarnate_core::ir::value::Constant::String(s.to_string()),
                ))
            };
            let typeof_eq = |a: JsExpr, s: &str| JsExpr::Cmp {
                kind: CmpKind::Eq,
                lhs: Box::new(JsExpr::TypeOf(Box::new(a))),
                rhs: Box::new(JsExpr::Literal(
                    reincarnate_core::ir::value::Constant::String(s.to_string()),
                )),
            };
            // Build innermost to outermost: "unknown" fallback
            // typeof val === "object" ? "struct" : "unknown"
            let t7 = JsExpr::Ternary {
                cond: Box::new(typeof_eq(arg.clone(), "object")),
                then_val: str_lit("struct"),
                else_val: str_lit("unknown"),
            };
            // typeof val === "number" ? "number" : ...
            let t6 = JsExpr::Ternary {
                cond: Box::new(typeof_eq(arg.clone(), "number")),
                then_val: str_lit("number"),
                else_val: Box::new(t7),
            };
            // Array.isArray(val) ? "array" : ...
            let t5 = JsExpr::Ternary {
                cond: Box::new(JsExpr::Call {
                    callee: Box::new(build_dotted_path("Array.isArray")),
                    args: vec![arg.clone()],
                }),
                then_val: str_lit("array"),
                else_val: Box::new(t6),
            };
            // typeof val === "string" ? "string" : ...
            let t4 = JsExpr::Ternary {
                cond: Box::new(typeof_eq(arg.clone(), "string")),
                then_val: str_lit("string"),
                else_val: Box::new(t5),
            };
            // typeof val === "boolean" ? "bool" : ...
            let t3 = JsExpr::Ternary {
                cond: Box::new(typeof_eq(arg.clone(), "boolean")),
                then_val: str_lit("bool"),
                else_val: Box::new(t4),
            };
            // val === null ? "null" : ...
            let t2 = JsExpr::Ternary {
                cond: Box::new(JsExpr::Cmp {
                    kind: CmpKind::Eq,
                    lhs: Box::new(arg.clone()),
                    rhs: Box::new(JsExpr::Literal(reincarnate_core::ir::value::Constant::Null)),
                }),
                then_val: str_lit("null"),
                else_val: Box::new(t3),
            };
            // val === undefined ? "undefined" : ...
            let t1 = JsExpr::Ternary {
                cond: Box::new(JsExpr::Cmp {
                    kind: CmpKind::Eq,
                    lhs: Box::new(arg),
                    rhs: Box::new(JsExpr::Var("undefined".to_string())),
                }),
                then_val: str_lit("undefined"),
                else_val: Box::new(t2),
            };
            Some(t1)
        }

        // variable_struct_exists_rt: (Unknown, String) -> Bool
        // emit as: struct != null && Object.prototype.hasOwnProperty.call(struct, name)
        "variable_struct_exists_rt" => {
            let s = lower_expr(&args[0], ctx);
            let name = lower_expr(&args[1], ctx);
            let not_null = JsExpr::LooseNe {
                lhs: Box::new(s.clone()),
                rhs: Box::new(JsExpr::Literal(reincarnate_core::ir::value::Constant::Null)),
            };
            let has_own = JsExpr::Call {
                callee: Box::new(build_dotted_path("Object.prototype.hasOwnProperty.call")),
                args: vec![s, name],
            };
            Some(JsExpr::LogicalAnd {
                lhs: Box::new(not_null),
                rhs: Box::new(has_own),
            })
        }

        // variable_struct_get_rt: (Unknown, String) -> Unknown
        // emit as: struct != null ? struct[name] : undefined
        "variable_struct_get_rt" => {
            let s = lower_expr(&args[0], ctx);
            let name = lower_expr(&args[1], ctx);
            let not_null = JsExpr::LooseNe {
                lhs: Box::new(s.clone()),
                rhs: Box::new(JsExpr::Literal(reincarnate_core::ir::value::Constant::Null)),
            };
            let index = JsExpr::Index {
                collection: Box::new(s),
                index: Box::new(name),
            };
            Some(JsExpr::Ternary {
                cond: Box::new(not_null),
                then_val: Box::new(index),
                else_val: Box::new(JsExpr::Var("undefined".to_string())),
            })
        }

        // variable_struct_names_count_rt: (Unknown) -> Float64
        // emit as: struct != null ? Object.keys(struct).length : 0
        "variable_struct_names_count_rt" => {
            let s = lower_expr(&args[0], ctx);
            let not_null = JsExpr::LooseNe {
                lhs: Box::new(s.clone()),
                rhs: Box::new(JsExpr::Literal(reincarnate_core::ir::value::Constant::Null)),
            };
            let keys_call = JsExpr::Call {
                callee: Box::new(build_dotted_path("Object.keys")),
                args: vec![s],
            };
            let length = JsExpr::Field {
                object: Box::new(keys_call),
                field: "length".to_string(),
            };
            Some(JsExpr::Ternary {
                cond: Box::new(not_null),
                then_val: Box::new(length),
                else_val: Box::new(JsExpr::Literal(
                    reincarnate_core::ir::value::Constant::Float(0.0),
                )),
            })
        }

        // variable_struct_get_names_rt: (Unknown) -> Array<String>
        // emit as: struct != null ? Object.keys(struct) : []
        "variable_struct_get_names_rt" => {
            let s = lower_expr(&args[0], ctx);
            let not_null = JsExpr::LooseNe {
                lhs: Box::new(s.clone()),
                rhs: Box::new(JsExpr::Literal(reincarnate_core::ir::value::Constant::Null)),
            };
            let keys_call = JsExpr::Call {
                callee: Box::new(build_dotted_path("Object.keys")),
                args: vec![s],
            };
            Some(JsExpr::Ternary {
                cond: Box::new(not_null),
                then_val: Box::new(keys_call),
                else_val: Box::new(JsExpr::ArrayInit(vec![])),
            })
        }

        // variable_struct_set_rt: (Unknown, String, Unknown) -> Void
        // emit as: struct[name] = val
        "variable_struct_set_rt" => {
            let s = lower_expr(&args[0], ctx);
            let name = lower_expr(&args[1], ctx);
            let val = lower_expr(&args[2], ctx);
            Some(JsExpr::Assign {
                lhs: Box::new(JsExpr::Index {
                    collection: Box::new(s),
                    index: Box::new(name),
                }),
                rhs: Box::new(val),
            })
        }

        // string_digits_rt(str) -> str.replace(/\D/g, "")
        "string_digits_rt" => Some(JsExpr::Call {
            callee: Box::new(JsExpr::Field {
                object: Box::new(lower_expr(&args[0], ctx)),
                field: "replace".to_string(),
            }),
            args: vec![
                JsExpr::Regex(r"/\D/g".to_string()),
                JsExpr::Literal(reincarnate_core::ir::value::Constant::String(String::new())),
            ],
        }),

        // string_letters_rt(str) -> str.replace(/[^a-zA-Z]/g, "")
        "string_letters_rt" => Some(JsExpr::Call {
            callee: Box::new(JsExpr::Field {
                object: Box::new(lower_expr(&args[0], ctx)),
                field: "replace".to_string(),
            }),
            args: vec![
                JsExpr::Regex(r"/[^a-zA-Z]/g".to_string()),
                JsExpr::Literal(reincarnate_core::ir::value::Constant::String(String::new())),
            ],
        }),

        // string_format_rt(n, tot, dec) -> (s => s.length < tot ? s.padStart(tot) : s)(n.toFixed(dec))
        // Mirrors runtime.ts: const s = n.toFixed(dec); return s.length < tot ? s.padStart(tot) : s;
        "string_format_rt" => {
            let n = lower_expr(&args[0], ctx);
            let tot = lower_expr(&args[1], ctx);
            let dec = lower_expr(&args[2], ctx);
            let s_fixed = JsExpr::Call {
                callee: Box::new(JsExpr::Field {
                    object: Box::new(n),
                    field: "toFixed".to_string(),
                }),
                args: vec![dec],
            };
            let s_len = JsExpr::Field {
                object: Box::new(JsExpr::Var("s".to_string())),
                field: "length".to_string(),
            };
            let s_lt_tot = JsExpr::Cmp {
                kind: CmpKind::Lt,
                lhs: Box::new(s_len),
                rhs: Box::new(tot.clone()),
            };
            let s_padstart = JsExpr::Call {
                callee: Box::new(JsExpr::Field {
                    object: Box::new(JsExpr::Var("s".to_string())),
                    field: "padStart".to_string(),
                }),
                args: vec![tot],
            };
            let ternary = JsExpr::Ternary {
                cond: Box::new(s_lt_tot),
                then_val: Box::new(s_padstart),
                else_val: Box::new(JsExpr::Var("s".to_string())),
            };
            let arrow = JsExpr::ArrowFunction {
                params: vec![("s".to_string(), Type::String)],
                return_ty: Type::String,
                body: vec![JsStmt::Return(Some(ternary))],
                has_rest_param: false,
                cast_as: None,
                infer_param_types: false,
            };
            Some(JsExpr::Call {
                callee: Box::new(arrow),
                args: vec![s_fixed],
            })
        }

        // --- Not a core builtin ---
        _ => None,
    }
}
