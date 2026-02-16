//! Harlowe AST → IR translation.
//!
//! Translates parsed Harlowe passage AST nodes into reincarnate-core IR
//! functions using SystemCall-based dispatch to the Harlowe runtime layer.
//!
//! Content nodes are IR values (Type::Dynamic returned from SystemCalls).
//! The backend's SSA inlining produces nested expressions from flat IR
//! values. Two synthetic SystemCalls are rewritten by the backend:
//! - `Harlowe.Output.content_array(a, b, c)` → `[a, b, c]` (ArrayInit)
//! - `Harlowe.Output.text_node(s)` → `s` (identity — strings ARE nodes)
//!
//! SystemCall namespaces:
//! - `Harlowe.State`: get/set story and temp variables
//! - `Harlowe.Output`: content builders (color, strong, br, link, etc.)
//! - `Harlowe.Navigation`: goto
//! - `Harlowe.Engine`: changers, data ops, runtime helpers

use std::collections::HashMap;

use reincarnate_core::ir::{BlockId, CmpKind, Function, FunctionBuilder, FunctionSig, Type, ValueId, Visibility};

use super::ast::*;
use super::macros::{self, MacroKind};

/// Result of translating a Harlowe passage.
pub struct TranslateResult {
    /// The main passage function.
    pub func: Function,
    /// Callback functions generated for link hooks, live intervals, etc.
    pub callbacks: Vec<Function>,
}

/// Sanitize a passage name into a valid function name.
pub fn passage_func_name(name: &str) -> String {
    let sanitized: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
        .collect();
    format!("passage_{sanitized}")
}

/// Translate a parsed Harlowe passage AST into an IR Function.
pub fn translate_passage(name: &str, ast: &PassageAst) -> TranslateResult {
    let func_name = passage_func_name(name);
    let sig = FunctionSig {
        params: vec![],
        return_ty: Type::Dynamic,
        defaults: vec![],
        has_rest_param: false,
    };
    let fb = FunctionBuilder::new(&func_name, sig, Visibility::Public);
    let mut ctx = TranslateCtx {
        fb,
        temp_vars: HashMap::new(),
        func_name: func_name.clone(),
        callback_count: 0,
        callbacks: Vec::new(),
        set_target: None,
    };

    let content = ctx.lower_content_array(&ast.body);
    ctx.fb.ret(Some(content));

    let callbacks = std::mem::take(&mut ctx.callbacks);

    TranslateResult {
        func: ctx.fb.build(),
        callbacks,
    }
}

struct TranslateCtx {
    fb: FunctionBuilder,
    temp_vars: HashMap<String, ValueId>,
    func_name: String,
    callback_count: usize,
    callbacks: Vec<Function>,
    /// Inside `(set: $x to ...)`, holds the target expression so that
    /// `it` resolves to $x's current value rather than the global `it`.
    set_target: Option<Expr>,
}

impl TranslateCtx {
    // ── Content array helper ────────────────────────────────────────

    /// Lower a sequence of nodes into a single content_array value.
    /// Filters out None results (side-effect-only nodes like `set`).
    fn lower_content_array(&mut self, nodes: &[Node]) -> ValueId {
        let vals: Vec<ValueId> = self.lower_nodes_to_structured(nodes);
        self.fb
            .system_call("Harlowe.Output", "content_array", &vals, Type::Dynamic)
    }

    // ── Node lowering ──────────────────────────────────────────────

    /// Lower nodes, pairing HtmlOpen/HtmlClose into structured elements.
    fn lower_nodes_to_structured(&mut self, nodes: &[Node]) -> Vec<ValueId> {
        let mut vals = Vec::new();
        let mut i = 0;
        while i < nodes.len() {
            match &nodes[i].kind {
                NodeKind::HtmlOpen { tag, attrs } => {
                    // Consume children until matching HtmlClose
                    let tag = tag.clone();
                    let attrs = attrs.clone();
                    let (children_end, children) =
                        self.collect_html_children(nodes, i + 1, &tag);
                    let children_arr = self.fb.system_call(
                        "Harlowe.Output",
                        "content_array",
                        &children,
                        Type::Dynamic,
                    );
                    let tag_val = self.fb.const_string(&tag);
                    let mut args = vec![tag_val, children_arr];
                    for (k, v) in &attrs {
                        args.push(self.fb.const_string(k));
                        args.push(self.fb.const_string(v));
                    }
                    let el = self.fb.system_call(
                        "Harlowe.Output",
                        "el",
                        &args,
                        Type::Dynamic,
                    );
                    vals.push(el);
                    i = children_end;
                }
                NodeKind::HtmlClose(_) => {
                    // Stray close tag — skip
                    i += 1;
                }
                _ => {
                    if let Some(v) = self.lower_node(&nodes[i]) {
                        vals.push(v);
                    }
                    i += 1;
                }
            }
        }
        vals
    }

