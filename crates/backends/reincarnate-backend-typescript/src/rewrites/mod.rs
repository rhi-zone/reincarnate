//! Engine-specific rewrite modules.
//!
//! Each module converts engine-specific `SystemCall` nodes into native JS
//! constructs during the `lower` pass. The rewrites are scoped: the lowering
//! pass only activates Flash rewrites when a Flash runtime is present.

pub mod flash;
pub mod gamemaker;
pub mod twine;

use crate::js_ast::JsExpr;

/// Pop an argument from a system call's argument list.
/// Panics with a descriptive message if the list is empty.
pub(crate) fn take_arg(args: &mut Vec<JsExpr>, call_name: &str) -> JsExpr {
    args.pop().unwrap_or_else(|| {
        panic!("{call_name}: argument list exhausted (no more arguments to pop)")
    })
}
