//! # gen-gml-builtins
//!
//! Parses the GML manual HTML (from `.gml-manual/`, a gitignored clone of
//! github.com/YoYoGames/GameMaker-Manual) and generates two committed output files:
//!
//! - `crates/frontends/reincarnate-frontend-gamemaker/builtins.json`
//! - `crates/frontends/reincarnate-frontend-gamemaker/src/builtins_generated.rs`
//!
//! ## Why the outputs are committed to git
//!
//! The `.gml-manual/` clone is gitignored because it is a large external repository
//! that CI and other contributors do not need to clone. The generated files are
//! committed so that downstream builds never require a local manual clone. They also
//! capture the exact spec version we are targeting, and `builtins_generated.rs` is the
//! authoritative record of GML function signatures in this codebase. IR function bodies
//! are written manually and cross-validated against this table at test time.
//!
//! ## Regenerating
//!
//! ```bash
//! # Clone the manual (once):
//! git clone https://github.com/YoYoGames/GameMaker-Manual .gml-manual
//!
//! # Regenerate:
//! cargo run -p gen-gml-builtins
//! cargo fmt --all
//! ```

use std::collections::{BTreeMap, HashMap};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{bail, Context, Result};
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};

// ── Data structures ──────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GmlParam {
    name: String,
    /// Normalized type string (e.g. "Real", "Id", "String").
    type_key: String,
    optional: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GmlFunction {
    name: String,
    params: Vec<GmlParam>,
    /// Normalized return type string.
    return_type: String,
    variadic: bool,
    /// Alternate spellings listed in the `rh-index-keywords` meta tag (e.g.
    /// `color_get_red` alongside the canonical `colour_get_red`).
    #[serde(default)]
    aliases: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
struct Output {
    source: String,
    note: String,
    functions: Vec<GmlFunction>,
}

// ── Type mapping ─────────────────────────────────────────────────────────────

/// Map a `data-keyref` attribute value to a normalized type string.
fn normalize_type(keyref: &str) -> &'static str {
    if keyref == "Type_Real" {
        return "Real";
    }
    if keyref == "Type_Int" {
        return "Int";
    }
    if keyref == "Type_Bool" {
        return "Bool";
    }
    if keyref == "Type_String" {
        return "String";
    }
    if keyref == "Type_Void" {
        return "Void";
    }
    if keyref == "Type_Undefined" || keyref == "Type_Any" {
        return "Any";
    }
    if keyref == "Type_Array" {
        return "Array";
    }
    if keyref == "Type_Struct" {
        return "Struct";
    }
    if keyref.starts_with("Type_ID_") {
        return "Id";
    }
    if keyref.starts_with("Type_Asset_") {
        return "Asset";
    }
    if keyref.starts_with("Type_Constant_") {
        return "Constant";
    }
    if keyref.starts_with("Type_Resource_") {
        return "Resource";
    }
    "Unknown"
}

/// Map a normalized type string to a Rust `Type` expression.
fn rust_type(type_key: &str) -> &'static str {
    match type_key {
        "Real" => "Type::Float(64)",
        "Int" => "Type::Int(32)",
        "Bool" => "Type::Bool",
        "String" => "Type::String",
        "Void" => "Type::Void",
        "Array" => "Type::Array(Box::new(Type::Unknown))",
        "Id" | "Asset" | "Constant" | "Resource" => "Type::Int(32)",
        // Struct, Any, Unknown, and anything else
        _ => "Type::Unknown",
    }
}

// ── HTML parsing ─────────────────────────────────────────────────────────────

/// Extract alternate names from `<meta name="rh-index-keywords" content="..." />`.
///
/// Returns names listed in the comma-separated `content` attribute that differ
/// from `canonical` (after trimming).  Returns an empty vec if the tag is absent
/// or contains no additional names.
fn extract_aliases(content: &str, canonical: &str) -> Vec<String> {
    // Find the rh-index-keywords meta tag via a simple string search — faster
    // than a full CSS selector and sufficient given the known format.
    let marker = "name=\"rh-index-keywords\"";
    let pos = match content.find(marker) {
        Some(p) => p,
        None => return Vec::new(),
    };
    let rest = &content[pos + marker.len()..];
    // Find content="..."
    let content_attr = "content=\"";
    let c_pos = match rest.find(content_attr) {
        Some(p) => p,
        None => return Vec::new(),
    };
    let after = &rest[c_pos + content_attr.len()..];
    let end = match after.find('"') {
        Some(e) => e,
        None => return Vec::new(),
    };
    let keywords = &after[..end];
    keywords
        .split(',')
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty() && s != canonical)
        .collect()
}