    /// Collect children between HtmlOpen and matching HtmlClose.
    /// Returns (next_index_after_close, children_vals).
    fn collect_html_children(
        &mut self,
        nodes: &[Node],
        start: usize,
        close_tag: &str,
    ) -> (usize, Vec<ValueId>) {
        let mut vals = Vec::new();
        let mut i = start;
        while i < nodes.len() {
            match &nodes[i].kind {
                NodeKind::HtmlClose(tag) if tag == close_tag => {
                    return (i + 1, vals);
                }
                NodeKind::HtmlOpen { tag, attrs } => {
                    let tag = tag.clone();
                    let attrs = attrs.clone();
                    let (end, children) =
                        self.collect_html_children(nodes, i + 1, &tag);
                    let children_arr = self.fb.system_call(
                        "Harlowe.Output",
                        "content_array",
                        &children,
                        Type::Dynamic,
                    );
                    let tag_val = self.fb.const_string(&tag);
                    let mut args = vec![tag_val, children_arr];
                    for (k, v) in &attrs {
                        args.push(self.fb.const_string(k));
                        args.push(self.fb.const_string(v));
                    }
                    let el = self.fb.system_call(
                        "Harlowe.Output",
                        "el",
                        &args,
                        Type::Dynamic,
                    );
                    vals.push(el);
                    i = end;
                }
                NodeKind::HtmlClose(_) => {
                    // Mismatched close tag — stop here
                    return (i + 1, vals);
                }
                _ => {
                    if let Some(v) = self.lower_node(&nodes[i]) {
                        vals.push(v);
                    }
                    i += 1;
                }
            }
        }
        // Ran out of nodes without finding close tag
        (i, vals)
    }

    fn lower_node(&mut self, node: &Node) -> Option<ValueId> {
        match &node.kind {
            NodeKind::Text(text) => {
                if text.is_empty() {
                    None
                } else {
                    Some(self.emit_text_node(text))
                }
            }
            NodeKind::Macro(mac) => self.lower_macro(mac),
            NodeKind::Hook(nodes) => Some(self.lower_content_array(nodes)),
            NodeKind::Link(link) => Some(self.lower_link(link)),
            NodeKind::VarInterp(name) => Some(self.lower_var_interp(name)),
            NodeKind::HtmlOpen { tag, attrs } => {
                // Standalone open tag without matching close — emit as element
                let tag_val = self.fb.const_string(tag);
                let empty = self.fb.system_call(
                    "Harlowe.Output",
                    "content_array",
                    &[],
                    Type::Dynamic,
                );
                let mut args = vec![tag_val, empty];
                for (k, v) in attrs {
                    args.push(self.fb.const_string(k));
                    args.push(self.fb.const_string(v));
                }
                Some(
                    self.fb
                        .system_call("Harlowe.Output", "el", &args, Type::Dynamic),
                )
            }
            NodeKind::HtmlClose(_) => None,
            NodeKind::HtmlVoid { tag, attrs } => Some(self.emit_void_element(tag, attrs)),
            NodeKind::Markup { tag, body } => {
                let method = match tag.as_str() {
                    "strong" | "em" | "del" | "sup" | "sub" => tag.as_str(),
                    _ => "el",
                };
                if method == "el" {
                    let children = self.lower_content_array(body);
                    let tag_val = self.fb.const_string(tag);
                    Some(self.fb.system_call(
                        "Harlowe.Output",
                        "el",
                        &[tag_val, children],
                        Type::Dynamic,
                    ))
                } else {
                    let children = self.lower_content_array(body);
                    Some(self.fb.system_call(
                        "Harlowe.Output",
                        method,
                        &[children],
                        Type::Dynamic,
                    ))
                }
            }
            NodeKind::ChangerApply { name, hook } => {
                let changer = if let Some(stripped) = name.strip_prefix('$') {
                    let n = self.fb.const_string(stripped);
                    self.fb
                        .system_call("Harlowe.State", "get", &[n], Type::Dynamic)
                } else {
                    let stripped = name.strip_prefix('_').unwrap_or(name);
                    self.get_or_load_temp(stripped)
                };
                let children = self.lower_content_array(hook);
                Some(self.fb.system_call(
                    "Harlowe.Output",
                    "styled",
                    &[changer, children],
                    Type::Dynamic,
                ))
            }
            NodeKind::LineBreak => {
                Some(
                    self.fb
                        .system_call("Harlowe.Output", "br", &[], Type::Dynamic),
                )
            }
        }
    }

    // ── Output helpers ─────────────────────────────────────────────

    /// Create a text_node value (backend rewrites to identity — string IS the node).
    fn emit_text_node(&mut self, text: &str) -> ValueId {
        let s = self.fb.const_string(text);
        self.fb
            .system_call("Harlowe.Output", "text_node", &[s], Type::Dynamic)
    }

    fn emit_void_element(&mut self, tag: &str, attrs: &[(String, String)]) -> ValueId {
        match tag {
            "br" => self.fb.system_call("Harlowe.Output", "br", &[], Type::Dynamic),
            "hr" => self.fb.system_call("Harlowe.Output", "hr", &[], Type::Dynamic),
            "img" => {
                // Extract src from attrs
                let src = attrs
                    .iter()
                    .find(|(k, _)| k == "src")
                    .map(|(_, v)| v.as_str())
                    .unwrap_or("");
                let src_val = self.fb.const_string(src);
                self.fb
                    .system_call("Harlowe.Output", "img", &[src_val], Type::Dynamic)
            }
            _ => {
                let tag_val = self.fb.const_string(tag);
                let mut args = vec![tag_val];
                for (k, v) in attrs {
                    args.push(self.fb.const_string(k));
                    args.push(self.fb.const_string(v));
                }
                self.fb
                    .system_call("Harlowe.Output", "voidEl", &args, Type::Dynamic)
            }
        }
    }

    // ── Variable interpolation ─────────────────────────────────────

    fn lower_var_interp(&mut self, name: &str) -> ValueId {
        // `$var` or `_var` in body text → get + printVal
        let val = if let Some(stripped) = name.strip_prefix('$') {
            let n = self.fb.const_string(stripped);
            self.fb
                .system_call("Harlowe.State", "get", &[n], Type::Dynamic)
        } else if let Some(stripped) = name.strip_prefix('_') {
            self.get_or_load_temp(stripped)
        } else {
            let n = self.fb.const_string(name);
            self.fb
                .system_call("Harlowe.State", "get", &[n], Type::Dynamic)
        };
        self.fb
            .system_call("Harlowe.Output", "printVal", &[val], Type::Dynamic)
    }

