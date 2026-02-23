//! SugarCube-specific JsExpr → JsExpr rewrites.
//!
//! Converts `SugarCube.Engine.*` SystemCall nodes into native JavaScript
//! constructs: `new`, `typeof`, `delete`, `in`, `Math.pow`, `String()`,
//! null-checks (`def`/`ndef`/`is_nullish`), closure inlining, and direct
//! calls to standalone pure functions exported from `sugarcube/engine.ts`
//! (`clone`, `iterate`, `iterator_*`, `ushr`, `instanceof_`).

use std::collections::HashMap;

use reincarnate_core::ir::value::Constant;
use reincarnate_core::ir::CmpKind;

use crate::js_ast::{JsExpr, JsFunction};

/// Returns the bare function names that a `SugarCube.Engine.*` rewrite
/// will introduce, if any. Used by import generation to emit correct imports.
pub(super) fn rewrite_introduced_calls(method: &str) -> &'static [&'static str] {
    match method {
        "clone" => &["clone"],
        "iterate" => &["iterate"],
        "iterator_has_next" => &["iterator_has_next"],
        "iterator_next_value" => &["iterator_next_value"],
        "iterator_next_key" => &["iterator_next_key"],
        "ushr" => &["ushr"],
        "instanceof" => &["instanceof_"],
        _ => &[],
    }
}

