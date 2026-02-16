//! Twine-specific JsExpr → JsExpr rewrite pass.
//!
//! Resolves `SugarCube.Engine.*` SystemCall nodes that map to native JavaScript
//! constructs (`new`, `typeof`, `delete`, `in`, `**`, etc.) and `Harlowe.Output.*`
//! content tree SystemCalls (`content_array` → ArrayInit, `text_node` → identity).
//! All other SystemCalls pass through to runtime modules via the printer's
//! `SystemCall` fallback + auto-import machinery.

use std::collections::HashMap;

use reincarnate_core::ir::value::Constant;
use reincarnate_core::ir::CmpKind;

use crate::js_ast::{JsExpr, JsFunction, JsStmt};

/// Returns the bare function names that a SystemCall rewrite will introduce,
/// if any. Used by import generation to emit the correct imports before
/// the rewrite pass runs.
///
/// For `Harlowe.Output.*` calls, the method names (except `content_array` and
/// `text_node` which are rewritten to JS constructs) become bare function calls
/// that need to be imported via `function_modules`.
pub fn rewrite_introduced_calls(system: &str, method: &str) -> &'static [&'static str] {
    if system == "Harlowe.Output" {
        // content_array → ArrayInit, text_node → identity: no import needed.
        // All other methods become bare function calls imported via function_modules.
        match method {
            "content_array" | "text_node" => &[],
            "color" => &["color"],
            "background" => &["background"],
            "textStyle" => &["textStyle"],
            "font" => &["font"],
            "align" => &["align"],
            "opacity" => &["opacity"],
            "css" => &["css"],
            "transition" => &["transition"],
            "transitionTime" => &["transitionTime"],
            "hidden" => &["hidden"],
            "textSize" => &["textSize"],
            "textRotateZ" => &["textRotateZ"],
            "collapse" => &["collapse"],
            "nobr" => &["nobr"],
            "hoverStyle" => &["hoverStyle"],
            "styled" => &["styled"],
            "el" => &["el"],
            "strong" => &["strong"],
            "em" => &["em"],
            "del" => &["del"],
            "sup" => &["sup"],
            "sub" => &["sub"],
            "br" => &["br"],
            "hr" => &["hr"],
            "img" => &["img"],
            "voidEl" => &["voidEl"],
            "link" => &["link"],
            "linkCb" => &["linkCb"],
            "live" => &["live"],
            "printVal" => &["printVal"],
            "displayPassage" => &["displayPassage"],
            _ => &[],
        }
    } else {
        // SugarCube rewrites produce only built-in JS constructs (new, typeof, etc.)
        // and calls to Math.pow / String — none of which need function-module imports.
        &[]
    }
}

/// Rewrite a function's body, resolving SugarCube.Engine SystemCalls that map
/// to native JS constructs and inlining closure bodies as arrow functions.
pub fn rewrite_twine_function(
    mut func: JsFunction,
    closure_bodies: &HashMap<String, JsFunction>,
) -> JsFunction {
    rewrite_stmts(&mut func.body, closure_bodies);
    func
}

fn rewrite_stmts(stmts: &mut [JsStmt], closures: &HashMap<String, JsFunction>) {
    for stmt in stmts.iter_mut() {
        rewrite_stmt(stmt, closures);
    }
}

fn rewrite_stmt(stmt: &mut JsStmt, closures: &HashMap<String, JsFunction>) {
    match stmt {
        JsStmt::VarDecl { init, .. } => {
            if let Some(expr) = init {
                rewrite_expr(expr, closures);
            }
        }
        JsStmt::Assign { target, value } => {
            rewrite_expr(target, closures);
            rewrite_expr(value, closures);
        }
        JsStmt::CompoundAssign { target, value, .. } => {
            rewrite_expr(target, closures);
            rewrite_expr(value, closures);
        }
        JsStmt::Expr(e) => rewrite_expr(e, closures),
        JsStmt::Return(Some(e)) => rewrite_expr(e, closures),
        JsStmt::Return(None) => {}
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            rewrite_expr(cond, closures);
            rewrite_stmts(then_body, closures);
            rewrite_stmts(else_body, closures);
        }
        JsStmt::While { cond, body } => {
            rewrite_expr(cond, closures);
            rewrite_stmts(body, closures);
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            rewrite_stmts(init, closures);
            rewrite_expr(cond, closures);
            rewrite_stmts(update, closures);
            rewrite_stmts(body, closures);
        }
        JsStmt::Loop { body } => rewrite_stmts(body, closures),
        JsStmt::ForOf { iterable, body, .. } => {
            rewrite_expr(iterable, closures);
            rewrite_stmts(body, closures);
        }
        JsStmt::Throw(e) => rewrite_expr(e, closures),
        JsStmt::Dispatch { blocks, .. } => {
            for (_, stmts) in blocks {
                rewrite_stmts(stmts, closures);
            }
        }
        JsStmt::Switch {
            value,
            cases,
            default_body,
        } => {
            rewrite_expr(value, closures);
            for (_, stmts) in cases {
                rewrite_stmts(stmts, closures);
            }
            rewrite_stmts(default_body, closures);
        }
        JsStmt::Break | JsStmt::Continue | JsStmt::LabeledBreak { .. } => {}
    }
}

