//! Harlowe-specific JsExpr → JsExpr rewrites.
//!
//! Converts `Harlowe.Output.*` SystemCall nodes into native JS constructs
//! or bare function calls:
//! - `content_array(...)` → `[...]` (ArrayInit)
//! - `text_node(s)` → `s` (identity — strings ARE content nodes)
//! - All other methods → bare function calls imported via `function_modules`

use crate::js_ast::JsExpr;

/// Returns the bare function names that a `Harlowe.Output` rewrite will
/// introduce, if any. Used by import generation to emit correct imports.
///
/// `content_array` and `text_node` are rewritten to JS constructs (not
/// function calls), so they return empty.
pub(super) fn rewrite_introduced_calls(method: &str) -> &'static [&'static str] {
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
}

/// Try to rewrite a `Harlowe.Output.*` SystemCall.
pub(super) fn try_rewrite(method: &str, args: &mut Vec<JsExpr>) -> Option<JsExpr> {
    match method {
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
            return_ty: Type::Void,
            body: vec![JsStmt::Expr(expr)],
            is_generator: false,
            visibility: Visibility::Public,
            method_kind: MethodKind::Free,
            has_rest_param: false,
        }
    }

    fn no_closures() -> std::collections::HashMap<String, JsFunction> {
        std::collections::HashMap::new()
    }

    fn extract_expr(func: &JsFunction) -> &JsExpr {
        match &func.body[0] {
            JsStmt::Expr(e) => e,
            _ => panic!("expected Expr statement"),
        }
    }

    #[test]
    fn rewrite_content_array() {
        let expr = JsExpr::SystemCall {
            system: "Harlowe.Output".into(),
            method: "content_array".into(),
            args: vec![
                JsExpr::Literal(Constant::String("hello".into())),
                JsExpr::Var("x".into()),
            ],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
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
    fn rewrite_text_node() {
        let expr = JsExpr::SystemCall {
            system: "Harlowe.Output".into(),
            method: "text_node".into(),
            args: vec![JsExpr::Literal(Constant::String("hello".into()))],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        match extract_expr(&func) {
            JsExpr::Literal(Constant::String(s)) => assert_eq!(s, "hello"),
            other => panic!("expected string literal, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_builder_call() {
        let expr = JsExpr::SystemCall {
            system: "Harlowe.Output".into(),
            method: "color".into(),
            args: vec![
                JsExpr::Literal(Constant::String("red".into())),
                JsExpr::ArrayInit(vec![JsExpr::Literal(Constant::String("text".into()))]),
            ],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        match extract_expr(&func) {
            JsExpr::Call { callee, args } => {
                assert!(matches!(callee.as_ref(), JsExpr::Var(n) if n == "color"));
                assert_eq!(args.len(), 2);
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn rewrite_br() {
        let expr = JsExpr::SystemCall {
            system: "Harlowe.Output".into(),
            method: "br".into(),
            args: vec![],
        };
        let func = super::super::rewrite_twine_function(func_with_expr(expr), &no_closures());
        match extract_expr(&func) {
            JsExpr::Call { callee, args } => {
                assert!(matches!(callee.as_ref(), JsExpr::Var(n) if n == "br"));
                assert!(args.is_empty());
            }
            other => panic!("expected Call, got {other:?}"),
        }
    }

    #[test]
    fn introduced_calls() {
        assert_eq!(rewrite_introduced_calls("color"), &["color"]);
        assert_eq!(rewrite_introduced_calls("strong"), &["strong"]);
        assert_eq!(rewrite_introduced_calls("br"), &["br"]);
        assert_eq!(rewrite_introduced_calls("link"), &["link"]);
        assert_eq!(rewrite_introduced_calls("printVal"), &["printVal"]);
        assert!(rewrite_introduced_calls("content_array").is_empty());
        assert!(rewrite_introduced_calls("text_node").is_empty());
    }
}
