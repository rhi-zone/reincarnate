//! Harlowe story format parser and IR lowering.
//!
//! Harlowe uses a hook-based macro syntax with `(macro:)` calls and
//! `[hook]` content blocks. Its expression language is distinct from
//! JavaScript — it uses `is`, `is not`, `contains`, `is in`, etc.

pub mod ast;
pub mod expr;
pub mod lexer;
pub mod macros;
pub mod parser;
pub mod translate;

use ast::PassageAst;

/// Parse a single Harlowe passage into an AST.
pub fn parse_passage(source: &str) -> PassageAst {
    parser::parse(source)
}
