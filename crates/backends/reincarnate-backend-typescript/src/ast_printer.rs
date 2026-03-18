//! TypeScript AST printer.
//!
//! Prints `JsFunction` → TypeScript source. Handles all TS-specific formatting:
//! type annotations, identifier sanitization, precedence-based parenthesization.
//! Contains zero engine knowledge — all SystemCall resolution happened during
//! the `lower` pass.

use std::cell::Cell;
use std::fmt::Write;

use reincarnate_core::ir::ast::BinOp;
use reincarnate_core::ir::{CastKind, CmpKind, Constant, MethodKind, Type, UnaryOp, Visibility};

use crate::emit::sanitize_ident;
use crate::js_ast::{JsExpr, JsFunction, JsStmt};
use crate::types::ts_type;

thread_local! {
    /// When true, bare `return;` is printed as `return undefined;` to satisfy
    /// `noImplicitReturns` in non-void functions (AS3 `returnvoid` in typed functions).
    static BARE_RETURN_UNDEFINED: Cell<bool> = const { Cell::new(false) };

    /// When true, `null` literals in value positions are printed as `null!` to
    /// satisfy `strictNullChecks`.  In AS3, null is valid for any reference type;
    /// `null!` (type: `never`, assignable to everything) preserves runtime behavior.
    /// Comparison operands use `print_expr_operand` which bypasses this.
    pub(crate) static NULL_ASSERT: Cell<bool> = const { Cell::new(false) };
}

/// Unwrap nested `Cast(x, T, Coerce)` when the inner is also a Coerce to the
/// same type — collapses `String(String(x))` → `String(x)`, etc.
/// Also unwraps `Call(Var("String"), [x])` inside `Cast(_, String, Coerce)` (and
/// similarly for Number/int/uint/Boolean), since the Call is semantically
/// identical to the outer Coerce.
fn unwrap_coerce<'a>(expr: &'a JsExpr, target_ty: &Type) -> &'a JsExpr {
    match expr {
        JsExpr::Cast {
            expr: inner,
            ty,
            kind: CastKind::Coerce,
        } if ty == target_ty => unwrap_coerce(inner, target_ty),
        JsExpr::Call { callee, args } if args.len() == 1 => {
            let wrapper_name = match target_ty {
                Type::String => Some("String"),
                Type::Float(_) => Some("Number"),
                Type::Int(32) => Some("int"),
                Type::UInt(32) => Some("uint"),
                Type::Bool => Some("Boolean"),
                _ => None,
            };
            if let Some(name) = wrapper_name {
                if matches!(callee.as_ref(), JsExpr::Var(n) if n == name) {
                    return unwrap_coerce(&args[0], target_ty);
                }
            }
            expr
        }
        _ => expr,
    }
}

// ---------------------------------------------------------------------------
// Function / method printing
// ---------------------------------------------------------------------------

/// Returns `true` if the last statement in a block is a terminal statement
/// (return, throw, break, continue, or an if/else where both branches terminate).
/// Used to decide whether to append a synthetic `return 0 as any;` to avoid
/// TS2366 "Function lacks ending return statement".
pub(crate) fn ends_with_terminal(stmts: &[JsStmt]) -> bool {
    match stmts.last() {
        Some(JsStmt::Return(_)) | Some(JsStmt::Throw(_)) => true,
        Some(JsStmt::Break) | Some(JsStmt::Continue) | Some(JsStmt::LabeledBreak { .. }) => true,
        Some(JsStmt::If {
            then_body,
            else_body,
            ..
        }) => {
            !else_body.is_empty() && ends_with_terminal(then_body) && ends_with_terminal(else_body)
        }
        // A switch is terminal if it has a default case and every case body (including
        // default) ends with a terminal.  Without a default, some input value could fall
        // through without hitting any case, so it is not terminal.
        //
        // Empty case bodies (fall-through cases like `case A: case B: return X;`) are
        // terminal by convention — they produce no code and their fall-through target
        // will be checked separately.  Only non-empty bodies need to end with a terminal.
        Some(JsStmt::Switch {
            cases,
            default_body,
            ..
        }) => {
            !default_body.is_empty()
                && ends_with_terminal(default_body)
                && cases
                    .iter()
                    .all(|(_, body)| body.is_empty() || ends_with_terminal(body))
        }
        // An infinite loop (`while (true)`) is terminal if its body contains no top-level
        // `break` — i.e. there is no way to fall through to the code below it.
        // (TypeScript itself will report TS7027 for code after such a loop, which is why
        // we must not append a synthetic `return 0 as any;` in that case.)
        Some(JsStmt::Loop { body }) => !loop_body_has_break(body),
        _ => false,
    }
}

/// Returns `true` if `stmts` contain a `break` (or `continue` — but only `break` matters
/// for fall-through) at the top level of the current loop body.
/// Does NOT recurse into nested `Loop`/`While`/`For`/`ForOf` — their `break`s only exit
/// those inner loops.
fn loop_body_has_break(stmts: &[JsStmt]) -> bool {
    for stmt in stmts {
        match stmt {
            JsStmt::Break | JsStmt::LabeledBreak { .. } => return true,
            // Recurse into conditionals — a break inside an if exits the *loop*, not the if.
            JsStmt::If {
                then_body,
                else_body,
                ..
            } => {
                if loop_body_has_break(then_body) || loop_body_has_break(else_body) {
                    return true;
                }
            }
            // Do NOT recurse into nested loops — their breaks are scoped to those loops.
            JsStmt::Loop { .. }
            | JsStmt::While { .. }
            | JsStmt::For { .. }
            | JsStmt::ForOf { .. } => {}
            _ => {}
        }
    }
    false
}

/// Returns the GML implicit return literal for a given return type, or `None`
/// if no synthetic return is needed (void, any, or types where TypeScript
/// doesn't require all paths to return).
///
/// GML implicitly returns 0 when a function exits without a `return` statement.
/// The zero value is type-appropriate: `0` for numbers, `false` for booleans.
/// Dynamic (`any`) return types are excluded — TypeScript doesn't enforce
/// complete returns for `any`, so no synthetic return is needed.
fn implicit_gml_return(ty: &Type) -> Option<&'static str> {
    match ty {
        Type::Int(_) | Type::UInt(_) | Type::Float(_) => Some("0"),
        Type::Bool => Some("false"),
        _ => None,
    }
}

