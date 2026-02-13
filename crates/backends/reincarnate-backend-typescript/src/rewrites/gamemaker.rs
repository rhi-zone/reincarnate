//! GameMaker-specific JsExpr → JsExpr rewrite pass.
//!
//! Resolves GameMaker SystemCall nodes into native JavaScript constructs.
//! Much simpler than Flash — only 8 SystemCall patterns to handle.

use crate::js_ast::{JsExpr, JsFunction, JsStmt};

/// Rewrite a function's body, resolving GameMaker SystemCalls.
pub fn rewrite_gamemaker_function(mut func: JsFunction) -> JsFunction {
    rewrite_stmts(&mut func.body);
    func
}

fn rewrite_stmts(stmts: &mut [JsStmt]) {
    for stmt in stmts.iter_mut() {
        rewrite_stmt(stmt);
    }
}

fn rewrite_stmt(stmt: &mut JsStmt) {
    match stmt {
        JsStmt::VarDecl { init, .. } => {
            if let Some(expr) = init {
                rewrite_expr(expr);
            }
        }
        JsStmt::Assign { target, value } => {
            rewrite_expr(target);
            rewrite_expr(value);
        }
        JsStmt::CompoundAssign { target, value, .. } => {
            rewrite_expr(target);
            rewrite_expr(value);
        }
        JsStmt::Expr(e) => rewrite_expr(e),
        JsStmt::Return(Some(e)) => rewrite_expr(e),
        JsStmt::Return(None) => {}
        JsStmt::If {
            cond,
            then_body,
            else_body,
        } => {
            rewrite_expr(cond);
            rewrite_stmts(then_body);
            rewrite_stmts(else_body);
        }
        JsStmt::While { cond, body } => {
            rewrite_expr(cond);
            rewrite_stmts(body);
        }
        JsStmt::For {
            init,
            cond,
            update,
            body,
        } => {
            rewrite_stmts(init);
            rewrite_expr(cond);
            rewrite_stmts(update);
            rewrite_stmts(body);
        }
        JsStmt::Loop { body } => {
            rewrite_stmts(body);
        }
        JsStmt::ForOf { iterable, body, .. } => {
            rewrite_expr(iterable);
            rewrite_stmts(body);
        }
        JsStmt::Throw(e) => rewrite_expr(e),
        JsStmt::Dispatch { blocks, .. } => {
            for (_, stmts) in blocks {
                rewrite_stmts(stmts);
            }
        }
        JsStmt::Break | JsStmt::Continue | JsStmt::LabeledBreak { .. } => {}
    }
}

fn rewrite_expr(expr: &mut JsExpr) {
    // First, recurse into children.
    rewrite_expr_children(expr);

    // Then, attempt to resolve SystemCall patterns.
    let replacement = match expr {
        JsExpr::SystemCall {
            system,
            method,
            args,
        } => try_rewrite_system_call(system, method, args),
        _ => None,
    };

    if let Some(new_expr) = replacement {
        *expr = new_expr;
    }
}

fn rewrite_expr_children(expr: &mut JsExpr) {
    match expr {
        JsExpr::Binary { lhs, rhs, .. } | JsExpr::Cmp { lhs, rhs, .. } => {
            rewrite_expr(lhs);
            rewrite_expr(rhs);
        }
        JsExpr::LogicalOr { lhs, rhs } | JsExpr::LogicalAnd { lhs, rhs } => {
            rewrite_expr(lhs);
            rewrite_expr(rhs);
        }
        JsExpr::Unary { expr: inner, .. } => rewrite_expr(inner),
        JsExpr::Not(inner) | JsExpr::PostIncrement(inner) => rewrite_expr(inner),
        JsExpr::Field { object, .. } => rewrite_expr(object),
        JsExpr::Index { collection, index } => {
            rewrite_expr(collection);
            rewrite_expr(index);
        }
        JsExpr::Call { callee, args } => {
            rewrite_expr(callee);
            for arg in args {
                rewrite_expr(arg);
            }
        }
        JsExpr::Ternary {
            cond,
            then_val,
            else_val,
        } => {
            rewrite_expr(cond);
            rewrite_expr(then_val);
            rewrite_expr(else_val);
        }
        JsExpr::ArrayInit(items) | JsExpr::TupleInit(items) => {
            for item in items {
                rewrite_expr(item);
            }
        }
        JsExpr::ObjectInit(fields) => {
            for (_, val) in fields {
                rewrite_expr(val);
            }
        }
        JsExpr::New { callee, args } => {
            rewrite_expr(callee);
            for arg in args {
                rewrite_expr(arg);
            }
        }
        JsExpr::TypeOf(inner) => rewrite_expr(inner),
        JsExpr::In { key, object } => {
            rewrite_expr(key);
            rewrite_expr(object);
        }
        JsExpr::Delete { object, key } => {
            rewrite_expr(object);
            rewrite_expr(key);
        }
        JsExpr::Cast { expr: inner, .. } | JsExpr::TypeCheck { expr: inner, .. } => {
            rewrite_expr(inner)
        }
        JsExpr::ArrowFunction { body, .. } => rewrite_stmts(body),
        JsExpr::SuperCall(args) | JsExpr::SuperMethodCall { args, .. } => {
            for arg in args {
                rewrite_expr(arg);
            }
        }
        JsExpr::SuperGet(_) => {}
        JsExpr::SuperSet { value, .. } => rewrite_expr(value),
        JsExpr::GeneratorCreate { args, .. } => {
            for arg in args {
                rewrite_expr(arg);
            }
        }
        JsExpr::GeneratorResume(inner) => rewrite_expr(inner),
        JsExpr::Yield(inner) => {
            if let Some(e) = inner {
                rewrite_expr(e);
            }
        }
        JsExpr::Activation => {}
        JsExpr::SystemCall { args, .. } => {
            for arg in args {
                rewrite_expr(arg);
            }
        }
        // Leaf nodes — nothing to recurse into.
        JsExpr::Literal(_) | JsExpr::Var(_) | JsExpr::This => {}
    }
}