/// Parse one `.htm` file. Returns `None` if it is not a function page.
fn parse_page(path: &Path) -> Result<Option<GmlFunction>> {
    let content =
        fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;

    // Quick pre-filter: must have a Syntax: heading (either plain or bolded variant).
    let has_syntax =
        content.contains("<h4>Syntax:</h4>") || content.contains("<h4><b>Syntax:</b></h4>");
    if !has_syntax {
        return Ok(None);
    }

    let doc = Html::parse_document(&content);

    // ── Function name ────────────────────────────────────────────────────────
    let title_sel = Selector::parse("span[data-field='title'][data-format='default']").unwrap();
    let name = match doc.select(&title_sel).next() {
        Some(el) => el.text().collect::<String>().trim().to_owned(),
        None => return Ok(None),
    };

    // Skip if name is the template placeholder "title" (RoboHelp pages where the
    // data-field="title" span was never substituted with the actual function name).
    if name == "title" {
        return Ok(None);
    }

    // Function names must be valid GML identifiers: start with letter or underscore,
    // contain only ASCII word characters. Anything else is an index/overview page.
    if name.is_empty()
        || !name.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
        || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
    {
        return Ok(None);
    }

    // ── Verify this is a function page (has a Syntax: heading) ─────────────
    // Two variants appear in the manual:
    //   <h4>Syntax:</h4>             — used by most pages
    //   <h4><b>Syntax:</b></h4>      — used by ~54 older pages (e.g. abs.htm)
    let h4_sel = Selector::parse("h4").unwrap();
    let has_syntax_h4 = doc
        .select(&h4_sel)
        .any(|el| el.text().collect::<String>().trim() == "Syntax:");
    if !has_syntax_h4 {
        return Ok(None);
    }

    // ── Syntax line (detect variadic) ────────────────────────────────────────
    // The syntax line is the first <p class="code"> after <h4>Syntax:</h4>.
    // We do this with a raw string search because scraper doesn't support
    // adjacent-sibling selectors well for this pattern.
    let syntax_line = extract_syntax_line(&content);
    let variadic = syntax_line
        .as_deref()
        .map(|s| s.contains("..."))
        .unwrap_or(false);

    // ── Argument table ───────────────────────────────────────────────────────
    let params = parse_arg_table(&doc)?;

    // ── Return type ──────────────────────────────────────────────────────────
    let return_type = parse_return_type(&content);

    // ── Aliases from rh-index-keywords meta tag ──────────────────────────────
    let aliases = extract_aliases(&content, &name);

    Ok(Some(GmlFunction {
        name,
        params,
        return_type,
        variadic,
        aliases,
    }))
}

/// Extract the raw text of the `<p class="code">` that immediately follows
/// `<h4>Syntax:</h4>` (or `<h4><b>Syntax:</b></h4>`) in the source.
fn extract_syntax_line(content: &str) -> Option<String> {
    // Try both heading variants.
    let after = find_after_syntax_h4(content)?;

    let p_start = after.find("<p class=\"code\">")?;
    let p_content = &after[p_start + "<p class=\"code\">".len()..];
    let p_end = p_content.find("</p>")?;
    Some(p_content[..p_end].to_owned())
}

/// Return the slice of `content` immediately after the Syntax: heading.
fn find_after_syntax_h4(content: &str) -> Option<&str> {
    // Plain variant first (most common).
    if let Some(i) = content.find("<h4>Syntax:</h4>") {
        return Some(&content[i + "<h4>Syntax:</h4>".len()..]);
    }
    // Bold variant (<h4><b>Syntax:</b></h4>).
    if let Some(i) = content.find("<h4><b>Syntax:</b></h4>") {
        return Some(&content[i + "<h4><b>Syntax:</b></h4>".len()..]);
    }
    None
}