/// Returns `true` if the statement list contains a bare `return;` (no value).
/// Recurses into conditionals and switches but NOT into nested functions/closures.
fn has_bare_return(stmts: &[JsStmt]) -> bool {
    for stmt in stmts {
        match stmt {
            JsStmt::Return(None) => return true,
            JsStmt::If {
                then_body,
                else_body,
                ..
            } => {
                if has_bare_return(then_body) || has_bare_return(else_body) {
                    return true;
                }
            }
            JsStmt::While { body, .. }
            | JsStmt::Loop { body }
            | JsStmt::For { body, .. }
            | JsStmt::ForOf { body, .. } => {
                if has_bare_return(body) {
                    return true;
                }
            }
            JsStmt::Switch {
                cases,
                default_body,
                ..
            } => {
                for (_, case_body) in cases {
                    if has_bare_return(case_body) {
                        return true;
                    }
                }
                if has_bare_return(default_body) {
                    return true;
                }
            }
            JsStmt::Dispatch { blocks, .. } => {
                for (_, block_body) in blocks {
                    if has_bare_return(block_body) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

/// Returns the effective TypeScript return type string for a function and whether
/// bare returns should be emitted as `return undefined;`.
/// If the function has a concrete non-void return type but also contains bare
/// `return;` statements, the type is widened to `T | undefined` to satisfy
/// `noImplicitReturns`. (AS3 functions can `returnvoid` from typed functions.)
fn effective_return_type(js: &JsFunction) -> (String, bool) {
    let ret_ty = ts_type(&js.return_ty);
    let needs_undefined =
        !matches!(js.return_ty, Type::Void | Type::Dynamic) && has_bare_return(&js.body);
    if needs_undefined {
        (format!("{ret_ty} | undefined"), true)
    } else {
        (ret_ty, false)
    }
}

/// Print a standalone function.
pub fn print_function(js: &JsFunction, preamble: Option<&str>, out: &mut String) {
    let vis = visibility_prefix(js.visibility);
    let star = if js.is_generator { "*" } else { "" };
    let params = print_params(&js.params, &js.param_defaults, js.has_rest_param, false);
    let (ret_ty, bare_undefined) = effective_return_type(js);

    let _ = writeln!(
        out,
        "{vis}function{star} {}({params}): {ret_ty} {{",
        sanitize_ident(&js.name),
    );

    if let Some(pre) = preamble {
        let _ = writeln!(out, "  {pre}");
    }

    BARE_RETURN_UNDEFINED.set(bare_undefined);
    print_stmts(&js.body, out, "  ");
    BARE_RETURN_UNDEFINED.set(false);

    // GML functions implicitly return 0 when no explicit return is reached.
    // Emit a synthetic return to suppress TS2366 for concrete return types.
    // Dynamic (any) return types don't need this — TypeScript doesn't require
    // all paths to return for `any`-typed functions.
    if let Some(implicit) = implicit_gml_return(&js.return_ty) {
        if !ends_with_terminal(&js.body) {
            let _ = writeln!(out, "  return {implicit};");
        }
    }

    let _ = writeln!(out, "}}\n");
}

/// Print a class method.
///
/// `extra_first_param` is prepended to the constructor parameter list. This is
/// used by the Flash emitter to inject `_shims: FlashShims` (or
/// `readonly _shims: FlashShims` for base classes) as the first constructor
/// parameter so each game instance carries its own shim set.
pub fn print_class_method(
    js: &JsFunction,
    raw_name: &str,
    skip_self: bool,
    preamble: Option<&str>,
    is_override: bool,
    extra_first_param: Option<&str>,
    out: &mut String,
) {
    let (params, param_defaults) = if skip_self && !js.params.is_empty() {
        let defaults_offset = js.param_defaults.len().min(1);
        (&js.params[1..], &js.param_defaults[defaults_offset..])
    } else {
        (&js.params[..], &js.param_defaults[..])
    };
    let params_str = print_params(params, param_defaults, js.has_rest_param, false);
    let (ret_ty, bare_undefined) = effective_return_type(js);
    let star = if js.is_generator { "*" } else { "" };

    // cinit → static initializer block
    if raw_name == "cinit" && matches!(js.method_kind, MethodKind::StaticInit) {
        let _ = writeln!(out, "  static {{");
        print_stmts(&js.body, out, "    ");
        let _ = writeln!(out, "  }}\n");
        return;
    }

    let ov = if is_override { "override " } else { "" };
    match js.method_kind {
        MethodKind::Constructor => {
            if let Some(extra) = extra_first_param {
                let sep = if params_str.is_empty() { "" } else { ", " };
                let _ = writeln!(out, "  constructor({extra}{sep}{params_str}) {{");
            } else {
                let _ = writeln!(out, "  constructor({params_str}) {{");
            }
        }
        MethodKind::Getter => {
            let name = raw_name.strip_prefix("get_").unwrap_or(raw_name);
            let _ = writeln!(out, "  {ov}get {name}(): {ret_ty} {{");
        }
        MethodKind::Setter => {
            let name = raw_name.strip_prefix("set_").unwrap_or(raw_name);
            let _ = writeln!(out, "  {ov}set {name}({params_str}) {{");
        }
        MethodKind::Static => {
            let _ = writeln!(
                out,
                "  static {ov}{star}{}({params_str}): {ret_ty} {{",
                sanitize_ident(raw_name),
            );
        }
        _ => {
            let _ = writeln!(
                out,
                "  {ov}{star}{}({params_str}): {ret_ty} {{",
                sanitize_ident(raw_name),
            );
        }
    }

    let indent = if matches!(js.method_kind, MethodKind::Free) {
        "  "
    } else {
        "    "
    };
    if let Some(pre) = preamble {
        let _ = writeln!(out, "{indent}{pre}");
    }
    BARE_RETURN_UNDEFINED.set(bare_undefined);
    print_stmts(&js.body, out, indent);
    BARE_RETURN_UNDEFINED.set(false);

    // GML methods implicitly return 0 when no explicit return is reached.
    // Emit a synthetic return to suppress TS2366 for concrete return types.
    if let Some(implicit) = implicit_gml_return(&js.return_ty) {
        if !matches!(
            js.method_kind,
            MethodKind::Constructor | MethodKind::Setter | MethodKind::Getter
        ) && !ends_with_terminal(&js.body)
        {
            let _ = writeln!(out, "{indent}return {implicit};");
        }
    }

    let _ = writeln!(out, "  }}");
}

fn print_params(
    params: &[(String, Type)],
    defaults: &[Option<Constant>],
    has_rest_param: bool,
    infer_dynamic: bool,
) -> String {
    params
        .iter()
        .enumerate()
        .map(|(i, (name, ty))| {
            let prefix = if has_rest_param && i == params.len() - 1 {
                "..."
            } else {
                ""
            };
            let default_suffix = defaults
                .get(i)
                .and_then(|d| d.as_ref())
                .map(|c| format!(" = {}", emit_constant(c)))
                .unwrap_or_default();
            // When infer_dynamic is set and the type is Dynamic, omit `: any` so
            // TypeScript can contextually infer the type from the call site.
            // When the default constant's TypeScript type is incompatible with the declared
            // param type, widen the annotation to `any`.  This arises when type inference
            // narrows a param (e.g. to `number` from callers) but the actual default is a
            // different type (e.g. `false` for a bool default, or `0.0` for GML's missing-arg
            // sentinel on a GMLObject-typed param).
            let default_mismatches_type = defaults
                .get(i)
                .and_then(|d| d.as_ref())
                .map(|c| {
                    !matches!(
                        (c, ty),
                        // Bool default is compatible with Bool type
                        (Constant::Bool(_), Type::Bool)
                        // Numeric default is compatible with numeric types
                        | (Constant::Float(_) | Constant::Int(_) | Constant::UInt(_),
                           Type::Int(_) | Type::UInt(_) | Type::Float(_))
                        // String default is compatible with String type
                        | (Constant::String(_), Type::String)
                        // Null is compatible with Option or Dynamic
                        | (Constant::Null, Type::Option(_) | Type::Dynamic)
                        // Any default is compatible with Dynamic
                        | (_, Type::Dynamic)
                    )
                })
                .unwrap_or(false);
            let effective_ty = if default_mismatches_type {
                &Type::Dynamic
            } else {
                ty
            };
            if infer_dynamic && matches!(ty, Type::Dynamic) {
                format!("{prefix}{}{default_suffix}", sanitize_ident(name))
            } else {
                format!(
                    "{prefix}{}: {}{default_suffix}",
                    sanitize_ident(name),
                    ts_type(effective_ty)
                )
            }
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ---------------------------------------------------------------------------
// Statement printing
// ---------------------------------------------------------------------------

/// Return true if `stmts` contains a `break` (or labeled break) that exits the
/// current loop — i.e. a break at the top level of the body or directly inside
/// an `if`/`else`/`dispatch` block, but NOT inside a nested `while`/`for`/`switch`
/// (which would break the inner loop, not the outer one).
fn body_has_reachable_break(stmts: &[JsStmt]) -> bool {
    for stmt in stmts {
        match stmt {
            JsStmt::Break | JsStmt::LabeledBreak { .. } => return true,
            JsStmt::If {
                then_body,
                else_body,
                ..
            } => {
                if body_has_reachable_break(then_body) || body_has_reachable_break(else_body) {
                    return true;
                }
            }
            JsStmt::Dispatch { blocks, .. } => {
                if blocks.iter().any(|(_, b)| body_has_reachable_break(b)) {
                    return true;
                }
            }
            // Nested While/For/Switch introduce a new break scope — breaks inside
            // them exit the inner loop, not the current one. Don't recurse.
            _ => {}
        }
    }
    false
}

fn print_stmts(stmts: &[JsStmt], out: &mut String, indent: &str) {
    for stmt in stmts {
        print_stmt(stmt, out, indent);
        // Stop after a terminal statement — anything that follows is unreachable
        // and would trigger TS7027. Throw and Return are always unconditional exits.
        // A `while(true)` with no reachable break is also an unconditional infinite
        // loop — TypeScript flags code after it as TS7027.
        let is_terminal = match stmt {
            JsStmt::Throw(_) | JsStmt::Return(_) => true,
            JsStmt::While {
                cond: JsExpr::Literal(Constant::Bool(true)),
                body,
            } => !body_has_reachable_break(body),
            // `Loop` nodes emit as `while (true) { ... }` — same rule applies.
            JsStmt::Loop { body } => !body_has_reachable_break(body),
            _ => false,
        };
        if is_terminal {
            break;
        }
    }
}

fn print_stmt(stmt: &JsStmt, out: &mut String, indent: &str) {
    match stmt {
        JsStmt::VarDecl {
            name,
            ty,
            init,
            mutable,
        } => {
            let kw = if *mutable { "let" } else { "const" };
            let name_str = sanitize_ident(name);
            match (ty, init) {
                (Some(ty), Some(init)) => {
                    // Null literal init with non-nullable type: widen to `T | null`
                    // so `strictNullChecks` accepts the assignment.
                    // When NULL_ASSERT is active, null prints as `null!` (type `never`)
                    // which is assignable to any type — no widening needed.
                    let is_null_init = matches!(init, JsExpr::Literal(Constant::Null))
                        && !matches!(ty, Type::Dynamic | Type::Option(_))
                        && !NULL_ASSERT.get();
                    let type_str = if is_null_init {
                        format!("{} | null", ts_type(ty))
                    } else {
                        ts_type(ty)
                    };
                    // Cast to the same type: strip TS assertion forms and use type
                    // annotation. Keep function-call forms (asType, Number, int, etc.).
                    if let JsExpr::Cast {
                        expr,
                        ty: cast_ty,
                        kind,
                    } = init
                    {
                        // Only strip NullableCoerce (scalar) casts: `expr as number`
                        // can safely be replaced by a type annotation when the LHS
                        // already declares the same type.
                        //
                        // Coerce (struct/enum TS assertions) must NOT be stripped:
                        // the source expression may return `unknown` (e.g. State.get),
                        // and TypeScript requires an explicit `as T` to assign
                        // `unknown → T`. A type annotation alone causes TS2322.
                        //
                        // SystemCall inits must also NOT be stripped: runtime functions
                        // like `State.get(name)` return `unknown` in TypeScript regardless
                        // of what type inference narrowed the IR value to.  Stripping
                        // `as any[]` or `as string` leaves `unknown` assigned to a typed
                        // variable → TS2322.
                        let is_syscall_init = matches!(expr.as_ref(), JsExpr::SystemCall { .. });
                        let is_ts_assertion = matches!(kind, CastKind::NullableCoerce)
                            && !matches!(ty, Type::Struct(_) | Type::Enum(_))
                            && !is_syscall_init;
                        if cast_ty == ty && is_ts_assertion {
                            let _ = writeln!(
                                out,
                                "{indent}{kw} {name_str}: {type_str} = {};",
                                print_expr(expr),
                            );
                            return;
                        }
                    }
                    let _ = writeln!(
                        out,
                        "{indent}{kw} {name_str}: {type_str} = {};",
                        print_expr(init),
                    );
                }
                (Some(ty), None) => {
                    // Use definite-assignment assertion (`!`) on `let` declarations without
                    // an initializer. GML variables are declared at function scope and may
                    // be assigned only in some branches; TypeScript's control-flow analysis
                    // is too strict for these patterns and flags TS2454 ("used before
                    // assigned"). The `!` tells TypeScript to trust that the variable will
                    // be assigned before any read on every live path.
                    let bang = if *mutable { "!" } else { "" };
                    let _ = writeln!(out, "{indent}{kw} {name_str}{bang}: {};", ts_type(ty));
                }
                (None, Some(init)) => {
                    // Cast → determine if the cast form is a TS assertion (strippable
                    // to type annotation) or a runtime call (must keep in expr).
                    if let JsExpr::Cast { expr, ty, kind } = init {
                        // Only strip NullableCoerce (scalar) casts — see comment in
                        // the (Some(ty), Some(init)) branch above.  Coerce (struct/
                        // enum TS assertions) must keep the `as T` in the expression
                        // even when there is no existing type annotation.
                        // SystemCall inits must also NOT be stripped (see above comment).
                        let is_syscall_init = matches!(expr.as_ref(), JsExpr::SystemCall { .. });
                        let is_ts_assertion = matches!(kind, CastKind::NullableCoerce)
                            && !matches!(ty, Type::Struct(_) | Type::Enum(_))
                            && !is_syscall_init;
                        if is_ts_assertion {
                            // Strip TS assertion, use type annotation + inner expr.
                            let _ = writeln!(
                                out,
                                "{indent}{kw} {name_str}: {} = {};",
                                ts_type(ty),
                                print_expr(expr),
                            );
                        } else {
                            // Keep the cast form (runtime call or Coerce assertion),
                            // add type annotation for clarity.
                            let _ = writeln!(
                                out,
                                "{indent}{kw} {name_str}: {} = {};",
                                ts_type(ty),
                                print_expr(init),
                            );
                        }
                    } else {
                        // Empty array `[]` or empty object `{}` with no explicit type
                        // annotation would cause TypeScript to infer `any[]` (TS7034) or
                        // `{}` with no index signature (TS7053 when string-indexed later).
                        // Annotate conservatively so the types are explicit.
                        let annotation = match init {
                            // Annotate all array literals (empty or non-empty) as `any[]`.
                            // Without an explicit type annotation TypeScript infers the
                            // element union (e.g. `(number | boolean)[]` for a mixed
                            // array), which then causes TS2345 at call-sites that expect
                            // `number`.  AS3 arrays are untyped at runtime, so `any[]` is
                            // the faithful representation.
                            JsExpr::ArrayInit(_) => Some("any[]"),
                            // Any object literal without an explicit type annotation is
                            // treated as a dynamic map. TypeScript would otherwise infer
                            // a narrow structural type (`{}` or `{ k: T; ... }`) with no
                            // index signature, causing TS7053 when the object is later
                            // accessed with a dynamic or `any`-typed key.
                            JsExpr::ObjectInit(_) | JsExpr::Activation => {
                                Some("Record<string, any>")
                            }
                            _ => None,
                        };
                        if let Some(ann) = annotation {
                            let _ = writeln!(
                                out,
                                "{indent}{kw} {name_str}: {ann} = {};",
                                print_expr(init),
                            );
                        } else {
                            let _ =
                                writeln!(out, "{indent}{kw} {name_str} = {};", print_expr(init),);
                        }
                    }
                }
                (None, None) => {
                    let _ = writeln!(out, "{indent}{kw} {name_str};");
                }
            }
        }

        JsStmt::Assign { target, value } => {
            let tgt = print_expr(target);
            // Any object literal assigned to an untyped variable causes TypeScript to
            // infer a narrow structural type (`{}` or `{ k: T; ... }`), which has no
            // index signature. Subsequent dynamic-key access then fails with TS7053.
            // Cast to Record<string, any> to match the VarDecl treatment.
            let val = if matches!(value, JsExpr::ObjectInit(_)) {
                format!("{} as Record<string, any>", print_expr(value))
            } else {
                print_expr(value)
            };
            if tgt.starts_with('{') {
                let _ = writeln!(out, "{indent}({tgt}) = {val};");
            } else {
                let _ = writeln!(out, "{indent}{tgt} = {val};");
            }
        }

        JsStmt::CompoundAssign { target, op, value } => {
            let tgt = print_expr(target);
            let val = print_expr(value);
            let op_str = binop_str(*op);
            if tgt.starts_with('{') {
                let _ = writeln!(out, "{indent}({tgt}) {op_str}= {val};");
            } else {
                let _ = writeln!(out, "{indent}{tgt} {op_str}= {val};");
            }
        }

        JsStmt::Expr(expr) => {
            // For a standalone SystemCall statement the result is discarded —
            // no `as any` cast is needed (see `print_expr` for inline uses).
            let s = if let JsExpr::SystemCall {
                system,
                method,
                args,
            } = expr
            {
                let args_str: Vec<_> = args.iter().map(print_expr).collect();
                let sys_ident = sanitize_ident(system);
                let safe_method = if is_valid_js_ident(method) {
                    format!(".{method}")
                } else {
                    format!("[\"{}\"]", escape_js_string(method))
                };
                format!("{sys_ident}{safe_method}({})", args_str.join(", "))
            } else {
                print_expr(expr)
            };
            if s.starts_with('{') {
                let _ = writeln!(out, "{indent}({s});");
            } else {
                let _ = writeln!(out, "{indent}{s};");
            }
        }

        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            if then_body.is_empty() && else_body.is_empty() {
                return;
            }
            let inner = format!("{indent}  ");
            if else_body.is_empty() {
                let _ = writeln!(out, "{indent}if ({}) {{", print_expr(cond));
                print_stmts(then_body, out, &inner);
                let _ = writeln!(out, "{indent}}}");
            } else {
                let _ = writeln!(out, "{indent}if ({}) {{", print_expr(cond));
                print_stmts(then_body, out, &inner);
                let _ = writeln!(out, "{indent}}} else {{");
                print_stmts(else_body, out, &inner);
                let _ = writeln!(out, "{indent}}}");
            }
        }

        JsStmt::While { cond, body } => {
            let _ = writeln!(out, "{indent}while ({}) {{", print_expr(cond));
            let inner = format!("{indent}  ");
            print_stmts(body, out, &inner);
            let _ = writeln!(out, "{indent}}}");
        }

        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            let inner = format!("{indent}  ");
            // Try to emit as proper `for (init; cond; update)` syntax when
            // init is a single VarDecl and update is a single statement.
            if init.len() == 1 && update.len() == 1 {
                if let (Some(init_str), Some(update_str)) =
                    (print_for_init(&init[0]), print_for_update(&update[0]))
                {
                    let _ = writeln!(
                        out,
                        "{indent}for ({init_str}; {}; {update_str}) {{",
                        print_expr(cond),
                    );
                    print_stmts(body, out, &inner);
                    let _ = writeln!(out, "{indent}}}");
                    return;
                }
            }
            // Fallback: emit as `init; while (cond) { body; update; }`.
            print_stmts(init, out, indent);
            let _ = writeln!(out, "{indent}while ({}) {{", print_expr(cond));
            print_stmts(body, out, &inner);
            print_stmts(update, out, &inner);
            let _ = writeln!(out, "{indent}}}");
        }

        JsStmt::Loop { body } => {
            let _ = writeln!(out, "{indent}while (true) {{");
            let inner = format!("{indent}  ");
            print_stmts(body, out, &inner);
            let _ = writeln!(out, "{indent}}}");
        }

        JsStmt::ForOf {
            binding,
            declare,
            binding_ty,
            iterable,
            body,
        } => {
            let inner = format!("{indent}  ");
            let decl = if *declare { "const " } else { "" };
            // TypeScript doesn't allow type annotations on for-of bindings
            // (TS2483), so when the binding is typed Dynamic (AVM2 for-each-in)
            // we cast the iterable to `any[]` instead.
            let iterable_str = if matches!(binding_ty, Some(Type::Dynamic)) {
                format!("({} as any[])", print_expr(iterable))
            } else {
                print_expr(iterable)
            };
            let _ = writeln!(
                out,
                "{indent}for ({decl}{} of {iterable_str}) {{",
                sanitize_ident(binding),
            );
            print_stmts(body, out, &inner);
            let _ = writeln!(out, "{indent}}}");
        }

        JsStmt::Return(expr) => {
            if let Some(e) = expr {
                let _ = writeln!(out, "{indent}return {};", print_expr(e));
            } else if BARE_RETURN_UNDEFINED.get() {
                let _ = writeln!(out, "{indent}return undefined;");
            } else {
                let _ = writeln!(out, "{indent}return;");
            }
        }

        JsStmt::Break => {
            let _ = writeln!(out, "{indent}break;");
        }

        JsStmt::Continue => {
            let _ = writeln!(out, "{indent}continue;");
        }

        JsStmt::LabeledBreak { depth } => {
            let _ = writeln!(out, "{indent}break L{depth};");
        }

        JsStmt::Dispatch { blocks, entry } => {
            // A single-block dispatch is a degenerate case: the while/switch wrapper
            // would create an infinite loop with no exit, making subsequent code
            // unreachable (TS7027). Inline the block body directly.
            if blocks.len() == 1 {
                let (_, block_stmts) = &blocks[0];
                print_stmts(block_stmts, out, indent);
            } else {
                let _ = writeln!(out, "{indent}let $block = {entry};");
                let _ = writeln!(out, "{indent}while (true) {{");
                let _ = writeln!(out, "{indent}  switch ($block) {{");
                for (idx, block_stmts) in blocks {
                    let _ = writeln!(out, "{indent}    case {idx}: {{");
                    let case_indent = format!("{indent}      ");
                    print_stmts(block_stmts, out, &case_indent);
                    let _ = writeln!(out, "{indent}    }}");
                }
                let _ = writeln!(out, "{indent}  }}");
                let _ = writeln!(out, "{indent}}}");
            }
        }

        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            // Use `as any` only when the discriminant is a literal constant —
            // constant folding can produce e.g. `switch (0.0) { case 1: ... }`
            // where the case types don't match the discriminant type.
            let cast = if matches!(value, JsExpr::Literal(_)) {
                " as any"
            } else {
                ""
            };
            let _ = writeln!(out, "{indent}switch ({}{}) {{", print_expr(value), cast);
            let case_indent = format!("{indent}  ");
            for (case_expr, case_stmts) in cases {
                let _ = writeln!(out, "{indent}  case {}:", print_expr(case_expr));
                if case_stmts.is_empty() {
                    // Fall-through: no body, no break.
                } else {
                    print_stmts(case_stmts, out, &case_indent);
                    if !ends_with_terminal(case_stmts) {
                        let _ = writeln!(out, "{indent}    break;");
                    }
                }
            }
            if !default_body.is_empty() {
                let _ = writeln!(out, "{indent}  default:");
                print_stmts(default_body, out, &case_indent);
            }
            let _ = writeln!(out, "{indent}}}");
        }

        // --- JS-specific statements ---
        JsStmt::Throw(expr) => {
            let _ = writeln!(out, "{indent}throw {};", print_expr(expr));
        }
    }
}

// ---------------------------------------------------------------------------
// For-loop header helpers
// ---------------------------------------------------------------------------

/// Print a single init statement for a `for` header (no trailing semicolon).
fn print_for_init(stmt: &JsStmt) -> Option<String> {
    match stmt {
        JsStmt::VarDecl {
            name,
            ty,
            init: Some(init),
            mutable,
        } => {
            let kw = if *mutable { "let" } else { "const" };
            let name_str = sanitize_ident(name);
            match ty {
                Some(ty) => Some(format!(
                    "{kw} {name_str}: {} = {}",
                    ts_type(ty),
                    print_expr(init)
                )),
                None => Some(format!("{kw} {name_str} = {}", print_expr(init))),
            }
        }
        JsStmt::Assign { target, value } => {
            Some(format!("{} = {}", print_expr(target), print_expr(value)))
        }
        _ => None,
    }
}

/// Print a single update statement for a `for` header (no trailing semicolon).
fn print_for_update(stmt: &JsStmt) -> Option<String> {
    match stmt {
        JsStmt::CompoundAssign { target, op, value } => Some(format!(
            "{} {}= {}",
            print_expr(target),
            binop_str(*op),
            print_expr(value),
        )),
        JsStmt::Assign { target, value } => {
            Some(format!("{} = {}", print_expr(target), print_expr(value)))
        }
        JsStmt::Expr(expr) => Some(print_expr(expr)),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Expression printing
// ---------------------------------------------------------------------------

fn print_expr(expr: &JsExpr) -> String {
    match expr {
        JsExpr::Literal(c) => {
            let s = emit_constant(c);
            // In NULL_ASSERT mode, wrap `null` as `null!` so it's assignable to any
            // non-null type under strictNullChecks.  Comparison operands go through
            // `print_expr_operand` which calls `emit_constant` directly for literals,
            // bypassing this branch.
            if matches!(c, Constant::Null) && NULL_ASSERT.get() {
                format!("{s}!")
            } else {
                s
            }
        }

        JsExpr::Var(name) => sanitize_ident(name),

        JsExpr::This => "this".into(),

        JsExpr::Binary { op, lhs, rhs } => {
            format!(
                "{} {} {}",
                print_expr_operand(lhs),
                binop_str(*op),
                print_expr_operand(rhs),
            )
        }

        JsExpr::Unary { op, expr: inner } => {
            let op_str = match op {
                UnaryOp::Neg => "-",
                UnaryOp::BitNot => "~",
            };
            format!("{op_str}{}", print_expr_operand(inner))
        }

        JsExpr::Cmp { kind, lhs, rhs } => {
            let has_null = is_null_literal(lhs) || is_null_literal(rhs);
            let op_str = if has_null {
                match kind {
                    CmpKind::Eq => "==",
                    CmpKind::Ne => "!=",
                    _ => cmp_str(*kind),
                }
            } else {
                cmp_str(*kind)
            };
            // Suppress NULL_ASSERT for comparison operands — `x === null!` would
            // give TS2367 "no overlap" since `null!` is type `never`.
            let saved = NULL_ASSERT.get();
            NULL_ASSERT.set(false);
            let result = format!(
                "{} {op_str} {}",
                print_expr_operand(lhs),
                print_expr_operand(rhs),
            );
            NULL_ASSERT.set(saved);
            result
        }

        JsExpr::Field { object, field } => {
            // Activation objects and empty object literals have no index signature;
            // accessing dynamic properties on them causes TS2339. Cast to
            // Record<string, any> so the access type-checks.
            let is_bare_obj = matches!(object.as_ref(), JsExpr::Activation)
                || matches!(object.as_ref(), JsExpr::ObjectInit(p) if p.is_empty());
            let obj_str = if is_bare_obj {
                "({} as Record<string, any>)".to_string()
            } else {
                print_expr_operand(object)
            };
            if is_valid_js_ident(field) {
                format!("{obj_str}.{field}")
            } else {
                format!("{obj_str}[\"{}\"]", escape_js_string(field))
            }
        }

        JsExpr::Index { collection, index } => {
            // An inline object literal used as a map (e.g. `{ k: v }[key]`) has no index
            // signature, causing TS7053 when the key is `any`-typed. Cast to Record so
            // the lookup is well-typed.
            let coll_str = if matches!(collection.as_ref(), JsExpr::ObjectInit(_)) {
                format!("({} as Record<string, any>)", print_expr(collection))
            } else {
                print_expr_operand(collection)
            };
            format!("{coll_str}[{}]", print_expr(index))
        }

        JsExpr::Call { callee, args } => {
            let args_str: Vec<_> = args.iter().map(print_expr).collect();
            format!("{}({})", print_expr_operand(callee), args_str.join(", "),)
        }

        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            format!(
                "{} ? {} : {}",
                print_expr_operand(cond),
                print_expr_operand(then_val),
                print_expr_operand(else_val),
            )
        }

        JsExpr::LogicalOr { lhs, rhs } => {
            format!("{} || {}", print_expr_operand(lhs), print_expr_operand(rhs),)
        }

        JsExpr::LogicalAnd { lhs, rhs } => {
            format!("{} && {}", print_expr_operand(lhs), print_expr_operand(rhs),)
        }

        JsExpr::Cast {
            expr: inner,
            ty,
            kind,
        } => {
            match (kind, ty) {
                // NullableCoerce + Struct/Enum → asType(x, Foo)! (runtime null-on-failure).
                // The `!` preserves AS3 semantics: accessing null is a runtime crash in AS3,
                // just as `!` causes a runtime TypeError in TypeScript. Under strictNullChecks
                // the result type is `T` (non-nullable) so it is usable in all value contexts.
                (CastKind::NullableCoerce, Type::Struct(name) | Type::Enum(name)) => {
                    let short = name.rsplit("::").next().unwrap_or(name);
                    format!("asType({}, {})!", print_expr(inner), sanitize_ident(short))
                }
                // Coerce + Struct/Enum → TS assertion (compiler-guaranteed).
                // Special case: `null as Type` is TS2352 because the types don't overlap;
                // use `null as unknown as Type` to go through a safe intermediate.
                // Also: some type names (e.g. AS3 `Class`) map to `any` in TypeScript;
                // use ts_type() to get the canonical form so `as Class` doesn't emit
                // a reference to the runtime value `Class` as a type annotation (TS2749).
                (CastKind::Coerce, Type::Struct(name) | Type::Enum(name)) => {
                    let short = name.rsplit("::").next().unwrap_or(name);
                    let ts_name = if matches!(short, "Class" | "Object") {
                        ts_type(ty)
                    } else {
                        sanitize_ident(short)
                    };
                    let inner_s = print_expr_operand(inner);
                    if matches!(inner.as_ref(), JsExpr::Literal(Constant::Null)) {
                        format!("null as unknown as {ts_name}")
                    } else {
                        format!("{inner_s} as {ts_name}")
                    }
                }
                // Coerce + Float → Number(x).
                (CastKind::Coerce, Type::Float(_)) => {
                    format!("Number({})", print_expr(unwrap_coerce(inner, ty)))
                }
                // Coerce + Int(64) → Number(x).  GML's i64 maps to TS `number`.
                (CastKind::Coerce, Type::Int(64)) => {
                    format!("Number({})", print_expr(unwrap_coerce(inner, ty)))
                }
                // Coerce + Int(32) → int(x).
                (CastKind::Coerce, Type::Int(32)) => {
                    format!("int({})", print_expr(unwrap_coerce(inner, ty)))
                }
                // Coerce + UInt(32) → uint(x).
                (CastKind::Coerce, Type::UInt(32)) => {
                    format!("uint({})", print_expr(unwrap_coerce(inner, ty)))
                }
                // Coerce + String → String(x).
                (CastKind::Coerce, Type::String) => {
                    format!("String({})", print_expr(unwrap_coerce(inner, ty)))
                }
                // Coerce + Bool → Boolean(x).
                (CastKind::Coerce, Type::Bool) => {
                    format!("Boolean({})", print_expr(unwrap_coerce(inner, ty)))
                }
                // Coerce + Dynamic/other → passthrough (no-op).
                (CastKind::Coerce, _) => print_expr(inner),
                // NullableCoerce + Dynamic → widen to `any`.
                // Used when a void-typed value is tested as a boolean condition (TS1345):
                // `Cast(void_val, Dynamic, NullableCoerce)` emits `(expr as any)`.
                (CastKind::NullableCoerce, Type::Dynamic) => {
                    format!("{} as any", print_expr_operand(inner))
                }
                // NullableCoerce + ClassRef → widen to `any`. GML object class names (OBJT)
                // are integer indices at runtime; `as any` allows usage in numeric /
                // arithmetic contexts while still passing the class constructor at
                // runtime (e.g. for `instanceof` and `instance_create_*`).
                (CastKind::NullableCoerce, Type::ClassRef(_)) => {
                    format!("{} as any", print_expr_operand(inner))
                }
                // NullableCoerce + Primitive → TS type assertion.
                (CastKind::NullableCoerce, _) => {
                    format!("{} as {}", print_expr_operand(inner), ts_type(ty))
                }
            }
        }

        JsExpr::TypeCheck {
            expr: inner,
            ty,
            use_instanceof,
        } => print_type_check(inner, ty, *use_instanceof),

        JsExpr::ArrayInit(elems) => {
            let elems_str: Vec<_> = elems.iter().map(print_expr).collect();
            format!("[{}]", elems_str.join(", "))
        }

        JsExpr::ObjectInit(pairs) => {
            if pairs.is_empty() {
                return "{}".to_string();
            }
            let field_strs: Vec<_> = pairs
                .iter()
                .map(|(name, val)| {
                    if name == "..." {
                        // Spread entry: emit `...expr`
                        format!("...{}", print_expr_operand(val))
                    } else if is_valid_js_ident(name) {
                        format!("{name}: {}", print_expr(val))
                    } else {
                        format!("\"{}\": {}", escape_js_string(name), print_expr(val))
                    }
                })
                .collect();
            format!("{{ {} }}", field_strs.join(", "))
        }

        JsExpr::TupleInit(elems) => {
            let elems_str: Vec<_> = elems.iter().map(print_expr).collect();
            format!("[{}]", elems_str.join(", "))
        }

        JsExpr::Not(inner) => {
            format!("!{}", print_expr_operand(inner))
        }

        JsExpr::PostIncrement(inner) => {
            format!("{}++", print_expr_operand(inner))
        }

        JsExpr::Spread(inner) => {
            format!("...{}", print_expr_operand(inner))
        }

        JsExpr::GeneratorCreate { func: fname, args } => {
            let args_str: Vec<_> = args.iter().map(print_expr).collect();
            format!("{}({})", sanitize_ident(fname), args_str.join(", "))
        }

        JsExpr::GeneratorResume(inner) => {
            format!("{}.next()", print_expr(inner))
        }

        JsExpr::Yield(v) => {
            if let Some(inner) = v {
                format!("yield {}", print_expr(inner))
            } else {
                "yield".into()
            }
        }

        // --- JS-specific constructs ---
        JsExpr::New { callee, args } => {
            let args_str: Vec<_> = args.iter().map(print_expr).collect();
            // Parenthesize call-expression and cast callees to avoid parse
            // ambiguity:
            //   `new (f(a,b))()` not `new f(a,b)()`
            //   `new (x as T)()` not `new x as T()`
            let callee_str = match callee.as_ref() {
                JsExpr::Call { .. } | JsExpr::SystemCall { .. } | JsExpr::Cast { .. } => {
                    format!("({})", print_expr(callee))
                }
                _ => print_expr(callee),
            };
            format!("new {}({})", callee_str, args_str.join(", "))
        }

        JsExpr::TypeOf(inner) => {
            format!("typeof {}", print_expr_operand(inner))
        }

        JsExpr::In { key, object } => {
            format!(
                "{} in {}",
                print_expr_operand(key),
                print_expr_operand(object),
            )
        }

        JsExpr::Delete { object, key } => {
            format!("delete {}[{}]", print_expr_operand(object), print_expr(key))
        }

        JsExpr::SuperCall(args) => {
            let args_str: Vec<_> = args.iter().map(print_expr).collect();
            format!("super({})", args_str.join(", "))
        }

        JsExpr::SuperMethodCall { method, args } => {
            let args_str: Vec<_> = args.iter().map(print_expr).collect();
            format!("super.{}({})", sanitize_ident(method), args_str.join(", "))
        }

        JsExpr::SuperGet(prop) => {
            format!("super.{}", sanitize_ident(prop))
        }

        JsExpr::SuperSet { prop, value } => {
            format!("(super.{} = {})", sanitize_ident(prop), print_expr(value),)
        }

        JsExpr::NonNull(inner) => format!("{}!", print_expr(inner)),

        // GML array auto-init: `(this.field ??= [])`.
        // Parenthesized so it can appear as the collection of an index expr.
        JsExpr::NullCoalesceAssign { target, value } => {
            format!("({} ??= {})", print_expr(target), print_expr(value))
        }

        JsExpr::Activation => "({})".to_string(),

        JsExpr::ArrowFunction {
            params,
            return_ty,
            body,
            has_rest_param,
            cast_as,
            infer_param_types,
        } => {
            let params_str = print_params(params, &[], *has_rest_param, *infer_param_types);
            let ret_ty = ts_type(return_ty);
            let mut out = format!("({params_str}): {ret_ty} => {{\n");
            print_stmts(body, &mut out, "  ");
            out.push('}');
            if let Some(cast) = cast_as {
                out = format!("({out}) as {cast}");
            }
            out
        }

        // --- Fallback: unmapped system call ---
        JsExpr::SystemCall {
            system,
            method,
            args,
        } => {
            let args_str: Vec<_> = args.iter().map(print_expr).collect();
            let sys_ident = sanitize_ident(system);
            let safe_method = if is_valid_js_ident(method) {
                format!(".{method}")
            } else {
                format!("[\"{}\"]", escape_js_string(method))
            };
            format!("{sys_ident}{safe_method}({})", args_str.join(", "))
        }
    }
}

/// Print an expression as an operand (may need parenthesization).
fn print_expr_operand(expr: &JsExpr) -> String {
    if needs_parens(expr) {
        format!("({})", print_expr(expr))
    } else {
        print_expr(expr)
    }
}

/// Whether an expression needs parentheses when used as an operand.
fn needs_parens(expr: &JsExpr) -> bool {
    match expr {
        // Function-call forms (asType, Number, int, etc.) don't need parens.
        // Only `x as T` forms need them.
        JsExpr::Cast { ty, kind, .. } => match (kind, ty) {
            (CastKind::NullableCoerce, Type::Struct(_) | Type::Enum(_)) => false,
            (CastKind::Coerce, Type::Struct(_) | Type::Enum(_)) => true, // `x as Foo`
            (
                CastKind::Coerce,
                Type::Float(_)
                | Type::Int(32)
                | Type::Int(64)
                | Type::UInt(32)
                | Type::String
                | Type::Bool,
            ) => false,
            (CastKind::Coerce, _) => false,        // passthrough
            (CastKind::NullableCoerce, _) => true, // `x as T` / `x as any`
        },
        // Negative numeric literals need parens to allow member access:
        //   (-4).length  not  -4.length  (TS1351: identifier after numeric literal)
        JsExpr::Literal(Constant::Int(n)) if *n < 0 => true,
        JsExpr::Literal(Constant::Float(f)) if *f < 0.0 => true,
        // `x instanceof T` is a binary expression — needs parens in unary context,
        // e.g. `!(x instanceof T)` not `!x instanceof T` (wrong precedence).
        JsExpr::TypeCheck {
            use_instanceof: true,
            ..
        } => true,
        JsExpr::Binary { .. }
        | JsExpr::Cmp { .. }
        | JsExpr::Ternary { .. }
        | JsExpr::LogicalOr { .. }
        | JsExpr::LogicalAnd { .. }
        | JsExpr::Unary { .. }
        | JsExpr::Not(_)
        | JsExpr::In { .. }
        | JsExpr::SuperSet { .. }
        | JsExpr::ArrowFunction { .. } => true,
        _ => false,
    }
}

// ---------------------------------------------------------------------------
// Type check printing
// ---------------------------------------------------------------------------

fn print_type_check(expr: &JsExpr, ty: &Type, use_instanceof: bool) -> String {
    let operand = print_expr_operand(expr);
    match ty {
        Type::Bool => format!("typeof {operand} === \"boolean\""),
        Type::Int(_) | Type::UInt(_) | Type::Float(_) => {
            format!("typeof {operand} === \"number\"")
        }
        Type::String => format!("typeof {operand} === \"string\""),
        Type::Struct(name) | Type::Enum(name) => {
            let short = name.rsplit("::").next().unwrap_or(name);
            if use_instanceof {
                // GML: all objects are class instances, `instanceof` is correct.
                // Use `(expr as any)` to prevent TypeScript control-flow narrowing
                // (`this instanceof Wall` in a Wall method narrows else-branch to `never`).
                format!(
                    "({} as any) instanceof {}",
                    print_expr(expr),
                    sanitize_ident(short)
                )
            } else {
                // Flash: use isType() — handles both classes and AS3 interfaces.
                format!("isType({}, {})", print_expr(expr), sanitize_ident(short))
            }
        }
        Type::Union(types) => {
            let checks: Vec<_> = types
                .iter()
                .map(|t| print_type_check(expr, t, use_instanceof))
                .collect();
            format!("({})", checks.join(" || "))
        }
        _ => format!("typeof {operand} === \"object\""),
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn is_null_literal(expr: &JsExpr) -> bool {
    matches!(expr, JsExpr::Literal(Constant::Null))
}

fn binop_str(op: BinOp) -> &'static str {
    match op {
        BinOp::Add => "+",
        BinOp::Sub => "-",
        BinOp::Mul => "*",
        BinOp::Div => "/",
        BinOp::Rem => "%",
        BinOp::BitAnd => "&",
        BinOp::BitOr => "|",
        BinOp::BitXor => "^",
        BinOp::Shl => "<<",
        BinOp::Shr => ">>",
        BinOp::BoolAnd => "&&",
        BinOp::BoolOr => "||",
    }
}

fn cmp_str(kind: CmpKind) -> &'static str {
    match kind {
        CmpKind::Eq => "===",
        CmpKind::Ne => "!==",
        CmpKind::Lt => "<",
        CmpKind::Le => "<=",
        CmpKind::Gt => ">",
        CmpKind::Ge => ">=",
        CmpKind::CoercingEq => "==",
        CmpKind::CoercingNe => "!=",
    }
}

pub fn is_valid_js_ident(name: &str) -> bool {
    let mut chars = name.chars();
    match chars.next() {
        None => false,
        Some(first) => {
            (unicode_ident::is_xid_start(first) || first == '_' || first == '$')
                && chars.all(|c| unicode_ident::is_xid_continue(c) || c == '$')
        }
    }
}

pub(crate) fn emit_constant(c: &Constant) -> String {
    match c {
        Constant::Null => "null".into(),
        Constant::Bool(b) => b.to_string(),
        Constant::Int(n) => n.to_string(),
        Constant::UInt(n) => n.to_string(),
        Constant::Float(f) => format_float(*f),
        Constant::String(s) => {
            if s.contains('\n') {
                format!("`{}`", escape_js_template(s))
            } else {
                format!("\"{}\"", escape_js_string(s))
            }
        }
    }
}

fn format_float(f: f64) -> String {
    if f.fract() == 0.0 && f.is_finite() {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

pub fn escape_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

/// Escape a string for use inside a JS template literal (backtick-quoted).
/// Newlines are preserved literally; backticks and `${` are escaped.
fn escape_js_template(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '`' => out.push_str("\\`"),
            '$' if chars.peek() == Some(&'{') => {
                chars.next();
                out.push_str("\\${");
            }
            '\r' => {
                // Normalize \r\n to \n, drop bare \r
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push('\n');
            }
            _ => out.push(ch),
        }
    }
    out
}

fn visibility_prefix(vis: Visibility) -> &'static str {
    match vis {
        Visibility::Public => "export ",
        Visibility::Private | Visibility::Protected => "",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reincarnate_core::ir::Type;

    #[test]
    fn type_check_struct_is_type_for_flash() {
        let expr = JsExpr::Var("v0".into());
        let result = print_type_check(&expr, &Type::Struct("Monster".into()), false);
        assert_eq!(
            result, "isType(v0, Monster)",
            "Flash TypeCheck should use isType()"
        );
    }

    #[test]
    fn type_check_struct_instanceof_for_gml() {
        let expr = JsExpr::Var("v0".into());
        let result = print_type_check(&expr, &Type::Struct("OEnemy".into()), true);
        // Uses `(expr as any) instanceof T` to prevent TypeScript control-flow narrowing
        // (`this instanceof Wall` in a Wall method would narrow else-branch to `never`).
        assert_eq!(
            result, "(v0 as any) instanceof OEnemy",
            "GML TypeCheck should use (as any) instanceof"
        );
    }

    #[test]
    fn type_check_instanceof_needs_parens_when_negated() {
        // `!x instanceof T` is wrong — TypeScript parses as `(!x) instanceof T`.
        // `needs_parens` must return true for TypeCheck { use_instanceof: true }.
        let tc = JsExpr::TypeCheck {
            expr: Box::new(JsExpr::Var("v0".into())),
            ty: Type::Struct("OEnemy".into()),
            use_instanceof: true,
        };
        assert!(
            needs_parens(&tc),
            "instanceof TypeCheck needs parens as operand"
        );
        let not_tc = JsExpr::Not(Box::new(tc));
        let result = print_expr(&not_tc);
        assert_eq!(
            result, "!((v0 as any) instanceof OEnemy)",
            "Not(TypeCheck) must add parens"
        );
    }
}