fn rewrite_expr(expr: &mut JsExpr, closures: &HashMap<String, JsFunction>) {
    // Recurse into children first.
    rewrite_expr_children(expr, closures);

    // Then attempt to resolve SystemCall patterns.
    let replacement = match expr {
        JsExpr::SystemCall {
            system,
            method,
            args,
        } => try_rewrite_system_call(system, method, args, closures),
        _ => None,
    };

    if let Some(new_expr) = replacement {
        *expr = new_expr;
    }
}

fn rewrite_expr_children(expr: &mut JsExpr, closures: &HashMap<String, JsFunction>) {
    match expr {
        JsExpr::Binary { lhs, rhs, .. } | JsExpr::Cmp { lhs, rhs, .. } => {
            rewrite_expr(lhs, closures);
            rewrite_expr(rhs, closures);
        }
        JsExpr::LogicalOr { lhs, rhs } | JsExpr::LogicalAnd { lhs, rhs } => {
            rewrite_expr(lhs, closures);
            rewrite_expr(rhs, closures);
        }
        JsExpr::Unary { expr: inner, .. } => rewrite_expr(inner, closures),
        JsExpr::Not(inner) | JsExpr::PostIncrement(inner) | JsExpr::Spread(inner) => rewrite_expr(inner, closures),
        JsExpr::Field { object, .. } => rewrite_expr(object, closures),
        JsExpr::Index { collection, index } => {
            rewrite_expr(collection, closures);
            rewrite_expr(index, closures);
        }
        JsExpr::Call { callee, args } => {
            rewrite_expr(callee, closures);
            for arg in args {
                rewrite_expr(arg, closures);
            }
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            rewrite_expr(cond, closures);
            rewrite_expr(then_val, closures);
            rewrite_expr(else_val, closures);
        }
        JsExpr::ArrayInit(items) | JsExpr::TupleInit(items) => {
            for item in items {
                rewrite_expr(item, closures);
            }
        }
        JsExpr::ObjectInit(fields) => {
            for (_, val) in fields {
                rewrite_expr(val, closures);
            }
        }
        JsExpr::New { callee, args } => {
            rewrite_expr(callee, closures);
            for arg in args {
                rewrite_expr(arg, closures);
            }
        }
        JsExpr::TypeOf(inner) => rewrite_expr(inner, closures),
        JsExpr::In { key, object } => {
            rewrite_expr(key, closures);
            rewrite_expr(object, closures);
        }
        JsExpr::Delete { object, key } => {
            rewrite_expr(object, closures);
            rewrite_expr(key, closures);
        }
        JsExpr::Cast { expr: inner, .. } | JsExpr::TypeCheck { expr: inner, .. } => {
            rewrite_expr(inner, closures);
        }
        JsExpr::ArrowFunction { body, .. } => rewrite_stmts(body, closures),
        JsExpr::SuperCall(args) | JsExpr::SuperMethodCall { args, .. } => {
            for arg in args {
                rewrite_expr(arg, closures);
            }
        }
        JsExpr::SuperGet(_) => {}
        JsExpr::SuperSet { value, .. } => rewrite_expr(value, closures),
        JsExpr::GeneratorCreate { args, .. } => {
            for arg in args {
                rewrite_expr(arg, closures);
            }
        }
        JsExpr::GeneratorResume(inner) => rewrite_expr(inner, closures),
        JsExpr::Yield(inner) => {
            if let Some(e) = inner {
                rewrite_expr(e, closures);
            }
        }
        JsExpr::Activation => {}
        JsExpr::SystemCall { args, .. } => {
            for arg in args {
                rewrite_expr(arg, closures);
            }
        }
        JsExpr::Literal(_) | JsExpr::Var(_) | JsExpr::This => {}
    }
}