    // ── Macro lowering ─────────────────────────────────────────────

    fn lower_macro(&mut self, mac: &MacroNode) -> Option<ValueId> {
        match mac.name.as_str() {
            // State (side effects — return None)
            "set" => {
                self.lower_set(mac);
                None
            }
            "put" => {
                self.lower_put(mac);
                None
            }

            // Control flow
            "if" | "unless" => self.lower_if(mac),

            // Navigation (side effect — return None)
            "goto" | "go-to" => {
                self.lower_goto(mac);
                None
            }

            // Display (returns content)
            "display" => Some(self.lower_display(mac)),

            // Output
            "print" => Some(self.lower_print(mac)),

            // Links (return content)
            "link" => Some(self.lower_link_macro(mac)),
            "link-goto" => Some(self.lower_link_goto(mac)),

            // Changers (return content when hook present, otherwise create changer value)
            "color" | "colour" | "text-colour" | "text-color" | "text-style" | "font"
            | "align" | "transition" | "transition-time" | "transition-arrive"
            | "transition-depart" | "text-rotate-z" | "hover-style" | "css" | "background"
            | "opacity" | "text-size" | "collapse" | "nobr" | "hidden" => {
                self.lower_changer(mac)
            }

            // Timed
            "live" => Some(self.lower_live(mac)),
            "stop" => {
                self.fb
                    .system_call("Harlowe.Engine", "stop", &[], Type::Void);
                None
            }

            // Value macros (used as expressions — already handled by expr lowering
            // when inside expression context). If they appear standalone, print result.
            "str" | "string" | "num" | "number" | "random" | "either" | "a" | "array"
            | "dm" | "datamap" | "ds" | "dataset" => {
                Some(self.lower_value_macro_standalone(mac))
            }

            // Save/load (side effects)
            "save-game" => {
                self.lower_save_game(mac);
                None
            }
            "load-game" => {
                self.lower_load_game(mac);
                None
            }

            // Alert/prompt/confirm (side effects)
            "alert" => {
                self.lower_simple_command(mac, "Harlowe.Engine", "alert");
                None
            }
            "prompt" => {
                self.lower_simple_command(mac, "Harlowe.Engine", "prompt");
                None
            }
            "confirm" => {
                self.lower_simple_command(mac, "Harlowe.Engine", "confirm");
                None
            }

            // DOM manipulation (side effects)
            "replace" | "append" | "prepend" | "show" | "hide" | "rerun" => {
                self.lower_dom_macro(mac);
                None
            }

            // Click (side effects)
            "click" | "click-replace" | "click-append" | "click-prepend" => {
                self.lower_click_macro(mac);
                None
            }

            // Unknown → generic widget call
            _ => {
                self.lower_unknown_macro(mac);
                None
            }
        }
    }

    // ── (set:) ─────────────────────────────────────────────────────

    fn lower_set(&mut self, mac: &MacroNode) {
        for arg in &mac.args {
            self.lower_assignment(arg);
        }
    }

    fn lower_put(&mut self, mac: &MacroNode) {
        for arg in &mac.args {
            self.lower_assignment(arg);
        }
    }

    fn lower_assignment(&mut self, expr: &Expr) {
        match &expr.kind {
            ExprKind::Assign { target, value } => {
                let prev_target = self.set_target.replace(*target.clone());
                let val = self.lower_expr(value);
                self.set_target = prev_target;
                self.store_to_target(target, val);
            }
            _ => {
                self.lower_expr(expr);
            }
        }
    }

    fn store_to_target(&mut self, target: &Expr, value: ValueId) {
        match &target.kind {
            ExprKind::StoryVar(name) => {
                let n = self.fb.const_string(name.as_str());
                self.fb
                    .system_call("Harlowe.State", "set", &[n, value], Type::Void);
            }
            ExprKind::TempVar(name) => {
                let alloc = self.get_or_create_temp(name);
                self.fb.store(alloc, value);
            }
            ExprKind::Possessive { object, property } => {
                let obj = self.lower_expr(object);
                let prop = self.lower_expr(property);
                self.fb.system_call(
                    "Harlowe.Engine",
                    "set_property",
                    &[obj, prop, value],
                    Type::Void,
                );
            }
            _ => {
                self.lower_expr(target);
            }
        }
    }

    // ── (if:) / (unless:) / (else-if:) / (else:) ──────────────────

    fn lower_if(&mut self, mac: &MacroNode) -> Option<ValueId> {
        let merge_block = self.fb.create_block();
        let merge_params = self.fb.add_block_params(merge_block, &[Type::Dynamic]);
        let merge_param = merge_params[0];

        // Main condition
        let cond = if mac.args.is_empty() {
            self.fb.const_bool(true)
        } else {
            let raw_cond = self.lower_expr(&mac.args[0]);
            if mac.name == "unless" {
                self.fb.system_call(
                    "Harlowe.Engine",
                    "not",
                    &[raw_cond],
                    Type::Bool,
                )
            } else {
                raw_cond
            }
        };

        let then_block = self.fb.create_block();
        let else_block = self.fb.create_block();

        self.fb
            .br_if(cond, then_block, &[], else_block, &[]);

        // Then body (hook)
        self.fb.switch_to_block(then_block);
        let then_val = if let Some(ref hook) = mac.hook {
            self.lower_content_array(hook)
        } else {
            self.fb.system_call(
                "Harlowe.Output",
                "content_array",
                &[],
                Type::Dynamic,
            )
        };
        self.fb.br(merge_block, &[then_val]);

        // Else chain
        self.fb.switch_to_block(else_block);
        self.lower_if_clauses(&mac.clauses, 0, merge_block);

        self.fb.switch_to_block(merge_block);
        Some(merge_param)
    }

