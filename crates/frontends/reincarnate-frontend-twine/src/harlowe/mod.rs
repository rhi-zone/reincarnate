//! Harlowe story format parser and IR lowering.
//!
//! Harlowe uses a hook-based macro syntax with `(macro:)` calls and
//! `[hook]` content blocks. Its expression language is distinct from
//! JavaScript — it uses `is`, `is not`, `contains`, `is in`, etc.

#[allow(dead_code)]
pub mod ast;
#[allow(dead_code)]
pub mod expr;
#[allow(dead_code)]
pub mod lexer;
#[allow(dead_code)]
pub mod macros;
#[allow(dead_code)]
pub mod parser;
#[allow(dead_code)]
pub mod translate;

use ast::PassageAst;

/// Parse a single Harlowe passage into an AST.
#[allow(dead_code)]
pub fn parse_passage(source: &str) -> PassageAst {
    parser::parse(source)
}
