use std::collections::{BTreeSet, HashMap, HashSet};
use std::fmt::Write;
use std::fs;
use std::path::Path;

use reincarnate_core::entity::EntityRef;
use reincarnate_core::error::CoreError;
use reincarnate_core::ir::{
    Block, CmpKind, Constant, Function, InstId, Module, Op, StructDef, Type, ValueId, Visibility,
};

use crate::runtime::SYSTEM_NAMES;
use crate::types::ts_type;

/// Emit a single module as a `.ts` file into `output_dir`.
pub fn emit_module(module: &Module, output_dir: &Path) -> Result<(), CoreError> {
    let out = emit_module_to_string(module)?;
    let path = output_dir.join(format!("{}.ts", module.name));
    fs::write(&path, &out).map_err(CoreError::Io)?;
    Ok(())
}

/// Emit a module to a string (for testing).
pub fn emit_module_to_string(module: &Module) -> Result<String, CoreError> {
    let mut out = String::new();

    emit_runtime_imports(module, &mut out);
    emit_imports(module, &mut out);
    emit_structs(module, &mut out);
    emit_enums(module, &mut out);
    emit_globals(module, &mut out);
    emit_functions(module, &mut out)?;

    Ok(out)
}

// ---------------------------------------------------------------------------
// Emit context — alias resolution and inline-let tracking
// ---------------------------------------------------------------------------

/// Per-function context for copy propagation and inline `let` declarations.
struct EmitCtx {
    /// Value alias map: value → canonical source value (from copy propagation).
    aliases: HashMap<ValueId, ValueId>,
    /// Instructions to skip entirely (their effects are folded into aliases).
    skip_insts: HashSet<InstId>,
    /// Values that need a `let` keyword when first emitted.
    needs_let: HashSet<ValueId>,
}

impl EmitCtx {
    fn for_function(func: &Function) -> Self {
        let (aliases, skip_insts) = build_aliases(func);
        let needs_let = compute_needs_let(func, &skip_insts);
        Self {
            aliases,
            skip_insts,
            needs_let,
        }
    }

    /// Resolve a value through the alias chain.
    fn val(&self, v: ValueId) -> String {
        let mut resolved = v;
        for _ in 0..100 {
            match self.aliases.get(&resolved) {
                Some(&alias) if alias != resolved => resolved = alias,
                _ => break,
            }
        }
        format!("v{}", resolved.index())
    }

    /// Returns `"let "` the first time a value is emitted, `""` thereafter.
    fn let_prefix(&self, v: ValueId) -> &'static str {
        if self.needs_let.contains(&v) {
            "let "
        } else {
            ""
        }
    }

    fn should_skip(&self, inst_id: InstId) -> bool {
        self.skip_insts.contains(&inst_id)
    }
}