    fn lower_if_clauses(&mut self, clauses: &[IfClause], index: usize, merge_block: BlockId) {
        if index >= clauses.len() {
            // No else — produce empty array
            let empty = self.fb.system_call(
                "Harlowe.Output",
                "content_array",
                &[],
                Type::Dynamic,
            );
            self.fb.br(merge_block, &[empty]);
            return;
        }

        let clause = &clauses[index];

        if clause.kind == "else" {
            let val = self.lower_content_array(&clause.body);
            self.fb.br(merge_block, &[val]);
            return;
        }

        // else-if
        let cond = if let Some(ref cond_expr) = clause.cond {
            self.lower_expr(cond_expr)
        } else {
            self.fb.const_bool(true)
        };

        let then_block = self.fb.create_block();
        let next_else = self.fb.create_block();

        self.fb
            .br_if(cond, then_block, &[], next_else, &[]);

        self.fb.switch_to_block(then_block);
        let val = self.lower_content_array(&clause.body);
        self.fb.br(merge_block, &[val]);

        self.fb.switch_to_block(next_else);
        self.lower_if_clauses(clauses, index + 1, merge_block);
    }

    // ── (goto:) ────────────────────────────────────────────────────

    fn lower_goto(&mut self, mac: &MacroNode) {
        if let Some(arg) = mac.args.first() {
            let target = self.lower_expr(arg);
            self.fb
                .system_call("Harlowe.Navigation", "goto", &[target], Type::Void);
        }
    }

    // ── (display:) ─────────────────────────────────────────────────

    fn lower_display(&mut self, mac: &MacroNode) -> ValueId {
        if let Some(arg) = mac.args.first() {
            let target = self.lower_expr(arg);
            self.fb.system_call(
                "Harlowe.Output",
                "displayPassage",
                &[target],
                Type::Dynamic,
            )
        } else {
            self.fb.system_call(
                "Harlowe.Output",
                "content_array",
                &[],
                Type::Dynamic,
            )
        }
    }

    // ── (print:) ───────────────────────────────────────────────────

    fn lower_print(&mut self, mac: &MacroNode) -> ValueId {
        if let Some(arg) = mac.args.first() {
            let val = self.lower_expr(arg);
            self.fb
                .system_call("Harlowe.Output", "printVal", &[val], Type::Dynamic)
        } else {
            self.fb.system_call(
                "Harlowe.Output",
                "content_array",
                &[],
                Type::Dynamic,
            )
        }
    }

    // ── (link:) ────────────────────────────────────────────────────

    fn lower_link_macro(&mut self, mac: &MacroNode) -> ValueId {
        if let Some(arg) = mac.args.first() {
            let text = self.lower_expr(arg);

            if let Some(ref hook) = mac.hook {
                let cb_name = self.make_callback_name("link");
                let cb_ref = self.build_callback(&cb_name, hook);
                self.fb.system_call(
                    "Harlowe.Output",
                    "linkCb",
                    &[text, cb_ref],
                    Type::Dynamic,
                )
            } else {
                self.fb
                    .system_call("Harlowe.Output", "printVal", &[text], Type::Dynamic)
            }
        } else {
            self.fb.system_call(
                "Harlowe.Output",
                "content_array",
                &[],
                Type::Dynamic,
            )
        }
    }

    fn lower_link_goto(&mut self, mac: &MacroNode) -> ValueId {
        if mac.args.len() >= 2 {
            let text = self.lower_expr(&mac.args[0]);
            let passage = self.lower_expr(&mac.args[1]);
            self.fb.system_call(
                "Harlowe.Output",
                "link",
                &[text, passage],
                Type::Dynamic,
            )
        } else if let Some(arg) = mac.args.first() {
            let text = self.lower_expr(arg);
            self.fb.system_call(
                "Harlowe.Output",
                "link",
                &[text, text],
                Type::Dynamic,
            )
        } else {
            self.fb.system_call(
                "Harlowe.Output",
                "content_array",
                &[],
                Type::Dynamic,
            )
        }
    }

    // ── [[link]] ───────────────────────────────────────────────────

    fn lower_link(&mut self, link: &LinkNode) -> ValueId {
        let text = self.fb.const_string(&link.text);
        let passage = self.fb.const_string(&link.passage);
        self.fb
            .system_call("Harlowe.Output", "link", &[text, passage], Type::Dynamic)
    }

    // ── Changers ───────────────────────────────────────────────────