/// Try to rewrite a `SugarCube.Engine.*` SystemCall.
pub(super) fn try_rewrite(
    method: &str,
    args: &mut Vec<JsExpr>,
    closures: &HashMap<String, JsFunction>,
) -> Option<JsExpr> {
    match method {
        // closure(name[, cap0, cap1, ...]) → arrow function or IIFE
        //
        // With no captures (args.len() == 1): inline the closure body as a
        // plain ArrowFunction.
        //
        // With captures (args.len() > 1): emit an IIFE that binds the capture
        // values at creation time and returns an inner arrow for the actual call:
        //   ((cap0, cap1) => (reg_params...) => body)(cap_val0, cap_val1)
        //
        // The split between capture params and regular params is stored in
        // `JsFunction::num_capture_params`: the last N params are captures.
        "closure" if !args.is_empty() => {
            if let JsExpr::Literal(Constant::String(ref name)) = args[0] {
                if let Some(closure_func) = closures.get(name.as_str()).cloned() {
                    let n_cap = closure_func.num_capture_params;
                    let rewritten = super::rewrite_twine_function(closure_func, closures);
                    let n_total = rewritten.params.len();
                    let n_reg = n_total.saturating_sub(n_cap);

                    if n_cap == 0 || args.len() == 1 {
                        // No captures (or caller didn't pass any): plain arrow.
                        return Some(JsExpr::ArrowFunction {
                            params: rewritten.params,
                            return_ty: rewritten.return_ty,
                            body: rewritten.body,
                            has_rest_param: rewritten.has_rest_param,
                            cast_as: None,
                            infer_param_types: false,
                        });
                    }

                    // Split params: first n_reg are regular, last n_cap are captures.
                    let mut all_params = rewritten.params;
                    // Cap params are `any`-typed: Harlowe/SugarCube variables are
                    // dynamically typed and captured initial values may differ from
                    // the inferred type used inside the closure body.
                    let cap_params: Vec<(String, reincarnate_core::ir::Type)> =
                        all_params.split_off(n_reg)
                            .into_iter()
                            .map(|(name, _)| (name, reincarnate_core::ir::Type::Dynamic))
                            .collect();
                    let reg_params = all_params;                   // regular params

                    // Inner arrow: takes regular params, body unchanged.
                    let inner = JsExpr::ArrowFunction {
                        params: reg_params,
                        return_ty: rewritten.return_ty,
                        body: rewritten.body,
                        has_rest_param: rewritten.has_rest_param,
                        cast_as: None,
                        infer_param_types: false,
                    };

                    // Outer arrow: takes capture params, returns inner arrow.
                    let outer = JsExpr::ArrowFunction {
                        params: cap_params,
                        return_ty: reincarnate_core::ir::Type::Dynamic,
                        body: vec![crate::js_ast::JsStmt::Return(Some(inner))],
                        has_rest_param: false,
                        cast_as: None,
                        infer_param_types: false,
                    };

                    // IIFE: immediately invoke the outer arrow with capture values.
                    let capture_args = args.drain(1..).collect();
                    return Some(JsExpr::Call {
                        callee: Box::new(outer),
                        args: capture_args,
                    });
                }
            }
            None
        }
        // new(callee, ...args) → new callee(...args)
        "new" if !args.is_empty() => {
            let mut a = std::mem::take(args);
            let callee = a.remove(0);
            Some(JsExpr::New {
                callee: Box::new(callee),
                args: a,
            })
        }

        // typeof(v) → typeof v
        "typeof" if args.len() == 1 => {
            let v = args.pop().unwrap();
            Some(JsExpr::TypeOf(Box::new(v)))
        }

        // delete(expr) → delete expr
        "delete" if args.len() == 1 => {
            let v = args.pop().unwrap();
            match v {
                JsExpr::Field { object, field } => Some(JsExpr::Delete {
                    object,
                    key: Box::new(JsExpr::Literal(Constant::String(field))),
                }),
                JsExpr::Index { collection, index } => Some(JsExpr::Delete {
                    object: collection,
                    key: index,
                }),
                other => Some(JsExpr::Delete {
                    object: Box::new(other),
                    key: Box::new(JsExpr::Literal(Constant::String("__delete__".into()))),
                }),
            }
        }

        // in(key, obj) → key in obj
        "in" if args.len() == 2 => {
            let obj = args.pop().unwrap();
            let key = args.pop().unwrap();
            Some(JsExpr::In {
                key: Box::new(key),
                object: Box::new(obj),
            })
        }

        // pow(a, b) → Math.pow(a, b)
        "pow" if args.len() == 2 => {
            let b = args.pop().unwrap();
            let a = args.pop().unwrap();
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Field {
                    object: Box::new(JsExpr::Var("Math".into())),
                    field: "pow".into(),
                }),
                args: vec![a, b],
            })
        }

        // def(v) → v != null (loose inequality)
        "def" if args.len() == 1 => {
            let v = args.pop().unwrap();
            Some(JsExpr::Cmp {
                kind: CmpKind::Ne,
                lhs: Box::new(v),
                rhs: Box::new(JsExpr::Literal(Constant::Null)),
            })
        }

        // ndef(v) → v == null (loose equality)
        "ndef" if args.len() == 1 => {
            let v = args.pop().unwrap();
            Some(JsExpr::Cmp {
                kind: CmpKind::Eq,
                lhs: Box::new(v),
                rhs: Box::new(JsExpr::Literal(Constant::Null)),
            })
        }

        // is_nullish(v) → v == null (loose equality, matches null and undefined)
        "is_nullish" if args.len() == 1 => {
            let v = args.pop().unwrap();
            Some(JsExpr::Cmp {
                kind: CmpKind::Eq,
                lhs: Box::new(v),
                rhs: Box::new(JsExpr::Literal(Constant::Null)),
            })
        }

        // to_string(v) → String(v)
        "to_string" if args.len() == 1 => {
            let v = args.pop().unwrap();
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("String".into())),
                args: vec![v],
            })
        }

        // clone(v) → clone(v)  (standalone pure function from sugarcube/engine.ts)
        "clone" => {
            let a = std::mem::take(args);
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("clone".into())),
                args: a,
            })
        }

        // iterate / iterator_* → standalone pure functions from sugarcube/engine.ts
        "iterate" | "iterator_has_next" | "iterator_next_value" | "iterator_next_key" => {
            let name = method.to_string();
            let a = std::mem::take(args);
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var(name)),
                args: a,
            })
        }

        // ushr(a, b) → ushr(a, b)
        "ushr" if args.len() == 2 => {
            let a = std::mem::take(args);
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("ushr".into())),
                args: a,
            })
        }

        // instanceof(v, t) → instanceof_(v, t)
        "instanceof" if args.len() == 2 => {
            let a = std::mem::take(args);
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("instanceof_".into())),
                args: a,
            })
        }

        // Everything else (resolve, eval, arrow, error, done_*, break, continue,
        // parseLink) passes through to the runtime via the SystemCall fallback in
        // the printer.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::js_ast::{JsExpr, JsFunction, JsStmt};
    use reincarnate_core::ir::value::Constant;
    use reincarnate_core::ir::{MethodKind, Type, Visibility};

    fn func_with_expr(expr: JsExpr) -> JsFunction {
        JsFunction {
            name: "test".into(),
            params: vec![],
            param_defaults: vec![],
            return_ty: Type::Void,
            body: vec![JsStmt::Expr(expr)],
            is_generator: false,
            visibility: Visibility::Public,
            method_kind: MethodKind::Free,
            has_rest_param: false,
            num_capture_params: 0,
        }
    }

    fn no_closures() -> HashMap<String, JsFunction> {
        HashMap::new()
    }

    fn extract_expr(func: &JsFunction) -> &JsExpr {
        match &func.body[0] {
            JsStmt::Expr(e) => e,
            _ => panic!("expected Expr statement"),
        }
    }

    #[test]
    fn rewrite_new() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "new".into(),
            args: vec![
                JsExpr::Var("Date".into()),
                JsExpr::Literal(Constant::Int(2025)),
            ],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::New { .. }));
    }

    #[test]
    fn rewrite_typeof() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "typeof".into(),
            args: vec![JsExpr::Var("x".into())],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::TypeOf(_)));
    }

    #[test]
    fn rewrite_def_ndef() {
        let def = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "def".into(),
            args: vec![JsExpr::Var("x".into())],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(def), &no_closures());
        assert!(matches!(
            extract_expr(&func),
            JsExpr::Cmp {
                kind: CmpKind::Ne,
                ..
            }
        ));

        let ndef = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "ndef".into(),
            args: vec![JsExpr::Var("x".into())],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(ndef), &no_closures());
        assert!(matches!(
            extract_expr(&func),
            JsExpr::Cmp {
                kind: CmpKind::Eq,
                ..
            }
        ));
    }

    #[test]
    fn rewrite_pow() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "pow".into(),
            args: vec![JsExpr::Var("a".into()), JsExpr::Var("b".into())],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::Call { .. }));
    }

    #[test]
    fn rewrite_in() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "in".into(),
            args: vec![
                JsExpr::Literal(Constant::String("key".into())),
                JsExpr::Var("obj".into()),
            ],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::In { .. }));
    }

    #[test]
    fn passthrough_state_get() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.State".into(),
            method: "get".into(),
            args: vec![JsExpr::Literal(Constant::String("name".into()))],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::SystemCall { .. }));
    }

    #[test]
    fn passthrough_engine_eval() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "eval".into(),
            args: vec![JsExpr::Literal(Constant::String("code".into()))],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::SystemCall { .. }));
    }

    #[test]
    fn rewrite_to_string() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "to_string".into(),
            args: vec![JsExpr::Var("x".into())],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        match extract_expr(&func) {
            JsExpr::Call { callee, args } => {
                assert!(matches!(callee.as_ref(), JsExpr::Var(n) if n == "String"));
                assert_eq!(args.len(), 1);
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_delete_field() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "delete".into(),
            args: vec![JsExpr::Field {
                object: Box::new(JsExpr::Var("obj".into())),
                field: "prop".into(),
            }],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::Delete { .. }));
    }

    #[test]
    fn rewrite_is_nullish() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "is_nullish".into(),
            args: vec![JsExpr::Var("x".into())],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(
            extract_expr(&func),
            JsExpr::Cmp {
                kind: CmpKind::Eq,
                ..
            }
        ));
    }

    #[test]
    fn rewrite_closure_inline() {
        let mut closures = HashMap::new();
        closures.insert(
            "test_arrow_0".to_string(),
            JsFunction {
                name: "test_arrow_0".into(),
                params: vec![("x".into(), Type::Dynamic)],
                param_defaults: vec![],
                return_ty: Type::Dynamic,
                body: vec![JsStmt::Return(Some(JsExpr::Var("x".into())))],
                is_generator: false,
                visibility: Visibility::Private,
                method_kind: MethodKind::Closure,
                has_rest_param: false,
                num_capture_params: 0,
            },
        );

        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "closure".into(),
            args: vec![JsExpr::Literal(Constant::String("test_arrow_0".into()))],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &closures);
        match extract_expr(&func) {
            JsExpr::ArrowFunction { params, body, .. } => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].0, "x");
                assert_eq!(body.len(), 1);
            }
            other => panic!("expected ArrowFunction, got {other:?}"),
        }
    }

    fn sc_engine_call(method: &str, args: Vec<JsExpr>) -> JsExpr {
        JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: method.into(),
            args,
        }
    }

    fn var(name: &str) -> JsExpr {
        JsExpr::Var(name.into())
    }

    fn assert_direct_call(func: &JsFunction, expected_callee: &str, expected_arity: usize) {
        match extract_expr(func) {
            JsExpr::Call { callee, args } => {
                assert!(
                    matches!(callee.as_ref(), JsExpr::Var(n) if n == expected_callee),
                    "expected callee {expected_callee}, got {callee:?}"
                );
                assert_eq!(args.len(), expected_arity);
            }
            other => panic!("expected Call to {expected_callee}, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_clone() {
        let expr = sc_engine_call("clone", vec![var("x")]);
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert_direct_call(&func, "clone", 1);
    }

    #[test]
    fn rewrite_iterate() {
        let expr = sc_engine_call("iterate", vec![var("coll")]);
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert_direct_call(&func, "iterate", 1);
    }

    #[test]
    fn rewrite_iterator_has_next() {
        let expr = sc_engine_call("iterator_has_next", vec![var("iter")]);
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert_direct_call(&func, "iterator_has_next", 1);
    }

    #[test]
    fn rewrite_iterator_next_value() {
        let expr = sc_engine_call("iterator_next_value", vec![var("iter")]);
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert_direct_call(&func, "iterator_next_value", 1);
    }

    #[test]
    fn rewrite_iterator_next_key() {
        let expr = sc_engine_call("iterator_next_key", vec![var("iter")]);
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert_direct_call(&func, "iterator_next_key", 1);
    }

    #[test]
    fn rewrite_ushr() {
        let expr = sc_engine_call("ushr", vec![var("a"), var("b")]);
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert_direct_call(&func, "ushr", 2);
    }

    #[test]
    fn rewrite_instanceof() {
        let expr = sc_engine_call("instanceof", vec![var("v"), var("T")]);
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert_direct_call(&func, "instanceof_", 2);
    }

    #[test]
    fn introduced_calls_sugarcube() {
        assert_eq!(rewrite_introduced_calls("clone"), &["clone"]);
        assert_eq!(rewrite_introduced_calls("iterate"), &["iterate"]);
        assert_eq!(rewrite_introduced_calls("iterator_has_next"), &["iterator_has_next"]);
        assert_eq!(rewrite_introduced_calls("iterator_next_value"), &["iterator_next_value"]);
        assert_eq!(rewrite_introduced_calls("iterator_next_key"), &["iterator_next_key"]);
        assert_eq!(rewrite_introduced_calls("ushr"), &["ushr"]);
        assert_eq!(rewrite_introduced_calls("instanceof"), &["instanceof_"]);
        assert!(rewrite_introduced_calls("eval").is_empty());
        assert!(rewrite_introduced_calls("resolve").is_empty());
    }
}