/// Build an alias map via copy propagation on Alloc/Store/Load/Copy patterns.
///
/// For alloc'd pointers with exactly one Store, all Loads are aliased to the
/// stored value and the Alloc/Store/Load instructions are marked for skipping.
/// Copy instructions are always aliased.
fn build_aliases(func: &Function) -> (HashMap<ValueId, ValueId>, HashSet<InstId>) {
    let mut aliases = HashMap::new();
    let mut skip_insts = HashSet::new();

    // Track alloc'd pointers and their stores.
    let mut alloc_results: HashSet<ValueId> = HashSet::new();
    let mut store_count: HashMap<ValueId, usize> = HashMap::new();
    let mut store_value: HashMap<ValueId, ValueId> = HashMap::new();

    for (_inst_id, inst) in func.insts.iter() {
        match &inst.op {
            Op::Alloc(_) => {
                if let Some(r) = inst.result {
                    alloc_results.insert(r);
                }
            }
            Op::Store { ptr, value } => {
                if alloc_results.contains(ptr) {
                    *store_count.entry(*ptr).or_default() += 1;
                    store_value.insert(*ptr, *value);
                }
            }
            _ => {}
        }
    }

    // Single-store alloc pointers can be entirely eliminated.
    let single_store: HashSet<ValueId> = store_count
        .iter()
        .filter(|(_, &c)| c == 1)
        .map(|(&ptr, _)| ptr)
        .collect();

    for (inst_id, inst) in func.insts.iter() {
        match &inst.op {
            Op::Alloc(_) if inst.result.is_some_and(|r| single_store.contains(&r)) => {
                skip_insts.insert(inst_id);
            }
            Op::Store { ptr, .. } if single_store.contains(ptr) => {
                skip_insts.insert(inst_id);
            }
            Op::Load(ptr) if single_store.contains(ptr) => {
                if let Some(r) = inst.result {
                    aliases.insert(r, store_value[ptr]);
                    skip_insts.insert(inst_id);
                }
            }
            Op::Copy(src) => {
                if let Some(r) = inst.result {
                    aliases.insert(r, *src);
                    skip_insts.insert(inst_id);
                }
            }
            _ => {}
        }
    }

    // Resolve transitive aliases: v3 → v2 → v1 becomes v3 → v1.
    loop {
        let mut changed = false;
        let snapshot: Vec<_> = aliases.iter().map(|(k, v)| (*k, *v)).collect();
        for (key, target) in snapshot {
            if let Some(&next) = aliases.get(&target) {
                if next != aliases[&key] {
                    aliases.insert(key, next);
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }

    (aliases, skip_insts)
}

/// Determine which values should get an inline `let` declaration.
///
/// Entry-block parameters are declared in the function signature. Non-entry
/// block parameters are pre-declared before the dispatch loop. Everything else
/// (instruction results) gets `let` at its definition site.
fn compute_needs_let(func: &Function, skip_insts: &HashSet<InstId>) -> HashSet<ValueId> {
    let entry_params: HashSet<ValueId> =
        func.blocks[func.entry].params.iter().map(|p| p.value).collect();

    let mut needs_let = HashSet::new();
    for (inst_id, inst) in func.insts.iter() {
        if skip_insts.contains(&inst_id) {
            continue;
        }
        if let Some(r) = inst.result {
            if !entry_params.contains(&r) {
                needs_let.insert(r);
            }
        }
    }
    needs_let
}

// ---------------------------------------------------------------------------
// Identifier sanitization
// ---------------------------------------------------------------------------

/// Sanitize a name into a valid JavaScript identifier.
///
/// Replaces non-alphanumeric characters with `_` and prefixes with `_` if the
/// name starts with a digit.
pub(crate) fn sanitize_ident(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' || ch == '$' {
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
    out
}

/// Check whether a string is a valid JS identifier (safe for dot-access).
fn is_valid_js_ident(name: &str) -> bool {
    !name.is_empty()
        && !name.starts_with(|c: char| c.is_ascii_digit())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '$')
}

// ---------------------------------------------------------------------------
// Runtime imports (auto-detected from SystemCall ops)
// ---------------------------------------------------------------------------

fn collect_system_names(module: &Module) -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    for (_id, func) in module.functions.iter() {
        for (_inst_id, inst) in func.insts.iter() {
            if let Op::SystemCall { system, .. } = &inst.op {
                used.insert(system.clone());
            }
        }
    }
    used
}

fn emit_runtime_imports(module: &Module, out: &mut String) {
    let systems = collect_system_names(module);
    if systems.is_empty() {
        return;
    }
    let known: BTreeSet<&str> = SYSTEM_NAMES.iter().copied().collect();
    let mut generic: Vec<&str> = Vec::new();
    let mut flash: Vec<String> = Vec::new();
    for sys in &systems {
        if known.contains(sys.as_str()) {
            generic.push(sys.as_str());
        } else {
            flash.push(sanitize_ident(sys));
        }
    }
    if !generic.is_empty() {
        let _ = writeln!(
            out,
            "import {{ {} }} from \"./runtime\";",
            generic.join(", ")
        );
    }
    if !flash.is_empty() {
        let _ = writeln!(
            out,
            "import {{ {} }} from \"./runtime/flash\";",
            flash.join(", ")
        );
    }
    if !generic.is_empty() || !flash.is_empty() {
        out.push('\n');
    }
}

// ---------------------------------------------------------------------------
// Imports
// ---------------------------------------------------------------------------

fn emit_imports(module: &Module, out: &mut String) {
    for import in &module.imports {
        let name = match &import.alias {
            Some(alias) => {
                format!("{} as {}", sanitize_ident(&import.name), sanitize_ident(alias))
            }
            None => sanitize_ident(&import.name),
        };
        let _ = writeln!(out, "import {{ {name} }} from \"./{}\";", import.module);
    }
    if !module.imports.is_empty() {
        out.push('\n');
    }
}

// ---------------------------------------------------------------------------
// Structs → interfaces
// ---------------------------------------------------------------------------

fn emit_structs(module: &Module, out: &mut String) {
    for def in &module.structs {
        emit_struct(def, out);
    }
}

fn emit_struct(def: &StructDef, out: &mut String) {
    let vis = visibility_prefix(def.visibility);
    let _ = writeln!(out, "{vis}interface {} {{", sanitize_ident(&def.name));
    for (name, ty) in &def.fields {
        let _ = writeln!(out, "  {}: {};", sanitize_ident(name), ts_type(ty));
    }
    let _ = writeln!(out, "}}\n");
}

// ---------------------------------------------------------------------------
// Enums → discriminated union types
// ---------------------------------------------------------------------------

fn emit_enums(module: &Module, out: &mut String) {
    for def in &module.enums {
        let vis = visibility_prefix(def.visibility);
        let variants: Vec<String> = def
            .variants
            .iter()
            .map(|v| {
                if v.fields.is_empty() {
                    format!("{{ tag: \"{}\" }}", v.name)
                } else {
                    let fields: Vec<String> = v
                        .fields
                        .iter()
                        .enumerate()
                        .map(|(i, t)| format!("field{i}: {}", ts_type(t)))
                        .collect();
                    format!("{{ tag: \"{}\", {} }}", v.name, fields.join(", "))
                }
            })
            .collect();
        let _ = writeln!(
            out,
            "{vis}type {} = {};",
            sanitize_ident(&def.name),
            variants.join(" | ")
        );
        out.push('\n');
    }
}

// ---------------------------------------------------------------------------
// Globals
// ---------------------------------------------------------------------------

fn emit_globals(module: &Module, out: &mut String) {
    for global in &module.globals {
        let vis = visibility_prefix(global.visibility);
        let kw = if global.mutable { "let" } else { "const" };
        let _ = writeln!(
            out,
            "{vis}{kw} {}: {};",
            sanitize_ident(&global.name),
            ts_type(&global.ty)
        );
    }
    if !module.globals.is_empty() {
        out.push('\n');
    }
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

fn emit_functions(module: &Module, out: &mut String) -> Result<(), CoreError> {
    for (_id, func) in module.functions.iter() {
        emit_function(func, out)?;
    }
    Ok(())
}

fn emit_function(func: &Function, out: &mut String) -> Result<(), CoreError> {
    let ctx = EmitCtx::for_function(func);
    let vis = visibility_prefix(func.visibility);
    let star = if func.coroutine.is_some() { "*" } else { "" };

    // Parameters from entry block.
    let entry = &func.blocks[func.entry];
    let params: Vec<String> = entry
        .params
        .iter()
        .map(|p| format!("{}: {}", ctx.val(p.value), ts_type(&p.ty)))
        .collect();
    let ret_ty = ts_type(&func.sig.return_ty);

    let _ = write!(
        out,
        "{vis}function{star} {}({params}): {ret_ty}",
        sanitize_ident(&func.name),
        params = params.join(", "),
    );

    let is_simple = func.blocks.len() == 1;

    let _ = writeln!(out, " {{");

    if is_simple {
        emit_block_body(&ctx, func, func.entry, &func.blocks[func.entry], out, "  ")?;
    } else {
        // Pre-declare only non-entry block parameters (they're assigned from
        // other blocks via branch args).
        emit_block_param_declarations(&ctx, func, out);

        let _ = writeln!(out, "  let $block = {};", func.entry.index());
        let _ = writeln!(out, "  while (true) {{");
        let _ = writeln!(out, "    switch ($block) {{");

        for (block_id, block) in func.blocks.iter() {
            let _ = writeln!(out, "      case {}: {{", block_id.index());
            emit_block_body(&ctx, func, block_id, block, out, "        ")?;
            let _ = writeln!(out, "      }}");
        }

        let _ = writeln!(out, "    }}");
        let _ = writeln!(out, "  }}");
    }

    let _ = writeln!(out, "}}\n");
    Ok(())
}

/// Pre-declare `let` bindings for non-entry block parameters only.
fn emit_block_param_declarations(ctx: &EmitCtx, func: &Function, out: &mut String) {
    for (block_id, block) in func.blocks.iter() {
        if block_id == func.entry {
            continue;
        }
        for param in &block.params {
            let _ = writeln!(
                out,
                "  let {}: {};",
                ctx.val(param.value),
                ts_type(&param.ty)
            );
        }
    }
}

fn emit_block_body(
    ctx: &EmitCtx,
    func: &Function,
    _block_id: reincarnate_core::ir::BlockId,
    block: &Block,
    out: &mut String,
    indent: &str,
) -> Result<(), CoreError> {
    for &inst_id in &block.insts {
        if ctx.should_skip(inst_id) {
            continue;
        }
        emit_inst(ctx, func, inst_id, out, indent)?;
    }
    Ok(())
}

fn emit_inst(
    ctx: &EmitCtx,
    func: &Function,
    inst_id: InstId,
    out: &mut String,
    indent: &str,
) -> Result<(), CoreError> {
    let inst = &func.insts[inst_id];
    let result = inst.result;

    match &inst.op {
        // -- Constants --
        Op::Const(c) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(out, "{indent}{pfx}{} = {};", ctx.val(r), emit_constant(c));
        }

        // -- Arithmetic --
        Op::Add(a, b) => emit_binop(ctx, out, indent, result, a, b, "+"),
        Op::Sub(a, b) => emit_binop(ctx, out, indent, result, a, b, "-"),
        Op::Mul(a, b) => emit_binop(ctx, out, indent, result, a, b, "*"),
        Op::Div(a, b) => emit_binop(ctx, out, indent, result, a, b, "/"),
        Op::Rem(a, b) => emit_binop(ctx, out, indent, result, a, b, "%"),
        Op::Neg(a) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(out, "{indent}{pfx}{} = -{};", ctx.val(r), ctx.val(*a));
        }

        // -- Bitwise --
        Op::BitAnd(a, b) => emit_binop(ctx, out, indent, result, a, b, "&"),
        Op::BitOr(a, b) => emit_binop(ctx, out, indent, result, a, b, "|"),
        Op::BitXor(a, b) => emit_binop(ctx, out, indent, result, a, b, "^"),
        Op::BitNot(a) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(out, "{indent}{pfx}{} = ~{};", ctx.val(r), ctx.val(*a));
        }
        Op::Shl(a, b) => emit_binop(ctx, out, indent, result, a, b, "<<"),
        Op::Shr(a, b) => emit_binop(ctx, out, indent, result, a, b, ">>"),

        // -- Comparison --
        Op::Cmp(kind, a, b) => {
            let op = match kind {
                CmpKind::Eq => "===",
                CmpKind::Ne => "!==",
                CmpKind::Lt => "<",
                CmpKind::Le => "<=",
                CmpKind::Gt => ">",
                CmpKind::Ge => ">=",
            };
            emit_binop(ctx, out, indent, result, a, b, op);
        }

        // -- Logic --
        Op::Not(a) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(out, "{indent}{pfx}{} = !{};", ctx.val(r), ctx.val(*a));
        }

        // -- Control flow --
        Op::Br { target, args } => {
            emit_branch_args(ctx, func, *target, args, out, indent);
            let _ = writeln!(out, "{indent}$block = {}; continue;", target.index());
        }
        Op::BrIf {
            cond,
            then_target,
            then_args,
            else_target,
            else_args,
        } => {
            let _ = writeln!(out, "{indent}if ({}) {{", ctx.val(*cond));
            let inner = format!("{indent}  ");
            emit_branch_args(ctx, func, *then_target, then_args, out, &inner);
            let _ = writeln!(
                out,
                "{inner}$block = {}; continue;",
                then_target.index()
            );
            let _ = writeln!(out, "{indent}}} else {{");
            emit_branch_args(ctx, func, *else_target, else_args, out, &inner);
            let _ = writeln!(
                out,
                "{inner}$block = {}; continue;",
                else_target.index()
            );
            let _ = writeln!(out, "{indent}}}");
        }
        Op::Switch {
            value,
            cases,
            default,
        } => {
            let _ = writeln!(out, "{indent}switch ({}) {{", ctx.val(*value));
            for (constant, target, args) in cases {
                let _ = writeln!(out, "{indent}  case {}:", emit_constant(constant));
                let inner = format!("{indent}    ");
                emit_branch_args(ctx, func, *target, args, out, &inner);
                let _ = writeln!(
                    out,
                    "{inner}$block = {}; continue;",
                    target.index()
                );
            }
            let _ = writeln!(out, "{indent}  default:");
            let inner = format!("{indent}    ");
            emit_branch_args(ctx, func, default.0, &default.1, out, &inner);
            let _ = writeln!(
                out,
                "{inner}$block = {}; continue;",
                default.0.index()
            );
            let _ = writeln!(out, "{indent}}}");
        }
        Op::Return(v) => match v {
            Some(v) => {
                let _ = writeln!(out, "{indent}return {};", ctx.val(*v));
            }
            None => {
                let _ = writeln!(out, "{indent}return;");
            }
        },

        // -- Memory / fields --
        Op::Alloc(ty) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(
                out,
                "{indent}{pfx}{} = undefined as unknown as {};",
                ctx.val(r),
                ts_type(ty)
            );
        }
        Op::Load(ptr) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(out, "{indent}{pfx}{} = {};", ctx.val(r), ctx.val(*ptr));
        }
        Op::Store { ptr, value } => {
            let _ = writeln!(out, "{indent}{} = {};", ctx.val(*ptr), ctx.val(*value));
        }
        Op::GetField { object, field } => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            if is_valid_js_ident(field) {
                let _ = writeln!(
                    out,
                    "{indent}{pfx}{} = {}.{field};",
                    ctx.val(r),
                    ctx.val(*object)
                );
            } else {
                let _ = writeln!(
                    out,
                    "{indent}{pfx}{} = {}[\"{}\"];",
                    ctx.val(r),
                    ctx.val(*object),
                    escape_js_string(field)
                );
            }
        }
        Op::SetField {
            object,
            field,
            value,
        } => {
            if is_valid_js_ident(field) {
                let _ = writeln!(
                    out,
                    "{indent}{}.{field} = {};",
                    ctx.val(*object),
                    ctx.val(*value)
                );
            } else {
                let _ = writeln!(
                    out,
                    "{indent}{}[\"{}\"] = {};",
                    ctx.val(*object),
                    escape_js_string(field),
                    ctx.val(*value)
                );
            }
        }
        Op::GetIndex { collection, index } => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(
                out,
                "{indent}{pfx}{} = {}[{}];",
                ctx.val(r),
                ctx.val(*collection),
                ctx.val(*index)
            );
        }
        Op::SetIndex {
            collection,
            index,
            value,
        } => {
            let _ = writeln!(
                out,
                "{indent}{}[{}] = {};",
                ctx.val(*collection),
                ctx.val(*index),
                ctx.val(*value)
            );
        }

        // -- Calls --
        Op::Call { func: fname, args } => {
            let args_str = args
                .iter()
                .map(|a| ctx.val(*a))
                .collect::<Vec<_>>()
                .join(", ");
            let safe_name = sanitize_ident(fname);
            if let Some(r) = result {
                let pfx = ctx.let_prefix(r);
                let _ = writeln!(
                    out,
                    "{indent}{pfx}{} = {safe_name}({args_str});",
                    ctx.val(r)
                );
            } else {
                let _ = writeln!(out, "{indent}{safe_name}({args_str});");
            }
        }
        Op::CallIndirect { callee, args } => {
            let args_str = args
                .iter()
                .map(|a| ctx.val(*a))
                .collect::<Vec<_>>()
                .join(", ");
            if let Some(r) = result {
                let pfx = ctx.let_prefix(r);
                let _ = writeln!(
                    out,
                    "{indent}{pfx}{} = {}({args_str});",
                    ctx.val(r),
                    ctx.val(*callee)
                );
            } else {
                let _ = writeln!(out, "{indent}{}({args_str});", ctx.val(*callee));
            }
        }
        Op::SystemCall {
            system,
            method,
            args,
        } => {
            let args_str = args
                .iter()
                .map(|a| ctx.val(*a))
                .collect::<Vec<_>>()
                .join(", ");
            let sys_ident = sanitize_ident(system);
            let safe_method = if is_valid_js_ident(method) {
                format!(".{method}")
            } else {
                format!("[\"{}\"]", escape_js_string(method))
            };
            if let Some(r) = result {
                let pfx = ctx.let_prefix(r);
                let _ = writeln!(
                    out,
                    "{indent}{pfx}{} = {sys_ident}{safe_method}({args_str});",
                    ctx.val(r)
                );
            } else {
                let _ = writeln!(
                    out,
                    "{indent}{sys_ident}{safe_method}({args_str});",
                );
            }
        }

        // -- Type operations --
        Op::Cast(v, ty) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(
                out,
                "{indent}{pfx}{} = {} as {};",
                ctx.val(r),
                ctx.val(*v),
                ts_type(ty)
            );
        }
        Op::TypeCheck(v, ty) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let check = type_check_expr(ctx, *v, ty);
            let _ = writeln!(out, "{indent}{pfx}{} = {check};", ctx.val(r));
        }

        // -- Aggregate construction --
        Op::StructInit { name: _, fields } => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let field_strs: Vec<String> = fields
                .iter()
                .map(|(name, v)| {
                    if is_valid_js_ident(name) {
                        format!("{name}: {}", ctx.val(*v))
                    } else {
                        format!("\"{}\": {}", escape_js_string(name), ctx.val(*v))
                    }
                })
                .collect();
            let _ = writeln!(
                out,
                "{indent}{pfx}{} = {{ {} }};",
                ctx.val(r),
                field_strs.join(", ")
            );
        }
        Op::ArrayInit(elems) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let elems_str = elems
                .iter()
                .map(|v| ctx.val(*v))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(out, "{indent}{pfx}{} = [{elems_str}];", ctx.val(r));
        }
        Op::TupleInit(elems) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let elems_str = elems
                .iter()
                .map(|v| ctx.val(*v))
                .collect::<Vec<_>>()
                .join(", ");
            let ty_str = ts_type(&func.value_types[r]);
            let _ = writeln!(
                out,
                "{indent}{pfx}{} = [{elems_str}] as {ty_str};",
                ctx.val(r)
            );
        }

        // -- Coroutines --
        Op::Yield(v) => {
            if let Some(r) = result {
                let pfx = ctx.let_prefix(r);
                match v {
                    Some(yv) => {
                        let _ = writeln!(
                            out,
                            "{indent}{pfx}{} = yield {};",
                            ctx.val(r),
                            ctx.val(*yv)
                        );
                    }
                    None => {
                        let _ = writeln!(out, "{indent}{pfx}{} = yield;", ctx.val(r));
                    }
                }
            } else {
                match v {
                    Some(yv) => {
                        let _ = writeln!(out, "{indent}yield {};", ctx.val(*yv));
                    }
                    None => {
                        let _ = writeln!(out, "{indent}yield;");
                    }
                }
            }
        }
        Op::CoroutineCreate {
            func: fname,
            args,
        } => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let args_str = args
                .iter()
                .map(|a| ctx.val(*a))
                .collect::<Vec<_>>()
                .join(", ");
            let _ = writeln!(
                out,
                "{indent}{pfx}{} = {}({args_str});",
                ctx.val(r),
                sanitize_ident(fname)
            );
        }
        Op::CoroutineResume(v) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(
                out,
                "{indent}{pfx}{} = {}.next();",
                ctx.val(r),
                ctx.val(*v)
            );
        }

        // -- Misc --
        Op::GlobalRef(name) => {
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(
                out,
                "{indent}{pfx}{} = {};",
                ctx.val(r),
                sanitize_ident(name)
            );
        }
        Op::Copy(src) => {
            // Not skipped (multi-use or other reason).
            let r = result.unwrap();
            let pfx = ctx.let_prefix(r);
            let _ = writeln!(out, "{indent}{pfx}{} = {};", ctx.val(r), ctx.val(*src));
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn emit_binop(
    ctx: &EmitCtx,
    out: &mut String,
    indent: &str,
    result: Option<ValueId>,
    a: &ValueId,
    b: &ValueId,
    op: &str,
) {
    let r = result.unwrap();
    let pfx = ctx.let_prefix(r);
    let _ = writeln!(
        out,
        "{indent}{pfx}{} = {} {op} {};",
        ctx.val(r),
        ctx.val(*a),
        ctx.val(*b)
    );
}

fn emit_constant(c: &Constant) -> String {
    match c {
        Constant::Null => "null".into(),
        Constant::Bool(b) => b.to_string(),
        Constant::Int(n) => n.to_string(),
        Constant::UInt(n) => n.to_string(),
        Constant::Float(f) => format_float(*f),
        Constant::String(s) => format!("\"{}\"", escape_js_string(s)),
    }
}

fn format_float(f: f64) -> String {
    if f.fract() == 0.0 && f.is_finite() {
        format!("{f:.1}")
    } else {
        format!("{f}")
    }
}

fn escape_js_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            _ => out.push(ch),
        }
    }
    out
}