    fn lower_changer(&mut self, mac: &MacroNode) -> Option<ValueId> {
        // Map changer macro names to output builder function names
        let builder_name = match mac.name.as_str() {
            "color" | "colour" | "text-colour" | "text-color" => "color",
            "background" => "background",
            "text-style" => "textStyle",
            "font" => "font",
            "align" => "align",
            "opacity" => "opacity",
            "css" => "css",
            "transition" | "transition-arrive" | "transition-depart" => "transition",
            "transition-time" => "transitionTime",
            "hidden" => "hidden",
            "text-size" => "textSize",
            "text-rotate-z" => "textRotateZ",
            "collapse" => "collapse",
            "nobr" => "nobr",
            "hover-style" => "hoverStyle",
            _ => {
                // Unknown changer — fall back to generic styled
                if let Some(ref hook) = mac.hook {
                    let changer_name = self.fb.const_string(&mac.name);
                    let args: Vec<ValueId> =
                        mac.args.iter().map(|a| self.lower_expr(a)).collect();
                    let mut changer_args = vec![changer_name];
                    changer_args.extend(args);
                    let changer = self.fb.system_call(
                        "Harlowe.Engine",
                        "create_changer",
                        &changer_args,
                        Type::Dynamic,
                    );
                    let children = self.lower_content_array(hook);
                    return Some(self.fb.system_call(
                        "Harlowe.Output",
                        "styled",
                        &[changer, children],
                        Type::Dynamic,
                    ));
                }
                // Changer in expression context (no hook)
                let changer_name = self.fb.const_string(&mac.name);
                let args: Vec<ValueId> =
                    mac.args.iter().map(|a| self.lower_expr(a)).collect();
                let mut call_args = vec![changer_name];
                call_args.extend(args);
                return Some(self.fb.system_call(
                    "Harlowe.Engine",
                    "create_changer",
                    &call_args,
                    Type::Dynamic,
                ));
            }
        };

        if let Some(ref hook) = mac.hook {
            // Changer with hook → direct builder call: color("red", [children])
            let args: Vec<ValueId> = mac.args.iter().map(|a| self.lower_expr(a)).collect();
            let children = self.lower_content_array(hook);

            // Changers that take no value arg (hidden, collapse, nobr)
            let call_args = match builder_name {
                "hidden" | "collapse" | "nobr" => vec![children],
                _ => {
                    let mut a = args;
                    a.push(children);
                    a
                }
            };

            Some(self.fb.system_call(
                "Harlowe.Output",
                builder_name,
                &call_args,
                Type::Dynamic,
            ))
        } else {
            // Changer in expression context (no hook) → create_changer value
            let changer_name = self.fb.const_string(&mac.name);
            let args: Vec<ValueId> =
                mac.args.iter().map(|a| self.lower_expr(a)).collect();
            let mut call_args = vec![changer_name];
            call_args.extend(args);
            Some(self.fb.system_call(
                "Harlowe.Engine",
                "create_changer",
                &call_args,
                Type::Dynamic,
            ))
        }
    }

    // ── (live:) ────────────────────────────────────────────────────

    fn lower_live(&mut self, mac: &MacroNode) -> ValueId {
        let interval = if let Some(arg) = mac.args.first() {
            self.lower_expr(arg)
        } else {
            self.fb.const_float(1.0)
        };

        if let Some(ref hook) = mac.hook {
            let cb_name = self.make_callback_name("live");
            let cb_ref = self.build_callback(&cb_name, hook);
            self.fb.system_call(
                "Harlowe.Output",
                "live",
                &[interval, cb_ref],
                Type::Dynamic,
            )
        } else {
            self.fb.system_call(
                "Harlowe.Output",
                "content_array",
                &[],
                Type::Dynamic,
            )
        }
    }

    // ── Value macros (standalone) ──────────────────────────────────

    fn lower_value_macro_standalone(&mut self, mac: &MacroNode) -> ValueId {
        let name = self.fb.const_string(&mac.name);
        let args: Vec<ValueId> = mac.args.iter().map(|a| self.lower_expr(a)).collect();
        let mut call_args = vec![name];
        call_args.extend(args);
        let result = self.fb.system_call(
            "Harlowe.Engine",
            "value_macro",
            &call_args,
            Type::Dynamic,
        );
        if let Some(ref hook) = mac.hook {
            // Value macro with hook — use styled()
            let children = self.lower_content_array(hook);
            self.fb.system_call(
                "Harlowe.Output",
                "styled",
                &[result, children],
                Type::Dynamic,
            )
        } else {
            // Standalone value macro — return as printVal
            self.fb
                .system_call("Harlowe.Output", "printVal", &[result], Type::Dynamic)
        }
    }

    // ── Save/load ──────────────────────────────────────────────────

    fn lower_save_game(&mut self, mac: &MacroNode) {
        if let Some(arg) = mac.args.first() {
            let slot = self.lower_expr(arg);
            self.fb
                .system_call("Harlowe.Engine", "save_game", &[slot], Type::Dynamic);
        }
    }

    fn lower_load_game(&mut self, mac: &MacroNode) {
        if let Some(arg) = mac.args.first() {
            let slot = self.lower_expr(arg);
            self.fb
                .system_call("Harlowe.Engine", "load_game", &[slot], Type::Void);
        }
    }

    // ── Simple commands ────────────────────────────────────────────

    fn lower_simple_command(&mut self, mac: &MacroNode, system: &str, method: &str) {
        let args: Vec<ValueId> = mac.args.iter().map(|a| self.lower_expr(a)).collect();
        self.fb
            .system_call(system, method, &args, Type::Dynamic);
    }

    // ── DOM macros ─────────────────────────────────────────────────

    fn lower_dom_macro(&mut self, mac: &MacroNode) {
        let method = self.fb.const_string(&mac.name);
        let args: Vec<ValueId> = mac.args.iter().map(|a| self.lower_expr(a)).collect();
        let mut call_args = vec![method];
        call_args.extend(args);

        if let Some(ref hook) = mac.hook {
            let cb_name = self.make_callback_name("dom");
            let cb_ref = self.build_callback(&cb_name, hook);
            call_args.push(cb_ref);
        }

        self.fb
            .system_call("Harlowe.Engine", "dom_macro", &call_args, Type::Void);
    }

    // ── Click macros ───────────────────────────────────────────────

    fn lower_click_macro(&mut self, mac: &MacroNode) {
        let method = self.fb.const_string(&mac.name);
        let args: Vec<ValueId> = mac.args.iter().map(|a| self.lower_expr(a)).collect();
        let mut call_args = vec![method];
        call_args.extend(args);

        if let Some(ref hook) = mac.hook {
            let cb_name = self.make_callback_name("click");
            let cb_ref = self.build_callback(&cb_name, hook);
            call_args.push(cb_ref);
        }

        self.fb
            .system_call("Harlowe.Engine", "click_macro", &call_args, Type::Void);
    }