/// Try to rewrite a Twine SystemCall into a native JS expression.
///
/// Rewrites:
/// - `SugarCube.Engine.*` calls → native JS constructs (new, typeof, etc.)
/// - `Harlowe.Output.content_array(...)` → `[...]` (ArrayInit)
/// - `Harlowe.Output.text_node(s)` → `s` (identity — strings ARE content nodes)
/// - Other `Harlowe.Output.*` calls → bare function calls (method name as callee)
fn try_rewrite_system_call(
    system: &str,
    method: &str,
    args: &mut Vec<JsExpr>,
    closures: &HashMap<String, JsFunction>,
) -> Option<JsExpr> {
    // Harlowe.Output rewrites
    if system == "Harlowe.Output" {
        return match method {
            "content_array" => Some(JsExpr::ArrayInit(std::mem::take(args))),
            "text_node" if args.len() == 1 => Some(args.pop().unwrap()),
            _ => {
                // All other Harlowe.Output methods → bare function call
                let callee = JsExpr::Var(method.to_string());
                Some(JsExpr::Call {
                    callee: Box::new(callee),
                    args: std::mem::take(args),
                })
            }
        };
    }

    if system != "SugarCube.Engine" {
        return None;
    }

    match method {
        // closure(name) → inline arrow function from pre-compiled closure body
        "closure" if args.len() == 1 => {
            if let JsExpr::Literal(Constant::String(ref name)) = args[0] {
                if let Some(closure_func) = closures.get(name.as_str()).cloned() {
                    let rewritten = rewrite_twine_function(closure_func, closures);
                    return Some(JsExpr::ArrowFunction {
                        params: rewritten.params,
                        return_ty: rewritten.return_ty,
                        body: rewritten.body,
                        has_rest_param: rewritten.has_rest_param,
                        cast_as: None,
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
        // The frontend emits delete with a single expression argument.
        // If it's a field access, split into object + key for Delete node.
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
                // Fallback: delete v (not a standard pattern but preserve it)
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

        // Everything else (clone, resolve, iterate, iterator_*, eval, arrow,
        // error, done_*, break, continue, ushr, instanceof) passes through
        // to the runtime via the SystemCall fallback in the printer.
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reincarnate_core::ir::{MethodKind, Type, Visibility};

    /// Helper to build a JsFunction with a single expression statement.
    fn func_with_expr(expr: JsExpr) -> JsFunction {
        JsFunction {
            name: "test".into(),
            params: vec![],
            return_ty: Type::Void,
            body: vec![JsStmt::Expr(expr)],
            is_generator: false,
            visibility: Visibility::Public,
            method_kind: MethodKind::Free,
            has_rest_param: false,
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
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::New { .. }));
    }

    #[test]
    fn rewrite_typeof() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "typeof".into(),
            args: vec![JsExpr::Var("x".into())],
        };
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::TypeOf(_)));
    }

    #[test]
    fn rewrite_def_ndef() {
        let def = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "def".into(),
            args: vec![JsExpr::Var("x".into())],
        };
        let func = rewrite_twine_function(func_with_expr(def), &no_closures());
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
        let func = rewrite_twine_function(func_with_expr(ndef), &no_closures());
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
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
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
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::In { .. }));
    }

    #[test]
    fn passthrough_state_get() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.State".into(),
            method: "get".into(),
            args: vec![JsExpr::Literal(Constant::String("name".into()))],
        };
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        // Should remain as SystemCall — not rewritten
        assert!(matches!(extract_expr(&func), JsExpr::SystemCall { .. }));
    }

    #[test]
    fn passthrough_engine_eval() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "eval".into(),
            args: vec![JsExpr::Literal(Constant::String("code".into()))],
        };
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        // eval should pass through to runtime
        assert!(matches!(extract_expr(&func), JsExpr::SystemCall { .. }));
    }

    #[test]
    fn rewrite_to_string() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "to_string".into(),
            args: vec![JsExpr::Var("x".into())],
        };
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
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
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(extract_expr(&func), JsExpr::Delete { .. }));
    }

    #[test]
    fn rewrite_is_nullish() {
        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "is_nullish".into(),
            args: vec![JsExpr::Var("x".into())],
        };
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        assert!(matches!(
            extract_expr(&func),
            JsExpr::Cmp {
                kind: CmpKind::Eq,
                ..
            }
        ));
    }

    // --- Harlowe.Output rewrite tests ---

    #[test]
    fn rewrite_harlowe_content_array() {
        let expr = JsExpr::SystemCall {
            system: "Harlowe.Output".into(),
            method: "content_array".into(),
            args: vec![
                JsExpr::Literal(Constant::String("hello".into())),
                JsExpr::Var("x".into()),
            ],
        };
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        match extract_expr(&func) {
            JsExpr::ArrayInit(items) => {
                assert_eq!(items.len(), 2);
                assert!(matches!(&items[0], JsExpr::Literal(Constant::String(s)) if s == "hello"));
                assert!(matches!(&items[1], JsExpr::Var(n) if n == "x"));
            }
            other => panic!("expected ArrayInit, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_harlowe_text_node() {
        let expr = JsExpr::SystemCall {
            system: "Harlowe.Output".into(),
            method: "text_node".into(),
            args: vec![JsExpr::Literal(Constant::String("hello".into()))],
        };
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        match extract_expr(&func) {
            JsExpr::Literal(Constant::String(s)) => assert_eq!(s, "hello"),
            other => panic!("expected string literal, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_harlowe_builder_call() {
        // Harlowe.Output.color(val, children) → color(val, children)
        let expr = JsExpr::SystemCall {
            system: "Harlowe.Output".into(),
            method: "color".into(),
            args: vec![
                JsExpr::Literal(Constant::String("red".into())),
                JsExpr::ArrayInit(vec![JsExpr::Literal(Constant::String("text".into()))]),
            ],
        };
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        match extract_expr(&func) {
            JsExpr::Call { callee, args } => {
                assert!(matches!(callee.as_ref(), JsExpr::Var(n) if n == "color"));
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_harlowe_br() {
        let expr = JsExpr::SystemCall {
            system: "Harlowe.Output".into(),
            method: "br".into(),
            args: vec![],
        };
        let func = rewrite_twine_function(func_with_expr(expr), &no_closures());
        match extract_expr(&func) {
            JsExpr::Call { callee, args } => {
                assert!(matches!(callee.as_ref(), JsExpr::Var(n) if n == "br"));
                assert!(args.is_empty());
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn harlowe_introduced_calls() {
        assert_eq!(rewrite_introduced_calls("Harlowe.Output", "color"), &["color"]);
        assert_eq!(rewrite_introduced_calls("Harlowe.Output", "strong"), &["strong"]);
        assert_eq!(rewrite_introduced_calls("Harlowe.Output", "br"), &["br"]);
        assert_eq!(rewrite_introduced_calls("Harlowe.Output", "link"), &["link"]);
        assert_eq!(rewrite_introduced_calls("Harlowe.Output", "printVal"), &["printVal"]);
        // Rewritten to JS constructs — no import needed
        assert!(rewrite_introduced_calls("Harlowe.Output", "content_array").is_empty());
        assert!(rewrite_introduced_calls("Harlowe.Output", "text_node").is_empty());
        // SugarCube — no imports
        assert!(rewrite_introduced_calls("SugarCube.Engine", "new").is_empty());
    }

    #[test]
    fn rewrite_closure_inline() {
        // A closure SystemCall should be replaced with an inline ArrowFunction
        // when the closure body is available in the closure_bodies map.
        let mut closures = HashMap::new();
        closures.insert(
            "test_arrow_0".to_string(),
            JsFunction {
                name: "test_arrow_0".into(),
                params: vec![("x".into(), Type::Dynamic)],
                return_ty: Type::Dynamic,
                body: vec![JsStmt::Return(Some(JsExpr::Var("x".into())))],
                is_generator: false,
                visibility: Visibility::Private,
                method_kind: MethodKind::Closure,
                has_rest_param: false,
            },
        );

        let expr = JsExpr::SystemCall {
            system: "SugarCube.Engine".into(),
            method: "closure".into(),
            args: vec![JsExpr::Literal(Constant::String("test_arrow_0".into()))],
        };
        let func = rewrite_twine_function(func_with_expr(expr), &closures);
        match extract_expr(&func) {
            JsExpr::ArrowFunction { params, body, .. } => {
                assert_eq!(params.len(), 1);
                assert_eq!(params[0].0, "x");
                assert_eq!(body.len(), 1);
            }
            other => panic!("expected ArrowFunction, got {other:?}"),
        }
    }
}