fn visibility_prefix(vis: Visibility) -> &'static str {
    match vis {
        Visibility::Public => "export ",
        Visibility::Private => "",
    }
}

/// Assign branch args to the target block's parameter variables.
fn emit_branch_args(
    ctx: &EmitCtx,
    func: &Function,
    target: reincarnate_core::ir::BlockId,
    args: &[ValueId],
    out: &mut String,
    indent: &str,
) {
    let target_block = &func.blocks[target];
    for (param, arg) in target_block.params.iter().zip(args.iter()) {
        let _ = writeln!(
            out,
            "{indent}{} = {};",
            ctx.val(param.value),
            ctx.val(*arg)
        );
    }
}

/// Generate a TypeScript type-check expression.
fn type_check_expr(ctx: &EmitCtx, v: ValueId, ty: &Type) -> String {
    match ty {
        Type::Bool => format!("typeof {} === \"boolean\"", ctx.val(v)),
        Type::Int(_) | Type::UInt(_) | Type::Float(_) => {
            format!("typeof {} === \"number\"", ctx.val(v))
        }
        Type::String => format!("typeof {} === \"string\"", ctx.val(v)),
        Type::Struct(name) | Type::Enum(name) => {
            format!("{} instanceof {}", ctx.val(v), sanitize_ident(name))
        }
        _ => format!("typeof {} === \"object\"", ctx.val(v)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use reincarnate_core::ir::builder::{FunctionBuilder, ModuleBuilder};
    use reincarnate_core::ir::{
        EnumDef, EnumVariant, FunctionSig, Global, Import, StructDef, Visibility,
    };

    fn build_and_emit(build: impl FnOnce(&mut ModuleBuilder)) -> String {
        let mut mb = ModuleBuilder::new("test");
        build(&mut mb);
        emit_module_to_string(&mb.build()).unwrap()
    }

    #[test]
    fn simple_add_function() {
        let out = build_and_emit(|mb| {
            let sig = FunctionSig {
                params: vec![Type::Int(64), Type::Int(64)],
                return_ty: Type::Int(64),
            };
            let mut fb = FunctionBuilder::new("add", sig, Visibility::Public);
            let a = fb.param(0);
            let b = fb.param(1);
            let sum = fb.add(a, b);
            fb.ret(Some(sum));
            mb.add_function(fb.build());
        });

        assert!(out.contains("export function add(v0: number, v1: number): number {"));
        assert!(out.contains("let v2 = v0 + v1;"));
        assert!(out.contains("return v2;"));
        // Single block → no dispatch loop.
        assert!(!out.contains("$block"));
    }

    #[test]
    fn branching_with_block_args() {
        let out = build_and_emit(|mb| {
            let sig = FunctionSig {
                params: vec![Type::Bool, Type::Int(64), Type::Int(64)],
                return_ty: Type::Int(64),
            };
            let mut fb = FunctionBuilder::new("choose", sig, Visibility::Public);

            let cond = fb.param(0);
            let x = fb.param(1);
            let y = fb.param(2);

            let (then_block, then_vals) = fb.create_block_with_params(&[Type::Int(64)]);
            let (else_block, else_vals) = fb.create_block_with_params(&[Type::Int(64)]);

            fb.br_if(cond, then_block, &[x], else_block, &[y]);

            fb.switch_to_block(then_block);
            fb.ret(Some(then_vals[0]));

            fb.switch_to_block(else_block);
            fb.ret(Some(else_vals[0]));

            mb.add_function(fb.build());
        });

        assert!(out.contains("$block"));
        assert!(out.contains("switch ($block)"));
        assert!(out.contains("if (v0)"));
        // Block args assignment.
        assert!(out.contains("v3 = v1;"));
        assert!(out.contains("v4 = v2;"));
    }

    #[test]
    fn struct_emission() {
        let out = build_and_emit(|mb| {
            mb.add_struct(StructDef {
                name: "Point".into(),
                fields: vec![
                    ("x".into(), Type::Float(64)),
                    ("y".into(), Type::Float(64)),
                ],
                visibility: Visibility::Public,
            });
        });

        assert!(out.contains("export interface Point {"));
        assert!(out.contains("  x: number;"));
        assert!(out.contains("  y: number;"));
    }

    #[test]
    fn enum_emission() {
        let out = build_and_emit(|mb| {
            mb.add_enum(EnumDef {
                name: "Shape".into(),
                variants: vec![
                    EnumVariant {
                        name: "Circle".into(),
                        fields: vec![Type::Float(64)],
                    },
                    EnumVariant {
                        name: "Rect".into(),
                        fields: vec![Type::Float(64), Type::Float(64)],
                    },
                ],
                visibility: Visibility::Public,
            });
        });

        assert!(out.contains("export type Shape ="));
        assert!(out.contains("tag: \"Circle\""));
        assert!(out.contains("tag: \"Rect\""));
    }

    #[test]
    fn global_variables() {
        let out = build_and_emit(|mb| {
            mb.add_global(Global {
                name: "counter".into(),
                ty: Type::Int(64),
                visibility: Visibility::Public,
                mutable: true,
            });
            mb.add_global(Global {
                name: "MAX_SIZE".into(),
                ty: Type::Int(64),
                visibility: Visibility::Private,
                mutable: false,
            });
        });

        assert!(out.contains("export let counter: number;"));
        assert!(out.contains("const MAX_SIZE: number;"));
    }

    #[test]
    fn imports() {
        let out = build_and_emit(|mb| {
            mb.add_import(Import {
                module: "utils".into(),
                name: "helper".into(),
                alias: None,
            });
            mb.add_import(Import {
                module: "math".into(),
                name: "add".into(),
                alias: Some("mathAdd".into()),
            });
        });

        assert!(out.contains("import { helper } from \"./utils\";"));
        assert!(out.contains("import { add as mathAdd } from \"./math\";"));
    }

    #[test]
    fn system_call() {
        let out = build_and_emit(|mb| {
            let sig = FunctionSig {
                params: vec![],
                return_ty: Type::Void,
            };
            let mut fb = FunctionBuilder::new("init", sig, Visibility::Public);
            let x = fb.const_int(100);
            let y = fb.const_int(200);
            fb.system_call("renderer", "clear", &[x, y], Type::Void);
            fb.ret(None);
            mb.add_function(fb.build());
        });

        // Auto-injected runtime import.
        assert!(out.contains("import { renderer } from \"./runtime\";"));
        assert!(out.contains("renderer.clear(v0, v1);"));
    }

    #[test]
    fn multiple_system_imports() {
        let out = build_and_emit(|mb| {
            let sig = FunctionSig {
                params: vec![],
                return_ty: Type::Void,
            };
            let mut fb = FunctionBuilder::new("tick", sig, Visibility::Public);
            fb.system_call("timing", "tick", &[], Type::Void);
            fb.system_call("input", "update", &[], Type::Void);
            fb.system_call("renderer", "present", &[], Type::Void);
            fb.ret(None);
            mb.add_function(fb.build());
        });

        assert!(out.contains("import { input, renderer, timing } from \"./runtime\";"));
    }

    #[test]
    fn no_runtime_import_without_system_calls() {
        let out = build_and_emit(|mb| {
            let sig = FunctionSig {
                params: vec![],
                return_ty: Type::Void,
            };
            let mut fb = FunctionBuilder::new("noop", sig, Visibility::Public);
            fb.ret(None);
            mb.add_function(fb.build());
        });

        assert!(!out.contains("import"));
    }

    #[test]
    fn constants_all_types() {
        let out = build_and_emit(|mb| {
            let sig = FunctionSig {
                params: vec![],
                return_ty: Type::Void,
            };
            let mut fb = FunctionBuilder::new("constants", sig, Visibility::Public);
            fb.const_null();
            fb.const_bool(true);
            fb.const_bool(false);
            fb.const_int(42);
            fb.const_float(3.125);
            fb.const_string("hello \"world\"\nnewline");
            fb.ret(None);
            mb.add_function(fb.build());
        });

        assert!(out.contains("let v0 = null;"));
        assert!(out.contains("let v1 = true;"));
        assert!(out.contains("let v2 = false;"));
        assert!(out.contains("let v3 = 42;"));
        assert!(out.contains("let v4 = 3.125;"));
        assert!(out.contains(r#"let v5 = "hello \"world\"\nnewline";"#));
    }

    #[test]
    fn array_and_struct_init() {
        let out = build_and_emit(|mb| {
            let sig = FunctionSig {
                params: vec![],
                return_ty: Type::Void,
            };
            let mut fb = FunctionBuilder::new("init", sig, Visibility::Public);

            let a = fb.const_int(1);
            let b = fb.const_int(2);
            fb.array_init(&[a, b], Type::Int(64));

            let x = fb.const_float(10.0);
            let y = fb.const_float(20.0);
            fb.struct_init("Point", vec![("x".into(), x), ("y".into(), y)]);

            fb.ret(None);
            mb.add_function(fb.build());
        });

        assert!(out.contains("let v2 = [v0, v1];"));
        assert!(out.contains("let v5 = { x: v3, y: v4 };"));
    }

    #[test]
    fn copy_propagation_eliminates_alloc_store_load() {
        let out = build_and_emit(|mb| {
            let sig = FunctionSig {
                params: vec![Type::Int(64)],
                return_ty: Type::Int(64),
            };
            let mut fb = FunctionBuilder::new("identity", sig, Visibility::Public);
            let param = fb.param(0);
            // Alloc → Store → Load chain (typical local variable pattern).
            let ptr = fb.alloc(Type::Int(64));
            fb.store(ptr, param);
            let loaded = fb.load(ptr, Type::Int(64));
            fb.ret(Some(loaded));
            mb.add_function(fb.build());
        });

        // The alloc/store/load should be eliminated; return refers to the
        // original parameter directly.
        assert!(out.contains("return v0;"));
        assert!(!out.contains("undefined"));
    }

    #[test]
    fn sanitize_ident_handles_avm2_names() {
        assert_eq!(sanitize_ident("Flash.Object"), "Flash_Object");
        assert_eq!(sanitize_ident("flash.display::Loader"), "flash_display__Loader");
        assert_eq!(sanitize_ident("4l9JT7u2nN1ZFk+5"), "_4l9JT7u2nN1ZFk_5");
        assert_eq!(sanitize_ident("l/YEs377IakicDh/"), "l_YEs377IakicDh_");
        assert_eq!(sanitize_ident("normal_name"), "normal_name");
    }

    #[test]
    fn bracket_notation_for_non_ident_fields() {
        let out = build_and_emit(|mb| {
            let sig = FunctionSig {
                params: vec![Type::Dynamic],
                return_ty: Type::Dynamic,
            };
            let mut fb = FunctionBuilder::new("get_prop", sig, Visibility::Public);
            let obj = fb.param(0);
            let result = fb.get_field(obj, "flash.display::Loader", Type::Dynamic);
            fb.ret(Some(result));
            mb.add_function(fb.build());
        });

        // Non-ident field name should use bracket notation.
        assert!(out.contains("[\"flash.display::Loader\"]"));
        assert!(!out.contains(".flash.display::Loader"));
    }
}