    // ── Unknown macros ─────────────────────────────────────────────

    fn lower_unknown_macro(&mut self, mac: &MacroNode) {
        let name = self.fb.const_string(&mac.name);
        let args: Vec<ValueId> = mac.args.iter().map(|a| self.lower_expr(a)).collect();
        let mut call_args = vec![name];
        call_args.extend(args);
        self.fb
            .system_call("Harlowe.Engine", "unknown_macro", &call_args, Type::Dynamic);

        if let Some(ref hook) = mac.hook {
            // Just lower the hook for side effects; unknown macros can't be content
            for node in hook {
                self.lower_node(node);
            }
        }
    }

    // ── Callback building ──────────────────────────────────────────

    fn make_callback_name(&mut self, kind: &str) -> String {
        let name = format!("{}_{kind}_{}", self.func_name, self.callback_count);
        self.callback_count += 1;
        name
    }

    fn build_callback(&mut self, name: &str, body: &[Node]) -> ValueId {
        let sig = FunctionSig {
            params: vec![],
            return_ty: Type::Dynamic,
            defaults: vec![],
            has_rest_param: false,
        };
        let mut cb_fb = FunctionBuilder::new(name, sig, Visibility::Public);

        let saved_fb = std::mem::replace(&mut self.fb, cb_fb);
        let saved_temps = std::mem::take(&mut self.temp_vars);

        let content = self.lower_content_array(body);
        self.fb.ret(Some(content));

        cb_fb = std::mem::replace(&mut self.fb, saved_fb);
        self.temp_vars = saved_temps;

        self.callbacks.push(cb_fb.build());

        self.fb.global_ref(name, Type::Dynamic)
    }

    // ── Temp variable helpers ──────────────────────────────────────

    fn get_or_create_temp(&mut self, name: &str) -> ValueId {
        if let Some(&alloc) = self.temp_vars.get(name) {
            return alloc;
        }
        let alloc = self.fb.alloc(Type::Dynamic);
        self.fb.name_value(alloc, format!("_{name}"));
        self.temp_vars.insert(name.to_string(), alloc);
        alloc
    }

    fn get_or_load_temp(&mut self, name: &str) -> ValueId {
        let alloc = self.get_or_create_temp(name);
        self.fb.load(alloc, Type::Dynamic)
    }

    // ── Expression lowering ────────────────────────────────────────

    fn lower_expr(&mut self, expr: &Expr) -> ValueId {
        match &expr.kind {
            ExprKind::Number(n) => self.fb.const_float(*n),
            ExprKind::Str(s) => self.fb.const_string(s.as_str()),
            ExprKind::Bool(b) => self.fb.const_bool(*b),
            ExprKind::It => {
                if let Some(ref target) = self.set_target.clone() {
                    self.lower_expr(target)
                } else {
                    self.fb
                        .system_call("Harlowe.State", "get_it", &[], Type::Dynamic)
                }
            }
            ExprKind::StoryVar(name) => {
                let n = self.fb.const_string(name.as_str());
                self.fb
                    .system_call("Harlowe.State", "get", &[n], Type::Dynamic)
            }
            ExprKind::TempVar(name) => self.get_or_load_temp(name),
            ExprKind::Ident(name) => {
                self.fb.const_string(name.as_str())
            }
            ExprKind::ColorLiteral(color) => self.fb.const_string(color.as_str()),
            ExprKind::TimeLiteral(secs) => self.fb.const_float(*secs),
            ExprKind::Ordinal(ord) => {
                match ord {
                    Ordinal::Nth(n) => self.fb.const_int(*n as i64),
                    Ordinal::Last => {
                        self.fb.const_string("last")
                    }
                    Ordinal::Length => {
                        self.fb.const_string("length")
                    }
                }
            }
            ExprKind::Binary { op, left, right } => self.lower_binary(op, left, right),
            ExprKind::Unary { op, operand } => self.lower_unary(op, operand),
            ExprKind::Assign { target, value } => {
                let val = self.lower_expr(value);
                self.store_to_target(target, val);
                val
            }
            ExprKind::Call { name, args } => self.lower_call(name, args),
            ExprKind::Possessive { object, property } => {
                let obj = self.lower_expr(object);
                let prop = self.lower_expr(property);
                self.fb.system_call(
                    "Harlowe.Engine",
                    "get_property",
                    &[obj, prop],
                    Type::Dynamic,
                )
            }
            ExprKind::Of { property, object } => {
                let obj = self.lower_expr(object);
                let prop = self.lower_expr(property);
                self.fb.system_call(
                    "Harlowe.Engine",
                    "get_property",
                    &[obj, prop],
                    Type::Dynamic,
                )
            }
            ExprKind::Paren(inner) => self.lower_expr(inner),
            ExprKind::Error(_) => self.fb.const_bool(false),
        }
    }