/// Parse the argument table from the page. Returns an empty vec if none found.
fn parse_arg_table(doc: &Html) -> Result<Vec<GmlParam>> {
    let table_sel = Selector::parse("table").unwrap();
    let tr_sel = Selector::parse("tr").unwrap();
    let th_sel = Selector::parse("th").unwrap();
    let td_sel = Selector::parse("td").unwrap();
    let span_sel = Selector::parse("span[data-keyref]").unwrap();

    let mut params = Vec::new();

    for table in doc.select(&table_sel) {
        // Check if this table has the expected header row: Argument | Type | Description
        let rows: Vec<_> = table.select(&tr_sel).collect();
        if rows.is_empty() {
            continue;
        }
        let header = &rows[0];
        let headers: Vec<String> = header
            .select(&th_sel)
            .map(|th| th.text().collect::<String>().trim().to_owned())
            .collect();

        // Must have at least 3 columns: Argument, Type, Description
        if headers.len() < 3 {
            continue;
        }
        if headers[0] != "Argument" || headers[1] != "Type" {
            continue;
        }

        // Process data rows
        for row in rows.iter().skip(1) {
            let cells: Vec<_> = row.select(&td_sel).collect();
            if cells.len() < 3 {
                continue;
            }

            // col 0: parameter name. Some optional params are written as `[asset_type]`
            // with surrounding brackets — strip them to get the bare identifier.
            let raw_name = cells[0].text().collect::<String>();
            let raw_name = raw_name.trim();
            let name = raw_name
                .trim_start_matches('[')
                .trim_end_matches(']')
                .trim()
                .to_owned();
            if name.is_empty() {
                continue;
            }

            // col 1: type — extract data-keyref from the <span> inside
            let type_key = cells[1]
                .select(&span_sel)
                .next()
                .and_then(|s| s.value().attr("data-keyref"))
                .map(normalize_type)
                .unwrap_or("Unknown")
                .to_owned();

            // col 2: description — check for Tag_optional
            let desc_html = cells[2].html();
            let optional = desc_html.contains("Tag_optional.hts");

            params.push(GmlParam {
                name,
                type_key,
                optional,
            });
        }

        // Only process the first matching table per page
        if !params.is_empty() {
            break;
        }
    }

    Ok(params)
}

/// Extract the return type from the page, looking for the `Returns:` heading.
/// Handles `<h4>Returns:</h4>`, `<h4><b>Returns:</b></h4>`, and the
/// no-colon variants `<h4>Returns</h4>` and `<h4><b>Returns</b></h4>`.
fn parse_return_type(content: &str) -> String {
    // Try all heading variants (colon and no-colon, plain and bold).
    let after = if let Some(i) = content.find("<h4>Returns:</h4>") {
        &content[i + "<h4>Returns:</h4>".len()..]
    } else if let Some(i) = content.find("<h4><b>Returns:</b></h4>") {
        &content[i + "<h4><b>Returns:</b></h4>".len()..]
    } else if let Some(i) = content.find("<h4>Returns</h4>") {
        &content[i + "<h4>Returns</h4>".len()..]
    } else if let Some(i) = content.find("<h4><b>Returns</b></h4>") {
        &content[i + "<h4><b>Returns</b></h4>".len()..]
    } else {
        return "Unknown".to_owned();
    };

    // Find the next data-keyref in a <p class="code"> block
    let p_start = match after.find("<p class=\"code\">") {
        Some(i) => i,
        None => return "Unknown".to_owned(),
    };
    let p_content = &after[p_start..];
    let p_end = match p_content.find("</p>") {
        Some(i) => i,
        None => return "Unknown".to_owned(),
    };
    let p_html = &p_content[..p_end];

    // Extract data-keyref="..." from the span
    if let Some(kr_start) = p_html.find("data-keyref=\"") {
        let rest = &p_html[kr_start + "data-keyref=\"".len()..];
        if let Some(kr_end) = rest.find('"') {
            return normalize_type(&rest[..kr_end]).to_owned();
        }
    }

    "Unknown".to_owned()
}

// ── File walking ─────────────────────────────────────────────────────────────

fn walk_htm_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut files = Vec::new();
    walk_dir(dir, &mut files)?;
    files.sort();
    Ok(files)
}

fn walk_dir(dir: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    for entry in fs::read_dir(dir).with_context(|| format!("reading dir {}", dir.display()))? {
        let entry = entry?;
        let path = entry.path();
        if path.is_dir() {
            walk_dir(&path, out)?;
        } else if path.extension().and_then(|e| e.to_str()) == Some("htm") {
            out.push(path);
        }
    }
    Ok(())
}