/// Try to rewrite a GameMaker SystemCall into a native JS expression.
fn try_rewrite_system_call(
    system: &str,
    method: &str,
    args: &mut Vec<JsExpr>,
) -> Option<JsExpr> {
    match (system, method) {
        // GameMaker.Global.set(name, val) → variable_global_set(name, val)
        ("GameMaker.Global", "set") if args.len() == 2 => {
            let val = args.pop().unwrap();
            let name = args.pop().unwrap();
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("variable_global_set".into())),
                args: vec![name, val],
            })
        }
        // GameMaker.Global.get(name) → variable_global_get(name)
        ("GameMaker.Global", "get") if args.len() == 1 => {
            let name = args.pop().unwrap();
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("variable_global_get".into())),
                args: vec![name],
            })
        }
        // GameMaker.Instance.getOn(objId, field) → getInstanceField(objId, field)
        ("GameMaker.Instance", "getOn") if args.len() == 2 => {
            let field = args.pop().unwrap();
            let obj_id = args.pop().unwrap();
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("getInstanceField".into())),
                args: vec![obj_id, field],
            })
        }
        // GameMaker.Instance.setOn(objId, field, val) → setInstanceField(objId, field, val)
        ("GameMaker.Instance", "setOn") if args.len() == 3 => {
            let val = args.pop().unwrap();
            let field = args.pop().unwrap();
            let obj_id = args.pop().unwrap();
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("setInstanceField".into())),
                args: vec![obj_id, field, val],
            })
        }
        // GameMaker.Instance.getOther(field) → other[field]
        ("GameMaker.Instance", "getOther") if args.len() == 1 => {
            let field = args.pop().unwrap();
            Some(JsExpr::Index {
                collection: Box::new(JsExpr::Var("other".into())),
                index: Box::new(field),
            })
        }
        // GameMaker.Instance.setOther(field, val) → setOtherField(other, field, val)
        ("GameMaker.Instance", "setOther") if args.len() == 2 => {
            let val = args.pop().unwrap();
            let field = args.pop().unwrap();
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("setOtherField".into())),
                args: vec![JsExpr::Var("other".into()), field, val],
            })
        }
        // GameMaker.Instance.getAll(field) → getAllField(field)
        ("GameMaker.Instance", "getAll") if args.len() == 1 => {
            let field = args.pop().unwrap();
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("getAllField".into())),
                args: vec![field],
            })
        }
        // GameMaker.Instance.setAll(field, val) → setAllField(field, val)
        ("GameMaker.Instance", "setAll") if args.len() == 2 => {
            let val = args.pop().unwrap();
            let field = args.pop().unwrap();
            Some(JsExpr::Call {
                callee: Box::new(JsExpr::Var("setAllField".into())),
                args: vec![field, val],
            })
        }
        // GameMaker.Instance.withBegin/withEnd — complex block construct, pass through.
        ("GameMaker.Instance", "withBegin") => None,
        ("GameMaker.Instance", "withEnd") => None,
        _ => None,
    }
}