    fn lower_binary(&mut self, op: &BinaryOp, left: &Expr, right: &Expr) -> ValueId {
        match op {
            BinaryOp::And => return self.lower_logical_and(left, right),
            BinaryOp::Or => return self.lower_logical_or(left, right),
            _ => {}
        }

        let lhs = self.lower_expr(left);
        let rhs = self.lower_expr(right);

        match op {
            BinaryOp::Add | BinaryOp::Plus => {
                self.fb
                    .system_call("Harlowe.Engine", "plus", &[lhs, rhs], Type::Dynamic)
            }
            BinaryOp::Sub => self.fb.sub(lhs, rhs),
            BinaryOp::Mul => self.fb.mul(lhs, rhs),
            BinaryOp::Div => self.fb.div(lhs, rhs),
            BinaryOp::Mod => self.fb.rem(lhs, rhs),
            BinaryOp::Is => self.fb.cmp(CmpKind::Eq, lhs, rhs),
            BinaryOp::IsNot => self.fb.cmp(CmpKind::Ne, lhs, rhs),
            BinaryOp::Lt => self.fb.cmp(CmpKind::Lt, lhs, rhs),
            BinaryOp::Lte => self.fb.cmp(CmpKind::Le, lhs, rhs),
            BinaryOp::Gt => self.fb.cmp(CmpKind::Gt, lhs, rhs),
            BinaryOp::Gte => self.fb.cmp(CmpKind::Ge, lhs, rhs),
            BinaryOp::Contains => {
                self.fb
                    .system_call("Harlowe.Engine", "contains", &[lhs, rhs], Type::Bool)
            }
            BinaryOp::IsIn => {
                self.fb
                    .system_call("Harlowe.Engine", "is_in", &[lhs, rhs], Type::Bool)
            }
            BinaryOp::And | BinaryOp::Or => unreachable!(),
        }
    }

    fn lower_unary(&mut self, op: &UnaryOp, operand: &Expr) -> ValueId {
        let val = self.lower_expr(operand);
        match op {
            UnaryOp::Not => {
                self.fb
                    .system_call("Harlowe.Engine", "not", &[val], Type::Bool)
            }
            UnaryOp::Neg => {
                let zero = self.fb.const_float(0.0);
                self.fb.sub(zero, val)
            }
        }
    }

    fn lower_logical_and(&mut self, left: &Expr, right: &Expr) -> ValueId {
        let lhs = self.lower_expr(left);

        let rhs_block = self.fb.create_block();
        let merge_block = self.fb.create_block();
        let merge_params = self.fb.add_block_params(merge_block, &[Type::Dynamic]);
        let merge_param = merge_params[0];

        self.fb
            .br_if(lhs, rhs_block, &[], merge_block, &[lhs]);

        self.fb.switch_to_block(rhs_block);
        let rhs = self.lower_expr(right);
        self.fb.br(merge_block, &[rhs]);

        self.fb.switch_to_block(merge_block);
        merge_param
    }

    fn lower_logical_or(&mut self, left: &Expr, right: &Expr) -> ValueId {
        let lhs = self.lower_expr(left);

        let rhs_block = self.fb.create_block();
        let merge_block = self.fb.create_block();
        let merge_params = self.fb.add_block_params(merge_block, &[Type::Dynamic]);
        let merge_param = merge_params[0];

        self.fb
            .br_if(lhs, merge_block, &[lhs], rhs_block, &[]);

        self.fb.switch_to_block(rhs_block);
        let rhs = self.lower_expr(right);
        self.fb.br(merge_block, &[rhs]);

        self.fb.switch_to_block(merge_block);
        merge_param
    }

    // ── Inline macro calls in expressions ──────────────────────────