// ── Code generation ───────────────────────────────────────────────────────────

fn generate_rust(functions: &[GmlFunction]) -> String {
    let mut out = String::new();

    writeln!(
        out,
        "// AUTO-GENERATED by `cargo run -p gen-gml-builtins`. Do not edit by hand."
    )
    .unwrap();
    writeln!(out, "//").unwrap();
    writeln!(out, "// ## Regenerating").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(out, "// ```bash").unwrap();
    writeln!(out, "// cargo run -p gen-gml-builtins").unwrap();
    writeln!(out, "// cargo fmt --all").unwrap();
    writeln!(out, "// ```").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(out, "// ## Why this file is committed to git").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(
        out,
        "// The `.gml-manual/` clone of github.com/YoYoGames/GameMaker-Manual is"
    )
    .unwrap();
    writeln!(
        out,
        "// gitignored: it is a large external repository that CI and other contributors"
    )
    .unwrap();
    writeln!(
        out,
        "// do not need to clone. This generated file is committed so that downstream"
    )
    .unwrap();
    writeln!(out, "// builds never require a local manual clone.").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(
        out,
        "// This file is the authoritative record of GML function signatures in this"
    )
    .unwrap();
    writeln!(
        out,
        "// codebase. IR function bodies are written manually and cross-validated"
    )
    .unwrap();
    writeln!(out, "// against this table at test time.").unwrap();
    writeln!(out, "//").unwrap();
    writeln!(
        out,
        "// Source spec version: github.com/YoYoGames/GameMaker-Manual (see .gml-manual/)."
    )
    .unwrap();
    writeln!(out).unwrap();
    writeln!(out, "#![allow(clippy::all)]").unwrap();
    writeln!(out, "#![allow(dead_code)]").unwrap();
    writeln!(out).unwrap();
    writeln!(out, "use reincarnate_core::ir::ty::{{FunctionSig, Type}};").unwrap();
    writeln!(out, "use reincarnate_core::ir::value::Constant;").unwrap();
    writeln!(out).unwrap();
    writeln!(
        out,
        "/// Returns the full table of GML builtin function signatures parsed from the"
    )
    .unwrap();
    writeln!(
        out,
        "/// GameMaker manual HTML. Each entry is `(name, sig, aliases)`."
    )
    .unwrap();
    writeln!(out, "///").unwrap();
    writeln!(
        out,
        "/// This is a function (not a static) because `FunctionSig` contains `Vec` fields."
    )
    .unwrap();
    writeln!(
        out,
        "pub fn gml_builtins() -> Vec<(&'static str, FunctionSig, &'static [&'static str])> {{"
    )
    .unwrap();
    writeln!(out, "    vec![").unwrap();

    for func in functions {
        let has_defaults = func.params.iter().any(|p| p.optional) || func.variadic;

        writeln!(out, "        ({:?}, FunctionSig {{", func.name).unwrap();

        // params
        writeln!(out, "            params: vec![").unwrap();
        for param in &func.params {
            writeln!(
                out,
                "                {}, // {}",
                rust_type(&param.type_key),
                param.name
            )
            .unwrap();
        }
        writeln!(out, "            ],").unwrap();

        // return_ty
        writeln!(
            out,
            "            return_ty: {},",
            rust_type(&func.return_type)
        )
        .unwrap();

        // defaults
        if has_defaults {
            writeln!(out, "            defaults: vec![").unwrap();
            for param in &func.params {
                if param.optional {
                    // TODO: fill in actual default — using 0.0 placeholder
                    writeln!(
                        out,
                        "                Some(Constant::Float(0.0)), // TODO: actual default for `{}` must be filled in manually",
                        param.name
                    )
                    .unwrap();
                } else {
                    writeln!(out, "                None,").unwrap();
                }
            }
            writeln!(out, "            ],").unwrap();
        } else {
            writeln!(out, "            defaults: vec![],").unwrap();
        }

        // has_rest_param
        writeln!(out, "            has_rest_param: {},", func.variadic).unwrap();

        // param_lower_bounds — always empty for parsed builtins; callers set them programmatically
        writeln!(out, "            param_lower_bounds: vec![],").unwrap();

        // aliases slice
        if func.aliases.is_empty() {
            writeln!(out, "        }}, &[]),").unwrap();
        } else {
            let alias_list: Vec<String> = func.aliases.iter().map(|a| format!("{:?}", a)).collect();
            writeln!(out, "        }}, &[{}]),", alias_list.join(", ")).unwrap();
        }
    }

    writeln!(out, "    ]").unwrap();
    writeln!(out, "}}").unwrap();

    out
}

