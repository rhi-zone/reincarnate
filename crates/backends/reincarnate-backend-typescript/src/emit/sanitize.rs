// ---------------------------------------------------------------------------
// Identifier sanitization
// ---------------------------------------------------------------------------

use std::collections::{BTreeSet, HashMap, HashSet};

use reincarnate_core::ir::{Constant, FuncId, Module, Op};

/// JS/TS reserved words that cannot be used as identifiers.
///
/// Includes ECMAScript reserved words, strict-mode reserved words, and
/// TypeScript contextual keywords that cause parse errors as identifiers.
const JS_RESERVED: &[&str] = &[
    "arguments",
    "async",
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "eval",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "implements",
    "import",
    "in",
    "instanceof",
    "interface",
    "let",
    "new",
    "null",
    "package",
    "private",
    "protected",
    "public",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Sanitize a name into a valid JavaScript identifier.
///
/// Replaces non-alphanumeric characters with `_` and prefixes with `_` if the
/// name starts with a digit or is a reserved word.
pub(crate) fn sanitize_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for (i, ch) in name.chars().enumerate() {
        if i == 0 {
            // Allow digits at start — handled by the digit-start prepend below.
            if unicode_ident::is_xid_start(ch)
                || unicode_ident::is_xid_continue(ch)
                || ch == '_'
                || ch == '$'
            {
                out.push(ch);
            } else {
                out.push('_');
            }
        } else if unicode_ident::is_xid_continue(ch) || ch == '$' {
            out.push(ch);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        return "_".to_string();
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if JS_RESERVED.contains(&out.as_str()) {
        out.insert(0, '_');
    }
    out
}

/// Rename free functions whose sanitized name collides with a class name or
/// global variable name, avoiding TS2308/TS2440 duplicate-export errors.
/// Renames both `func.name` and all `Op::Call` references throughout the module.
pub(super) fn rename_colliding_free_funcs(
    module: &mut Module,
    free_funcs: &[FuncId],
    known_classes: &HashSet<String>,
) {
    let sanitized_global_names: HashSet<String> = module
        .globals
        .iter()
        .map(|g| sanitize_ident(&g.name))
        .collect();
    // Build (old_name → new_name) pairs for colliding free functions.
    let renames: HashMap<String, String> = free_funcs
        .iter()
        .filter_map(|&fid| {
            let raw = sanitize_ident(&module.functions[fid].name);
            if known_classes.contains(&raw) || sanitized_global_names.contains(&raw) {
                Some((module.functions[fid].name.clone(), format!("{raw}__fn")))
            } else {
                None
            }
        })
        .collect();
    if !renames.is_empty() {
        // Rename the function definitions.
        for fid in free_funcs {
            let name = &module.functions[*fid].name;
            if let Some(new_name) = renames.get(name) {
                module.functions[*fid].name = new_name.clone();
            }
        }
        // Update all Op::Call references throughout the module.
        let all_fids: Vec<FuncId> = module.functions.keys().collect();
        for fid in all_fids {
            for inst in module.functions[fid].insts.values_mut() {
                if let Op::Call { func, .. } = &mut inst.op {
                    if let Some(new_name) = renames.get(func.as_str()) {
                        func.clone_from(new_name);
                    }
                }
            }
        }
    }
}

/// Rename local variables (IR value names) that shadow imported function names.
///
/// GML allows local variables to share names with built-in functions (e.g. a
/// local `int` alongside calls to `int()`). In the emitted TypeScript, the
/// import `import { int } from "..."` is file-scoped, so a local `let int = ...`
/// shadows it, causing TS2349 ("This expression is not callable").
///
/// This pass prefixes colliding value names with `_`.
pub(super) fn rename_shadowing_locals(module: &mut Module, imported_names: &BTreeSet<String>) {
    for func in module.functions.values_mut() {
        for name in func.value_names.values_mut() {
            if imported_names.contains(name.as_str()) {
                *name = format!("_{name}");
            }
        }
    }
}

/// Resolve a sprite constant reference: if the field name is `sprite_index`
/// and the constant value is a non-negative integer matching an index in
/// `sprite_names`, return `Sprites.SpriteName` or `Sprites["name"]`.
pub(super) fn resolve_sprite_constant(
    name: &str,
    val: &Constant,
    sprite_names: &[String],
) -> Option<String> {
    if name != "sprite_index" || sprite_names.is_empty() {
        return None;
    }
    if let Constant::Int(idx) = val {
        let idx = *idx as usize;
        sprite_names.get(idx).map(|n| {
            if crate::ast_printer::is_valid_js_ident(n) {
                format!("Sprites.{n}")
            } else {
                let quoted = serde_json::to_string(n).expect("string serialization cannot fail");
                format!("Sprites[{quoted}]")
            }
        })
    } else {
        None
    }
}