    fn lower_call(&mut self, name: &str, args: &[Expr]) -> ValueId {
        let lowered_args: Vec<ValueId> = args.iter().map(|a| self.lower_expr(a)).collect();

        match name {
            "random" => {
                self.fb
                    .system_call("Harlowe.Engine", "random", &lowered_args, Type::Dynamic)
            }
            "either" => {
                self.fb
                    .system_call("Harlowe.Engine", "either", &lowered_args, Type::Dynamic)
            }
            "str" | "string" => {
                self.fb
                    .system_call("Harlowe.Engine", "str", &lowered_args, Type::Dynamic)
            }
            "num" | "number" => {
                self.fb
                    .system_call("Harlowe.Engine", "num", &lowered_args, Type::Dynamic)
            }
            "a" | "array" => {
                self.fb
                    .system_call("Harlowe.Engine", "array", &lowered_args, Type::Dynamic)
            }
            "dm" | "datamap" => {
                self.fb
                    .system_call("Harlowe.Engine", "datamap", &lowered_args, Type::Dynamic)
            }
            "ds" | "dataset" => {
                self.fb
                    .system_call("Harlowe.Engine", "dataset", &lowered_args, Type::Dynamic)
            }
            "round" | "floor" | "ceil" | "abs" | "min" | "max" | "sqrt" | "sin" | "cos"
            | "tan" | "log" | "pow" | "sign" | "clamp" | "lerp" => {
                let n = self.fb.const_string(name);
                let mut call_args = vec![n];
                call_args.extend(lowered_args);
                self.fb.system_call(
                    "Harlowe.Engine",
                    "math",
                    &call_args,
                    Type::Dynamic,
                )
            }
            "sorted" | "reversed" | "rotated" | "shuffled" | "count" | "range" | "find"
            | "altered" | "folded" | "interlaced" | "repeated" | "joined" | "some-pass"
            | "all-pass" | "none-pass" | "subarray" | "substring" | "lowercase"
            | "uppercase" | "datanames" | "datavalues" | "dataentries" => {
                let n = self.fb.const_string(name);
                let mut call_args = vec![n];
                call_args.extend(lowered_args);
                self.fb.system_call(
                    "Harlowe.Engine",
                    "collection_op",
                    &call_args,
                    Type::Dynamic,
                )
            }
            "saved-games" => {
                self.fb.system_call(
                    "Harlowe.Engine",
                    "saved_games",
                    &lowered_args,
                    Type::Dynamic,
                )
            }
            "passage" => {
                self.fb.system_call(
                    "Harlowe.Engine",
                    "current_passage",
                    &lowered_args,
                    Type::Dynamic,
                )
            }
            "visits" | "turns" | "time" | "history" => {
                let n = self.fb.const_string(name);
                self.fb
                    .system_call("Harlowe.Engine", "meta", &[n], Type::Dynamic)
            }
            "rgb" | "rgba" | "hsl" | "hsla" | "lch" | "lcha" | "complement" | "mix" => {
                let n = self.fb.const_string(name);
                let mut call_args = vec![n];
                call_args.extend(lowered_args);
                self.fb.system_call(
                    "Harlowe.Engine",
                    "color_op",
                    &call_args,
                    Type::Dynamic,
                )
            }
            name if macros::macro_kind(name) == MacroKind::Changer => {
                let n = self.fb.const_string(name);
                let mut call_args = vec![n];
                call_args.extend(lowered_args);
                self.fb.system_call(
                    "Harlowe.Engine",
                    "create_changer",
                    &call_args,
                    Type::Dynamic,
                )
            }
            _ => {
                let n = self.fb.const_string(name);
                let mut call_args = vec![n];
                call_args.extend(lowered_args);
                self.fb.system_call(
                    "Harlowe.Engine",
                    "call",
                    &call_args,
                    Type::Dynamic,
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::harlowe::parser;

    #[test]
    fn test_plain_text_emits_output() {
        let ast = parser::parse("Hello world");
        let result = translate_passage("test", &ast);
        assert_eq!(result.func.name, "passage_test");
        // Return type should be Dynamic (returns content array)
        assert_eq!(result.func.sig.return_ty, Type::Dynamic);
    }

    #[test]
    fn test_set_produces_state_set() {
        let ast = parser::parse("(set: $x to 1)");
        let result = translate_passage("test_set", &ast);
        assert_eq!(result.func.name, "passage_test_set");
    }

    #[test]
    fn test_if_creates_blocks() {
        let ast = parser::parse("(if: $x is 1)[yes](else:)[no]");
        let result = translate_passage("test_if", &ast);
        // Should have multiple blocks for the if/else
        assert!(result.func.blocks.len() >= 3);
    }

    #[test]
    fn test_link_produces_syscall() {
        let ast = parser::parse("[[Start->Begin]]");
        let result = translate_passage("test_link", &ast);
        assert_eq!(result.func.name, "passage_test_link");
    }

    #[test]
    fn test_link_macro_creates_callback() {
        let ast = parser::parse("(link: \"Continue\")[(goto: \"Next\")]");
        let result = translate_passage("test_link_macro", &ast);
        assert_eq!(result.callbacks.len(), 1);
        // Callback should return Dynamic
        assert_eq!(result.callbacks[0].sig.return_ty, Type::Dynamic);
    }

    #[test]
    fn test_goto_produces_navigation() {
        let ast = parser::parse("(goto: \"Event 3-check\")");
        let result = translate_passage("test_goto", &ast);
        assert_eq!(result.func.name, "passage_test_goto");
    }

    #[test]
    fn test_color_changer() {
        let ast = parser::parse("(color: green)[Hello]");
        let result = translate_passage("test_color", &ast);
        assert_eq!(result.func.name, "passage_test_color");
    }

    #[test]
    fn test_passage_func_name_sanitization() {
        assert_eq!(passage_func_name("Floor 1 entryway"), "passage_Floor_1_entryway");
        assert_eq!(passage_func_name("Event 3-check"), "passage_Event_3_check");
    }

    #[test]
    fn test_complex_passage() {
        let src = r#"(set: $recovery to "Floor 1 entryway")
You're at the **entryway**

(if: $hypnoStat < 70)[Normal text.](else:)[(color: magenta+white)[Hypno text.]]"#;
        let ast = parser::parse(src);
        let result = translate_passage("complex", &ast);
        assert!(!result.func.blocks.is_empty());
    }

    #[test]
    fn test_live_creates_callback() {
        let ast = parser::parse("(live: 2s)[(stop:)]");
        let result = translate_passage("test_live", &ast);
        assert_eq!(result.callbacks.len(), 1);
    }

    #[test]
    fn test_it_in_set_reads_target_variable() {
        use reincarnate_core::ir::inst::Op;
        let ast = parser::parse("(set: $x to it + 1)");
        let result = translate_passage("test_it_set", &ast);
        let func = &result.func;
        let has_get_it = func.blocks.values().any(|block| {
            block.insts.iter().any(|&inst_id| {
                matches!(&func.insts[inst_id].op, Op::SystemCall { system, method, .. }
                    if system == "Harlowe.State" && method == "get_it")
            })
        });
        assert!(!has_get_it, "should not use get_it() inside (set:)");
        let has_get_x = func.blocks.values().any(|block| {
            block.insts.iter().any(|&inst_id| {
                matches!(&func.insts[inst_id].op, Op::SystemCall { system, method, .. }
                    if system == "Harlowe.State" && method == "get")
            })
        });
        assert!(has_get_x, "should read target variable with get()");
    }

    #[test]
    fn test_content_array_in_ir() {
        use reincarnate_core::ir::inst::Op;
        let ast = parser::parse("Hello");
        let result = translate_passage("test_content", &ast);
        let func = &result.func;
        // Should have content_array and text_node system calls
        let has_content_array = func.blocks.values().any(|block| {
            block.insts.iter().any(|&inst_id| {
                matches!(&func.insts[inst_id].op, Op::SystemCall { system, method, .. }
                    if system == "Harlowe.Output" && method == "content_array")
            })
        });
        assert!(has_content_array, "should have content_array call");
        let has_text_node = func.blocks.values().any(|block| {
            block.insts.iter().any(|&inst_id| {
                matches!(&func.insts[inst_id].op, Op::SystemCall { system, method, .. }
                    if system == "Harlowe.Output" && method == "text_node")
            })
        });
        assert!(has_text_node, "should have text_node call");
    }
}