// ── Entry point ───────────────────────────────────────────────────────────────

fn main() -> Result<()> {
    // Locate workspace root (the directory containing this binary's Cargo.toml
    // chain). We use the CARGO_MANIFEST_DIR env var which Cargo sets at build time,
    // and walk up to find the workspace root.
    let workspace_root = locate_workspace_root()?;

    let manual_dir =
        workspace_root.join(".gml-manual/Manual/contents/GameMaker_Language/GML_Reference");
    if !manual_dir.exists() {
        bail!(
            "GML manual not found at {}.\n\
             Clone it with:\n\
             \n\
             git clone https://github.com/YoYoGames/GameMaker-Manual {}\n",
            manual_dir.display(),
            workspace_root.join(".gml-manual").display(),
        );
    }

    let json_out =
        workspace_root.join("crates/frontends/reincarnate-frontend-gamemaker/builtins.json");
    let rs_out = workspace_root
        .join("crates/frontends/reincarnate-frontend-gamemaker/src/builtins_generated.rs");

    // Walk all .htm files
    eprintln!("Scanning {}...", manual_dir.display());
    let files = walk_htm_files(&manual_dir)?;
    eprintln!("Found {} .htm files", files.len());

    // Parse each file, deduplicating by function name (first path wins since
    // files are sorted alphabetically).
    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    let mut functions: BTreeMap<String, GmlFunction> = BTreeMap::new(); // sorted by name

    for path in &files {
        match parse_page(path) {
            Ok(Some(func)) => {
                if let Some(prev) = seen.get(&func.name) {
                    eprintln!(
                        "WARNING: duplicate function {:?} in {} (previously seen in {}), skipping",
                        func.name,
                        path.display(),
                        prev.display(),
                    );
                } else {
                    seen.insert(func.name.clone(), path.clone());
                    functions.insert(func.name.clone(), func);
                }
            }
            Ok(None) => {} // not a function page
            Err(e) => {
                eprintln!("WARNING: error parsing {}: {e}", path.display());
            }
        }
    }

    let functions: Vec<GmlFunction> = functions.into_values().collect();
    let n = functions.len();

    // ── Write JSON ───────────────────────────────────────────────────────────
    let output = Output {
        source: "github.com/YoYoGames/GameMaker-Manual".to_owned(),
        note: "AUTO-GENERATED by `cargo run -p gen-gml-builtins`. Do not edit by hand. See tools/gen-gml-builtins/src/main.rs for regeneration instructions.".to_owned(),
        functions,
    };
    let json = serde_json::to_string_pretty(&output)?;
    fs::write(&json_out, json + "\n").with_context(|| format!("writing {}", json_out.display()))?;

    // ── Write Rust ───────────────────────────────────────────────────────────
    let rust_src = generate_rust(&output.functions);
    fs::write(&rs_out, rust_src).with_context(|| format!("writing {}", rs_out.display()))?;

    eprintln!(
        "Parsed {} functions, wrote builtins.json and builtins_generated.rs",
        n
    );

    Ok(())
}

/// Walk up from the binary's manifest dir to find the workspace root
/// (the directory containing a `Cargo.toml` with `[workspace]`).
fn locate_workspace_root() -> Result<PathBuf> {
    // At runtime we don't have CARGO_MANIFEST_DIR; use the executable path instead,
    // or fall back to the current working directory (which cargo sets to the workspace
    // root when running `cargo run`).
    //
    // The simplest reliable approach: look for a Cargo.toml with "[workspace]" starting
    // from the current working directory upward.
    let cwd = std::env::current_dir().context("getting current directory")?;
    let mut dir = cwd.as_path();
    loop {
        let candidate = dir.join("Cargo.toml");
        if candidate.exists() {
            let content = fs::read_to_string(&candidate)
                .with_context(|| format!("reading {}", candidate.display()))?;
            if content.contains("[workspace]") {
                return Ok(dir.to_path_buf());
            }
        }
        match dir.parent() {
            Some(p) => dir = p,
            None => bail!("Could not find workspace root (no Cargo.toml with [workspace] found above current directory)"),
        }
    }
}
