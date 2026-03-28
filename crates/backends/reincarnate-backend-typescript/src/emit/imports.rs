// ---------------------------------------------------------------------------
// Import generation — runtime, function, intra-module, and external imports
// ---------------------------------------------------------------------------

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write;

use reincarnate_core::entity::PrimaryMap;
use reincarnate_core::ir::module::TypeDecl;
use reincarnate_core::ir::{CastKind, ExternalImport, Function, Module, Op, Type, TypeId};
use reincarnate_core::project::RuntimeConfig;

use crate::runtime::SYSTEM_NAMES;

use super::{sanitize_ident, ClassGroup, ClassRegistry, EngineKind, RefSets};

/// Compute a relative import path between two sets of path segments.
///
/// Both `from` and `to` are file-level segments (the last element is the
/// filename without extension).
pub(super) fn relative_import_path(from: &[String], to: &[String]) -> String {
    // Find common prefix length (only among directory segments, not filenames).
    let from_dirs = from.len().saturating_sub(1);
    let to_dirs = to.len().saturating_sub(1);
    let common = from[..from_dirs]
        .iter()
        .zip(to[..to_dirs].iter())
        .take_while(|(a, b)| a == b)
        .count();

    // Go up from `from`'s directory.
    let ups = from_dirs - common;
    let mut parts = Vec::new();
    if ups == 0 {
        parts.push(".".to_string());
    } else {
        for _ in 0..ups {
            parts.push("..".to_string());
        }
    }
    // Go down into `to`'s remaining path.
    for seg in &to[common..] {
        parts.push(seg.clone());
    }
    parts.join("/")
}

// ---------------------------------------------------------------------------
// Runtime imports (auto-detected from SystemCall ops)
// ---------------------------------------------------------------------------

pub(super) fn collect_system_names_from_funcs<'a>(
    funcs: impl Iterator<Item = &'a Function>,
    // Optional map from intrinsic `Op::Call` function names to their system name.
    // Used to find system modules for GML intrinsic calls (which are `Op::Call`
    // rather than `Op::SystemCall` after Phase 3a migration).
    intrinsic_to_system: Option<&std::collections::HashMap<String, String>>,
) -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    for func in funcs {
        for (_inst_id, inst) in func.insts.iter() {
            match &inst.op {
                Op::SystemCall { system, .. } => {
                    used.insert(system.clone());
                }
                Op::Call { func: name, .. } => {
                    if let Some(map) = intrinsic_to_system {
                        if let Some(system) = map.get(name.as_str()) {
                            used.insert(system.clone());
                        }
                    }
                }
                _ => {}
            }
        }
    }
    used
}

/// Build the full intrinsic calls map: call name → (system, method).
///
/// Used by `collect_type_refs_from_function` to handle `Op::Call` with
/// intrinsic names the same way as the equivalent `Op::SystemCall`.
pub(super) fn build_intrinsic_calls_map(module: &Module) -> HashMap<String, (String, String)> {
    module
        .runtime_registry
        .iter()
        .filter_map(|(name, &fid)| {
            let func = &module.functions[fid];
            func.intrinsic.as_ref().map(|kind| {
                let (system, method) = kind.system_method();
                (name.clone(), (system.to_string(), method.to_string()))
            })
        })
        .collect()
}

/// Build the `intrinsic_to_system` map used by `collect_system_names_from_funcs`.
///
/// Maps each registered intrinsic call name (e.g. `"GameMaker.Instance.getField"`)
/// to its system name (e.g. `"GameMaker.Instance"`).  Only functions with an
/// `IntrinsicKind` are included.
pub(super) fn build_intrinsic_to_system(module: &Module) -> HashMap<String, String> {
    module
        .runtime_registry
        .iter()
        .filter_map(|(name, &fid)| {
            let func = &module.functions[fid];
            func.intrinsic.as_ref().map(|kind| {
                let (system, _method) = kind.system_method();
                (name.clone(), system.to_string())
            })
        })
        .collect()
}

/// Emit runtime imports for flat modules (files directly in `output_dir`).
pub(super) fn emit_runtime_imports(
    module: &Module,
    out: &mut String,
    runtime_config: Option<&RuntimeConfig>,
    engine: EngineKind,
    stateful_system_aliases: &mut BTreeMap<String, String>,
) {
    let all_funcs = || module.functions.iter().map(|(_id, f)| f);
    let intrinsic_to_system = build_intrinsic_to_system(module);
    let systems = collect_system_names_from_funcs(all_funcs(), Some(&intrinsic_to_system));
    emit_runtime_imports_with_prefix(systems, out, ".", runtime_config, stateful_system_aliases);
    let intrinsic_calls = build_intrinsic_calls_map(module);
    let calls = collect_call_names_from_funcs(all_funcs(), engine, Some(&intrinsic_calls));
    let mut _flat_stateful = BTreeSet::new();
    emit_function_imports_with_prefix(&calls, out, ".", runtime_config, &mut _flat_stateful);
}

/// Emit runtime imports for files inside a module directory.
///
/// `depth` is the number of namespace directories below the module dir. The
/// module dir itself is one level inside `output_dir`, so the prefix traverses
/// `depth + 1` parent directories.
pub(super) fn emit_runtime_imports_for(
    systems: BTreeSet<String>,
    out: &mut String,
    depth: usize,
    runtime_config: Option<&RuntimeConfig>,
    stateful_system_aliases: &mut BTreeMap<String, String>,
) {
    let prefix = "../".repeat(depth + 1);
    let prefix = prefix.trim_end_matches('/');
    emit_runtime_imports_with_prefix(
        systems,
        out,
        prefix,
        runtime_config,
        stateful_system_aliases,
    );
}

pub(super) fn emit_runtime_imports_with_prefix(
    systems: BTreeSet<String>,
    out: &mut String,
    prefix: &str,
    runtime_config: Option<&RuntimeConfig>,
    stateful_system_aliases: &mut BTreeMap<String, String>,
) {
    if systems.is_empty() {
        return;
    }
    let known: BTreeSet<&str> = SYSTEM_NAMES.iter().copied().collect();
    let mut generic: Vec<&str> = Vec::new();
    // Group engine-specific systems by their runtime sub-module path.
    let mut by_mod: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for sys in &systems {
        if known.contains(sys.as_str()) {
            generic.push(sys.as_str());
        } else if let Some(sm) = runtime_config.and_then(|c| c.system_modules.get(sys.as_str())) {
            if sm.stateful.unwrap_or(false) {
                // Stateful system module — skip namespace import; will be
                // aliased from `_rt` inside each function.
                let ident = sanitize_ident(sys);
                // Property name: strip engine prefix (e.g. "SugarCube.Output" → "Output").
                let prop = sys.split('.').next_back().unwrap_or(sys).to_string();
                stateful_system_aliases.insert(ident, prop);
            } else {
                by_mod
                    .entry(sm.path.clone())
                    .or_default()
                    .push(sanitize_ident(sys));
            }
        } else {
            // Fallback: derive module path from system name.
            let module = sys
                .split('.')
                .next_back()
                .unwrap_or(sys)
                .to_ascii_lowercase();
            by_mod.entry(module).or_default().push(sanitize_ident(sys));
        }
    }
    if !generic.is_empty() {
        let _ = writeln!(
            out,
            "import {{ {} }} from \"{prefix}/runtime\";",
            generic.join(", ")
        );
    }
    for (module, names) in &by_mod {
        // Namespace imports enable tree-shaking of individual methods.
        for name in names {
            let _ = writeln!(
                out,
                "import * as {name} from \"{prefix}/runtime/{module}\";",
            );
        }
    }
    if !generic.is_empty() || !by_mod.is_empty() {
        out.push('\n');
    }
}

/// Remove `import * as NAME from "...";` lines where `NAME.` never appears
/// in the rest of the output.  Engine-specific rewrite passes (e.g. GameMaker)
/// can eliminate all usages of a namespace import after the import collector
/// has already added it; this post-pass cleans them up.
pub(super) fn strip_unused_namespace_imports(out: &mut String) {
    // Collect (line_start, line_end, alias) for every namespace import.
    let mut removals: Vec<(usize, usize)> = Vec::new();
    for (start, line) in out.match_indices("import * as ").collect::<Vec<_>>() {
        // `line` is always the literal "import * as " prefix.
        let _ = line;
        let after_prefix = start + "import * as ".len();
        let rest = &out[after_prefix..];
        // Extract the alias (up to the next space or ' from').
        let alias_end = rest.find([' ', '\n']).unwrap_or(rest.len());
        let alias = &rest[..alias_end];
        if alias.is_empty() {
            continue;
        }
        // Find the end of this line (including the newline).
        let line_end = out[start..]
            .find('\n')
            .map(|i| start + i + 1)
            .unwrap_or(out.len());

        // Check whether `ALIAS.` appears anywhere AFTER this import line.
        let needle = format!("{alias}.");
        if !out[line_end..].contains(&needle) {
            removals.push((start, line_end));
        }
    }

    // Also strip `import { Sprites } from "..."` when `Sprites.` never appears.
    for (start, _) in out.match_indices("import { Sprites }").collect::<Vec<_>>() {
        let line_end = out[start..]
            .find('\n')
            .map(|i| start + i + 1)
            .unwrap_or(out.len());
        let tail = &out[line_end..];
        if !tail.contains("Sprites.") && !tail.contains("Sprites[") {
            removals.push((start, line_end));
        }
    }

    // Remove in reverse order to preserve byte offsets.
    for &(start, end) in removals.iter().rev() {
        out.drain(start..end);
    }
}

// ---------------------------------------------------------------------------
// Function-level imports (runtime stdlib free functions)
// ---------------------------------------------------------------------------

/// Collect all direct `Call` function names from a set of IR functions,
/// plus any bare function names introduced by engine-specific rewrites
/// or backend cast printing (e.g. `int(x)` from `Coerce + Int(32)`).
///
/// `intrinsic_calls`: optional map from intrinsic call names (e.g.
/// `"GameMaker.Instance.getField"`) to their `(system, method)` pairs.
/// When provided, intrinsic names are excluded from the free-function import
/// set (they lower to `Expr::SystemCall`, not free-function calls), and any
/// secondary names introduced by the engine rewrite pass are added instead.
pub(super) fn collect_call_names_from_funcs<'a>(
    funcs: impl Iterator<Item = &'a Function>,
    engine: EngineKind,
    intrinsic_calls: Option<&HashMap<String, (String, String)>>,
) -> BTreeSet<String> {
    let mut used = BTreeSet::new();
    for func in funcs {
        for (_inst_id, inst) in func.insts.iter() {
            match &inst.op {
                Op::Call { func: name, .. } => {
                    // If this is a registered intrinsic, emit the names that the
                    // rewrite pass will introduce (same as Op::SystemCall did) rather
                    // than the intrinsic name itself (which lowers to Expr::SystemCall).
                    if let Some((system, method)) =
                        intrinsic_calls.and_then(|m| m.get(name.as_str()))
                    {
                        if engine == EngineKind::GameMaker {
                            for introduced in
                                crate::rewrites::gamemaker::rewrite_introduced_calls(system, method)
                            {
                                used.insert((*introduced).to_string());
                            }
                        }
                        continue;
                    }
                    // Sanitize the raw IR name (e.g. `@@SetStatic@@` → `__SetStatic__`)
                    // so it matches the sanitized names stored in runtime.json.
                    used.insert(sanitize_ident(name));
                    if engine == EngineKind::GameMaker {
                        for introduced in
                            crate::rewrites::gamemaker::rewrite_introduced_direct_calls(name)
                        {
                            used.insert((*introduced).to_string());
                        }
                    }
                }
                Op::SystemCall { system, method, .. } if engine == EngineKind::GameMaker => {
                    for name in crate::rewrites::gamemaker::rewrite_introduced_calls(system, method)
                    {
                        used.insert((*name).to_string());
                    }
                }
                Op::SystemCall { system, method, .. } if engine == EngineKind::Twine => {
                    for name in crate::rewrites::twine::rewrite_introduced_calls(system, method) {
                        used.insert((*name).to_string());
                    }
                }
                // Function/asset references used as values (via @@pushref@@) — these
                // appear as bare Var nodes in the emitted JS and need the same import
                // treatment as direct calls.  Names are sanitized (e.g. `anon@N@...` →
                // `anon_N_...`) at print time; sanitize here too so runtime.json lookup works.
                Op::GlobalRef(name) => {
                    used.insert(sanitize_ident(name));
                    if engine == EngineKind::GameMaker {
                        for introduced in
                            crate::rewrites::gamemaker::rewrite_introduced_direct_calls(name)
                        {
                            used.insert((*introduced).to_string());
                        }
                    }
                }
                // Coerce casts emit bare function calls: int(x), uint(x).
                Op::Cast(_, Type::Int(32), CastKind::Coerce) => {
                    used.insert("int".to_string());
                }
                Op::Cast(_, Type::UInt(32), CastKind::Coerce) => {
                    used.insert("uint".to_string());
                }
                // Non-Coerce casts and casts to other types don't introduce free function calls.
                Op::Cast(..) => {}
                // Ops that don't introduce runtime free-function imports:
                Op::Const(_)
                | Op::Select { .. }
                | Op::Alloc(_)
                | Op::Load(_)
                | Op::Store { .. }
                | Op::GetField { .. }
                | Op::SetField { .. }
                | Op::GetIndex { .. }
                | Op::SetIndex { .. }
                | Op::MakeClosure { .. }
                | Op::CallIndirect { .. }
                | Op::SystemCall { .. }
                | Op::MethodCall { .. }
                | Op::TypeCheck(..)
                | Op::StructInit { .. }
                | Op::ArrayInit(_)
                | Op::TupleInit(_)
                | Op::Yield(_)
                | Op::CoroutineCreate { .. }
                | Op::CoroutineResume(_)
                | Op::Cmp(..)
                | Op::Spread(_) => {}
            }
        }
    }
    used
}

/// Emit import statements for runtime-provided free functions.
///
/// Scans `call_names` against `function_modules` in the runtime config,
/// groups matches by module path, and emits one import per module.
///
/// Functions from modules marked `stateful: true` are NOT imported — instead
/// their names are collected in `stateful_out` for destructuring from the
/// runtime instance at the call site.
pub(super) fn emit_function_imports_with_prefix(
    call_names: &BTreeSet<String>,
    out: &mut String,
    prefix: &str,
    runtime_config: Option<&RuntimeConfig>,
    stateful_out: &mut BTreeSet<String>,
) {
    let Some(cfg) = runtime_config else { return };
    if cfg.function_modules.is_empty() {
        return;
    }
    // Build reverse map: function_name → (module_path, stateful).
    let mut func_to_module: HashMap<&str, (&str, bool)> = HashMap::new();
    for group in &cfg.function_modules {
        let is_stateful = group.stateful.unwrap_or(false);
        for name in &group.names {
            func_to_module.insert(name.as_str(), (group.path.as_str(), is_stateful));
        }
    }
    // Group needed imports by module path (pure only).
    let mut by_mod: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
    for name in call_names {
        if let Some(&(path, stateful)) = func_to_module.get(name.as_str()) {
            if stateful {
                stateful_out.insert(name.clone());
            } else {
                by_mod.entry(path).or_default().insert(name.as_str());
            }
        }
    }
    for (module, names) in &by_mod {
        let names: Vec<&str> = names.iter().copied().collect();
        let _ = writeln!(
            out,
            "import {{ {} }} from \"{prefix}/runtime/{module}\";",
            names.join(", "),
        );
    }
    if !by_mod.is_empty() {
        out.push('\n');
    }
}

/// Emit import statements for user-defined free functions that live in `_init.ts`.
///
/// Any `Op::Call` name that matches a free function in the same module gets an
/// `import { name } from "<prefix>/_init"` line.
pub(super) fn emit_free_function_imports(
    call_names: &BTreeSet<String>,
    free_func_names: &HashSet<String>,
    depth: usize,
    out: &mut String,
) {
    let needed: BTreeSet<&str> = call_names
        .iter()
        .filter(|n| free_func_names.contains(n.as_str()))
        .map(|n| n.as_str())
        .collect();
    if needed.is_empty() {
        return;
    }
    // Sanitize identifier names (e.g. `anon@1155@...` → `anon_1155_...`) to match
    // what `sanitize_ident` produces for the exported function name.
    let mut names: Vec<String> = needed.into_iter().map(sanitize_ident).collect();
    names.sort();
    names.dedup();
    // _init.ts lives in module_dir, so we go up `depth` levels (one per
    // namespace segment).  depth=0 → "./_init", depth=1 → "../_init", etc.
    let prefix = if depth == 0 {
        ".".to_string()
    } else {
        "../".repeat(depth).trim_end_matches('/').to_string()
    };
    let _ = writeln!(
        out,
        "import {{ {} }} from \"{prefix}/_init\";",
        names.join(", ")
    );
    out.push('\n');
}

// ---------------------------------------------------------------------------
// Intra-module imports (class-to-class references)
// ---------------------------------------------------------------------------

/// Emit a categorized warning for an unmapped external reference, filtering
/// out known false positives (private/protected namespace accesses, `fl.*`
/// authoring library types).
pub(super) fn warn_unmapped_reference(name: &str) {
    let (ns, _short) = name.rsplit_once("::").unwrap_or(("", name));
    // No namespace at all — bare name, not a qualified reference.
    if ns.is_empty() {
        return;
    }
    // Private/protected namespace member access (e.g. "classes:Monster::consumables").
    // These contain a colon within the namespace portion.
    if ns.contains(':') {
        return;
    }
    // Flash authoring library — not part of the runtime.
    if crate::rewrites::flash::is_known_flash_namespace(ns) {
        return;
    }
    // Flash runtime stdlib — actionable: add to runtime.
    if let Some(pkg) = ns.strip_prefix("flash.") {
        eprintln!("warning: flash package '{pkg}' not in runtime stdlib (referenced: {name})");
        return;
    }
    eprintln!("warning: unmapped external reference: {name}");
}

/// Collect type names referenced by a class group, split into value and type-only refs.
///
/// **Value refs** (class constructor needed at runtime):
/// - `super_class` — `extends X` is a runtime expression
/// - `Op::TypeCheck` — emits `instanceof X`
///
/// **Type refs** (erased at runtime):
/// - Struct field types, function signatures, `Op::Alloc`, `Op::Cast`, `value_types`
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_class_references(
    group: &ClassGroup,
    module: &Module,
    registry: &ClassRegistry,
    external_imports: &BTreeMap<String, ExternalImport>,
    static_method_owners: &HashMap<String, String>,
    static_field_owners: &HashMap<String, String>,
    global_names: &HashSet<String>,
    unique_static_field_map: &HashMap<String, String>,
    engine: EngineKind,
    self_ts_name: &str,
) -> RefSets {
    let module_types = &module.types;
    let self_name = &group.class_def.name;
    let mut refs = RefSets::default();

    // Super class reference — runtime value (extends).
    if let Some(sc) = &group.class_def.super_class {
        let short = sc.rsplit("::").next().unwrap_or(sc);
        if short != self_name {
            if let Some(entry) = registry.lookup(sc) {
                refs.value_refs.insert(entry.short_name.clone());
            } else if external_imports.contains_key(sc) {
                refs.ext_value_refs.insert(sc.to_string());
            }
        }
    }

    // Interface references — runtime value (registerInterface needs constructors).
    for iface in &group.class_def.interfaces {
        let short = iface.rsplit("::").next().unwrap_or(iface);
        if short != self_name {
            if let Some(entry) = registry.lookup(iface) {
                refs.value_refs.insert(entry.short_name.clone());
            } else if external_imports.contains_key(iface.as_str()) {
                refs.ext_value_refs.insert(iface.to_string());
            }
        }
    }

    // Struct fields (class instance fields) — type-only.
    for field in &group.struct_def.fields {
        collect_type_ref(
            &field.ty,
            self_name,
            self_ts_name,
            module_types,
            registry,
            external_imports,
            &mut refs.type_refs,
            &mut refs.ext_type_refs,
        );
    }

    // Static fields — type-only (e.g. `static BEEHONY: Consumable` needs Consumable imported).
    for f in &group.class_def.static_fields {
        collect_type_ref(
            &f.ty,
            self_name,
            self_ts_name,
            module_types,
            registry,
            external_imports,
            &mut refs.type_refs,
            &mut refs.ext_type_refs,
        );
    }

    // Abstract member signatures — type-only.
    // Interfaces may declare abstract getters/setters/methods with types like
    // `InteractiveObject` that have no method body to scan.
    for m in &group.class_def.abstract_members {
        collect_type_ref(
            &m.return_ty,
            self_name,
            self_ts_name,
            module_types,
            registry,
            external_imports,
            &mut refs.type_refs,
            &mut refs.ext_type_refs,
        );
        for param_ty in &m.params {
            collect_type_ref(
                param_ty,
                self_name,
                self_ts_name,
                module_types,
                registry,
                external_imports,
                &mut refs.type_refs,
                &mut refs.ext_type_refs,
            );
        }
    }

    // Scan all method bodies for type references.
    let intrinsic_calls = build_intrinsic_calls_map(module);
    for &fid in &group.methods {
        let func = &module.functions[fid];
        collect_type_refs_from_function(
            func,
            self_name,
            self_ts_name,
            module_types,
            registry,
            external_imports,
            static_method_owners,
            static_field_owners,
            global_names,
            unique_static_field_map,
            &module.object_names,
            engine,
            Some(&intrinsic_calls),
            &mut refs,
        );
    }

    refs
}

/// Scan a function's instructions and signature for type references.
///
/// `intrinsic_calls`: optional map from intrinsic `Op::Call` function names to
/// `(system, method)` pairs.  Used to handle GML intrinsic calls (which appear
/// as `Op::Call` in the IR after Phase 3a) with the same import logic as the
/// equivalent `Op::SystemCall` variants they replaced.
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_type_refs_from_function(
    func: &Function,
    self_name: &str,
    self_ts_name: &str,
    module_types: &PrimaryMap<TypeId, TypeDecl>,
    registry: &ClassRegistry,
    external_imports: &BTreeMap<String, ExternalImport>,
    static_method_owners: &HashMap<String, String>,
    static_field_owners: &HashMap<String, String>,
    global_names: &HashSet<String>,
    unique_static_field_map: &HashMap<String, String>,
    object_names: &[String],
    engine: EngineKind,
    intrinsic_calls: Option<&HashMap<String, (String, String)>>,
    refs: &mut RefSets,
) {
    use reincarnate_core::ir::Constant;

    // Return type and param types — type-only.
    collect_type_ref(
        &func.sig.return_ty,
        self_name,
        self_ts_name,
        module_types,
        registry,
        external_imports,
        &mut refs.type_refs,
        &mut refs.ext_type_refs,
    );
    for ty in &func.sig.params {
        collect_type_ref(
            ty,
            self_name,
            self_ts_name,
            module_types,
            registry,
            external_imports,
            &mut refs.type_refs,
            &mut refs.ext_type_refs,
        );
    }

    // Build ValueId → &str map for const strings (to resolve SystemCall args).
    let const_strings: HashMap<_, _> = func
        .insts
        .iter()
        .filter_map(|(_id, inst)| {
            if let Op::Const(Constant::String(s)) = &inst.op {
                inst.result.map(|v| (v, s.as_str()))
            } else {
                None
            }
        })
        .collect();

    let direct_const_ints: HashMap<_, _> = func
        .insts
        .iter()
        .filter_map(|(_id, inst)| {
            if let Op::Const(Constant::Int(n)) = &inst.op {
                inst.result.map(|v| (v, *n))
            } else {
                None
            }
        })
        .collect();

    // Also track integers reachable through Coerce-to-Unknown (`coerce v_int, dyn`).
    // GML bytecode sometimes widens a const integer to `dyn` before passing it
    // to a SystemCall; the coerced ValueId is the actual syscall argument, but
    // the integer value lives on the source ValueId one step earlier.
    let mut const_ints = direct_const_ints.clone();
    for (_id, inst) in func.insts.iter() {
        if let Op::Cast(src, Type::Unknown, CastKind::Coerce) = &inst.op {
            if let (Some(result), Some(&n)) = (inst.result, direct_const_ints.get(src)) {
                const_ints.insert(result, n);
            }
        }
    }

    // Instructions.
    for (_inst_id, inst) in func.insts.iter() {
        match &inst.op {
            // TypeCheck emits `isType()` — runtime value reference, collected
            // separately so circular imports can be detected and late-bound.
            Op::TypeCheck(_, ty) => {
                let is_struct_or_enum = matches!(ty, Type::Instance(_) | Type::ClassRef(_));
                if is_struct_or_enum {
                    collect_type_ref(
                        ty,
                        self_name,
                        self_ts_name,
                        module_types,
                        registry,
                        external_imports,
                        &mut refs.typecheck_value_refs,
                        &mut refs.ext_value_refs,
                    );
                } else {
                    collect_type_ref(
                        ty,
                        self_name,
                        self_ts_name,
                        module_types,
                        registry,
                        external_imports,
                        &mut refs.value_refs,
                        &mut refs.ext_value_refs,
                    );
                }
            }
            // Alloc is type-only. Cast with Instance/Enum: NullableCoerce needs runtime value, Coerce is type-only.
            Op::Alloc(ty) => {
                collect_type_ref(
                    ty,
                    self_name,
                    self_ts_name,
                    module_types,
                    registry,
                    external_imports,
                    &mut refs.type_refs,
                    &mut refs.ext_type_refs,
                );
            }
            Op::Cast(_, ty, kind) => {
                let is_struct_or_enum = matches!(ty, Type::Instance(_) | Type::ClassRef(_));
                if is_struct_or_enum && *kind == CastKind::NullableCoerce {
                    // NullableCoerce needs runtime constructor — collected separately
                    // so circular imports can be detected and late-bound.
                    collect_type_ref(
                        ty,
                        self_name,
                        self_ts_name,
                        module_types,
                        registry,
                        external_imports,
                        &mut refs.typecheck_value_refs,
                        &mut refs.ext_value_refs,
                    );
                } else {
                    collect_type_ref(
                        ty,
                        self_name,
                        self_ts_name,
                        module_types,
                        registry,
                        external_imports,
                        &mut refs.type_refs,
                        &mut refs.ext_type_refs,
                    );
                }
            }
            // GetField with a class name → runtime value reference (used with `new`).
            Op::GetField { field, .. } => {
                if registry.lookup(field).is_some() {
                    collect_named_type_ref(
                        field,
                        self_name,
                        self_ts_name,
                        registry,
                        external_imports,
                        &mut refs.value_refs,
                        &mut refs.ext_value_refs,
                    );
                } else {
                    let short = field.rsplit("::").next().unwrap_or(field);
                    if short != self_name {
                        if external_imports.contains_key(field.as_str()) {
                            refs.ext_value_refs.insert(field.to_string());
                        } else {
                            warn_unmapped_reference(field);
                        }
                    }
                }
                // Flash: if this field access will be rewritten to OwnerClass.FIELD by the
                // unique_static_fields rewrite, register OwnerClass as a value import.
                // Field names may be qualified (e.g. "classes.Parser:Parser::haltOnErrors"),
                // but the map is keyed by short name — strip the namespace prefix first.
                if engine == EngineKind::Flash {
                    let short_field = field.rsplit("::").next().unwrap_or(field.as_str());
                    if let Some(owner) = unique_static_field_map.get(short_field) {
                        if owner != self_name {
                            if let Some(entry) = registry.lookup(owner) {
                                refs.value_refs.insert(entry.short_name.clone());
                            }
                        }
                    }
                }
            }
            // Engine-specific SystemCall import extraction — delegated to rewrite modules.
            Op::SystemCall {
                system,
                method,
                args,
            } => {
                match engine {
                    EngineKind::Flash
                        if system == "Flash.Scope"
                            && (method == "findPropStrict" || method == "findProperty") =>
                    {
                        crate::rewrites::flash::collect_flash_scope_refs(
                            args,
                            &const_strings,
                            self_name,
                            registry,
                            external_imports,
                            static_method_owners,
                            static_field_owners,
                            global_names,
                            refs,
                        );
                    }
                    // getField / setField / getOn / setOn with a const-int first arg:
                    // the rewrite resolves the integer index to a class constructor
                    // reference `ObjName` (via resolve_instance_target + strip_int_coerce).
                    // Register the class as a value import so it is available.
                    // Also handles dyn-coerced integers (tracked in the extended const_ints).
                    //
                    // For getOn/setOn also handle the string-name case (→ instances[0]!).
                    EngineKind::GameMaker
                        if system == "GameMaker.Instance"
                            && (method == "getField"
                                || method == "setField"
                                || method == "getOn"
                                || method == "setOn")
                            && !args.is_empty() =>
                    {
                        // Integer class-index case: first arg is a const int
                        // (possibly dyn-coerced — covered by the extended const_ints).
                        if let Some(&obj_idx) = args.first().and_then(|v| const_ints.get(v)) {
                            if obj_idx >= 0 {
                                if let Some(obj_name) = object_names.get(obj_idx as usize) {
                                    if obj_name != self_name {
                                        if let Some(entry) = registry.lookup(obj_name) {
                                            refs.value_refs.insert(entry.short_name.clone());
                                        } else if external_imports.contains_key(obj_name.as_str()) {
                                            refs.ext_value_refs.insert(obj_name.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        // String object-name case (getOn/setOn only): first arg
                        // is a const string naming the class → instances[0]!.
                        if method == "getOn" || method == "setOn" {
                            crate::rewrites::gamemaker::collect_gamemaker_instance_refs(
                                args,
                                &const_strings,
                                self_name,
                                self_ts_name,
                                registry,
                                external_imports,
                                refs,
                            );
                        }
                    }
                    // GameMaker.Global.get/set → globals_used (the GML rewriter turns
                    // these into `global_name` / `$set_global_name(val)` references).
                    // Only add names that are known module-level globals (in _globals.ts);
                    // other global fields (like GMS2.3+ __class__, __enumIndex__) are
                    // accessed via the runtime `global` object, not module imports.
                    EngineKind::GameMaker
                        if system == "GameMaker.Global"
                            && (method == "get" || method == "set")
                            && !args.is_empty() =>
                    {
                        if let Some(field_name) = args.first().and_then(|v| const_strings.get(v)) {
                            if global_names.contains(*field_name) {
                                refs.globals_used.insert(field_name.to_string());
                            }
                        }
                    }
                    // Flash scope lookups handled above; other Flash/Twine SystemCalls
                    // don't introduce class-constructor imports.
                    EngineKind::Flash | EngineKind::Twine => {}
                    // GameMaker SystemCalls not matching the Instance guard above
                    // don't introduce class-constructor imports.
                    EngineKind::GameMaker => {}
                }
            }
            // Intrinsic Op::Call — same import logic as the equivalent Op::SystemCall.
            // After Phase 3a, GML syscalls appear as Op::Call with a registered
            // intrinsic name (e.g. "GameMaker.Instance.getField").  The linear
            // lowering pass will convert them to Expr::SystemCall, so the TS rewrite
            // passes see the same patterns as before — but we must collect imports here
            // in the core-IR scanner the same way we do for Op::SystemCall.
            Op::Call {
                func: call_name,
                args,
            } if intrinsic_calls
                .and_then(|m| m.get(call_name.as_str()))
                .is_some() =>
            {
                let (system, method) = intrinsic_calls
                    .and_then(|m| m.get(call_name.as_str()))
                    .unwrap();
                match engine {
                    EngineKind::GameMaker
                        if system == "GameMaker.Instance"
                            && (method == "getField"
                                || method == "setField"
                                || method == "getOn"
                                || method == "setOn")
                            && !args.is_empty() =>
                    {
                        if let Some(&obj_idx) = args.first().and_then(|v| const_ints.get(v)) {
                            if obj_idx >= 0 {
                                if let Some(obj_name) = object_names.get(obj_idx as usize) {
                                    if obj_name != self_name {
                                        if let Some(entry) = registry.lookup(obj_name) {
                                            refs.value_refs.insert(entry.short_name.clone());
                                        } else if external_imports.contains_key(obj_name.as_str()) {
                                            refs.ext_value_refs.insert(obj_name.to_string());
                                        }
                                    }
                                }
                            }
                        }
                        if method == "getOn" || method == "setOn" {
                            crate::rewrites::gamemaker::collect_gamemaker_instance_refs(
                                args,
                                &const_strings,
                                self_name,
                                self_ts_name,
                                registry,
                                external_imports,
                                refs,
                            );
                        }
                    }
                    EngineKind::GameMaker
                        if system == "GameMaker.Global"
                            && (method == "get" || method == "set")
                            && !args.is_empty() =>
                    {
                        if let Some(field_name) = args.first().and_then(|v| const_strings.get(v)) {
                            if global_names.contains(*field_name) {
                                refs.globals_used.insert(field_name.to_string());
                            }
                        }
                    }
                    _ => {}
                }
            }
            // GlobalRef to a known class (OBJT in GameMaker) → class constructor
            // import. ClassRef type means the value IS the class, not an instance.
            Op::GlobalRef(name) if registry.lookup(name).is_some() => {
                collect_named_type_ref(
                    name,
                    self_name,
                    self_ts_name,
                    registry,
                    external_imports,
                    &mut refs.value_refs,
                    &mut refs.ext_value_refs,
                );
            }
            // GlobalRef to a non-class name — no type reference to collect.
            Op::GlobalRef(_) => {}
            // Ops that don't contain type references requiring imports.
            // (Type info from these flows through value_types, handled below.)
            Op::Const(_)
            | Op::Cmp(..)
            | Op::Select { .. }
            | Op::Load(_)
            | Op::Store { .. }
            | Op::SetField { .. }
            | Op::GetIndex { .. }
            | Op::SetIndex { .. }
            | Op::Call { .. }
            | Op::MakeClosure { .. }
            | Op::CallIndirect { .. }
            | Op::MethodCall { .. }
            | Op::StructInit { .. }
            | Op::ArrayInit(_)
            | Op::TupleInit(_)
            | Op::Yield(_)
            | Op::CoroutineCreate { .. }
            | Op::CoroutineResume(_)
            | Op::Spread(_) => {}
        }
    }

    // value_types — type-only.
    for (_vid, ty) in func.value_types.iter() {
        collect_type_ref(
            ty,
            self_name,
            self_ts_name,
            module_types,
            registry,
            external_imports,
            &mut refs.type_refs,
            &mut refs.ext_type_refs,
        );
    }
}

/// Look up a named type (by string name) and add its import reference.
///
/// This is the core name-based lookup shared by `collect_type_ref` for all
/// named-type variants (`Instance`, `ClassRef`, and legacy `Struct`).
#[allow(clippy::too_many_arguments)]
fn collect_named_type_ref(
    name: &str,
    self_name: &str,
    self_ts_name: &str,
    registry: &ClassRegistry,
    external_imports: &BTreeMap<String, ExternalImport>,
    refs: &mut BTreeSet<String>,
    ext_refs: &mut BTreeSet<String>,
) {
    let short = name.rsplit("::").next().unwrap_or(name);
    if let Some(entry) = registry.lookup(name) {
        // Skip self-imports: compare ts_names so that two classes with
        // the same raw name (e.g. duplicate OBJT entries) are distinguished.
        if entry.short_name != self_ts_name {
            refs.insert(entry.short_name.clone());
        }
    } else if short != self_name {
        // Not in the intra-module registry — check external imports.
        if external_imports.contains_key(name) {
            ext_refs.insert(name.to_string());
        } else {
            warn_unmapped_reference(name);
        }
    }
}

/// If a type references a class in the registry, add its short name.
/// If not in the registry but in `external_imports`, add to `ext_refs`.
///
/// `self_name` is the raw GML class name (for short-name collision check).
/// `self_ts_name` is the disambiguated TypeScript identifier for the current
/// class — used to prevent false self-import matches when two classes share the
/// same raw name (the second class gets a `_2` suffix ts_name).
#[allow(clippy::too_many_arguments)]
pub(super) fn collect_type_ref(
    ty: &Type,
    self_name: &str,
    self_ts_name: &str,
    module_types: &PrimaryMap<TypeId, TypeDecl>,
    registry: &ClassRegistry,
    external_imports: &BTreeMap<String, ExternalImport>,
    refs: &mut BTreeSet<String>,
    ext_refs: &mut BTreeSet<String>,
) {
    match ty {
        // Instance(id) is the canonical IR form after normalize_struct_types.
        // Look up the name directly without constructing an intermediate Struct.
        Type::Instance(id) => {
            if let Some(named) = module_types.get(*id) {
                if let Some(name) = named.name() {
                    collect_named_type_ref(
                        name,
                        self_name,
                        self_ts_name,
                        registry,
                        external_imports,
                        refs,
                        ext_refs,
                    );
                }
            }
        }
        // ClassRef(id) — the class constructor. Look up the name directly.
        Type::ClassRef(id) => {
            if let Some(named) = module_types.get(*id) {
                if let Some(name) = named.name() {
                    collect_named_type_ref(
                        name,
                        self_name,
                        self_ts_name,
                        registry,
                        external_imports,
                        refs,
                        ext_refs,
                    );
                }
            }
        }
        Type::Array(inner) | Type::Option(inner) => {
            collect_type_ref(
                inner,
                self_name,
                self_ts_name,
                module_types,
                registry,
                external_imports,
                refs,
                ext_refs,
            );
        }
        Type::Map(k, v) => {
            // AS3 Dictionary (Map<Unknown, _>) → Flash-specific `Dictionary` class.
            // Register the import so flash_ts_type()'s "Dictionary" emission resolves.
            if matches!(k.as_ref(), Type::Unknown)
                && external_imports.contains_key("flash.utils::Dictionary")
            {
                ext_refs.insert("flash.utils::Dictionary".to_string());
            } else {
                collect_type_ref(
                    k,
                    self_name,
                    self_ts_name,
                    module_types,
                    registry,
                    external_imports,
                    refs,
                    ext_refs,
                );
                collect_type_ref(
                    v,
                    self_name,
                    self_ts_name,
                    module_types,
                    registry,
                    external_imports,
                    refs,
                    ext_refs,
                );
            }
        }
        Type::Tuple(elems) => {
            for elem in elems {
                collect_type_ref(
                    elem,
                    self_name,
                    self_ts_name,
                    module_types,
                    registry,
                    external_imports,
                    refs,
                    ext_refs,
                );
            }
        }
        Type::Function(sig) => {
            collect_type_ref(
                &sig.return_ty,
                self_name,
                self_ts_name,
                module_types,
                registry,
                external_imports,
                refs,
                ext_refs,
            );
            for p in &sig.params {
                collect_type_ref(
                    p,
                    self_name,
                    self_ts_name,
                    module_types,
                    registry,
                    external_imports,
                    refs,
                    ext_refs,
                );
            }
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => {
            collect_type_ref(
                yield_ty,
                self_name,
                self_ts_name,
                module_types,
                registry,
                external_imports,
                refs,
                ext_refs,
            );
            collect_type_ref(
                return_ty,
                self_name,
                self_ts_name,
                module_types,
                registry,
                external_imports,
                refs,
                ext_refs,
            );
        }
        Type::Union(types) => {
            for t in types {
                collect_type_ref(
                    t,
                    self_name,
                    self_ts_name,
                    module_types,
                    registry,
                    external_imports,
                    refs,
                    ext_refs,
                );
            }
        }
        // Primitive and leaf types — no type references to collect.
        Type::Void
        | Type::Bool
        | Type::Int(_)
        | Type::UInt(_)
        | Type::Float(_)
        | Type::String
        | Type::Var(_)
        | Type::Unknown => {}
    }
}

/// Collect short names of intra-module classes referenced by a type (for globals).
pub(super) fn collect_global_type_imports(
    ty: &Type,
    module_types: &PrimaryMap<TypeId, TypeDecl>,
    registry: &ClassRegistry,
    refs: &mut BTreeSet<String>,
) {
    match ty {
        Type::Instance(id) => {
            if let Some(named) = module_types.get(*id) {
                if let Some(name) = named.name() {
                    if let Some(entry) = registry.lookup(name) {
                        refs.insert(entry.short_name.clone());
                    }
                }
            }
        }
        Type::ClassRef(id) => {
            if let Some(named) = module_types.get(*id) {
                if let Some(name) = named.name() {
                    if let Some(entry) = registry.lookup(name) {
                        refs.insert(entry.short_name.clone());
                    }
                }
            }
        }
        Type::Array(inner) | Type::Option(inner) => {
            collect_global_type_imports(inner, module_types, registry, refs);
        }
        Type::Map(k, v) => {
            collect_global_type_imports(k, module_types, registry, refs);
            collect_global_type_imports(v, module_types, registry, refs);
        }
        Type::Tuple(elems) | Type::Union(elems) => {
            for elem in elems {
                collect_global_type_imports(elem, module_types, registry, refs);
            }
        }
        Type::Function(sig) => {
            collect_global_type_imports(&sig.return_ty, module_types, registry, refs);
            for p in &sig.params {
                collect_global_type_imports(p, module_types, registry, refs);
            }
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => {
            collect_global_type_imports(yield_ty, module_types, registry, refs);
            collect_global_type_imports(return_ty, module_types, registry, refs);
        }
        // Primitive and leaf types — no class imports to collect.
        Type::Void
        | Type::Bool
        | Type::Int(_)
        | Type::UInt(_)
        | Type::Float(_)
        | Type::String
        | Type::Var(_)
        | Type::Unknown => {}
    }
}

/// Collect ALL struct/enum short names referenced by a type, regardless of registry membership.
/// Used to detect runtime types (e.g. `GMLObject`) that are not in the emitted class registry.
pub(super) fn collect_all_struct_names(
    ty: &Type,
    module_types: &PrimaryMap<TypeId, TypeDecl>,
    refs: &mut BTreeSet<String>,
) {
    match ty {
        Type::Instance(id) | Type::ClassRef(id) => {
            if let Some(named) = module_types.get(*id) {
                if let Some(name) = named.name() {
                    let short = name.rsplit("::").next().unwrap_or(name);
                    refs.insert(short.to_string());
                }
            }
        }
        Type::Array(inner) | Type::Option(inner) => {
            collect_all_struct_names(inner, module_types, refs);
        }
        Type::Map(k, v) => {
            collect_all_struct_names(k, module_types, refs);
            collect_all_struct_names(v, module_types, refs);
        }
        Type::Tuple(elems) | Type::Union(elems) => {
            for elem in elems {
                collect_all_struct_names(elem, module_types, refs);
            }
        }
        Type::Function(sig) => {
            collect_all_struct_names(&sig.return_ty, module_types, refs);
            for p in &sig.params {
                collect_all_struct_names(p, module_types, refs);
            }
        }
        Type::Coroutine {
            yield_ty,
            return_ty,
        } => {
            collect_all_struct_names(yield_ty, module_types, refs);
            collect_all_struct_names(return_ty, module_types, refs);
        }
        // Primitive and leaf types — no struct/enum names to collect.
        Type::Void
        | Type::Bool
        | Type::Int(_)
        | Type::UInt(_)
        | Type::Float(_)
        | Type::String
        | Type::Var(_)
        | Type::Unknown => {}
    }
}

/// Emit grouped `import` / `import type` statements for external runtime references.
///
/// Names in `ext_value_refs` / `ext_type_refs` are qualified keys into
/// `external_imports` (e.g. `"flash.text::TextFormat"` or `"stage"`).
pub(super) fn emit_external_imports(
    ext_value_refs: &BTreeSet<String>,
    ext_type_refs: &BTreeSet<String>,
    external_imports: &BTreeMap<String, ExternalImport>,
    module_exports: &BTreeMap<String, Vec<String>>,
    prefix: &str,
    out: &mut String,
) {
    if ext_value_refs.is_empty() && ext_type_refs.is_empty() {
        return;
    }
    // Group value refs by module_path, resolving qualified → short names.
    // Validate each name against module_exports.
    let mut val_by_mod: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for name in ext_value_refs {
        if let Some(imp) = external_imports.get(name.as_str()) {
            validate_module_export(&imp.short_name, &imp.module_path, module_exports);
            val_by_mod
                .entry(&imp.module_path)
                .or_default()
                .push(&imp.short_name);
        }
    }
    for (module_path, names) in &val_by_mod {
        let _ = writeln!(
            out,
            "import {{ {} }} from \"{prefix}/runtime/{module_path}\";",
            names.join(", ")
        );
    }
    // Collect resolved short names from value refs for dedup.
    let val_short_names: HashSet<&str> = ext_value_refs
        .iter()
        .filter_map(|n| {
            external_imports
                .get(n.as_str())
                .map(|i| i.short_name.as_str())
        })
        .collect();
    // Type-only imports (not already covered by value imports).
    let mut type_by_mod: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for name in ext_type_refs {
        if let Some(imp) = external_imports.get(name.as_str()) {
            if !val_short_names.contains(imp.short_name.as_str()) {
                validate_module_export(&imp.short_name, &imp.module_path, module_exports);
                type_by_mod
                    .entry(&imp.module_path)
                    .or_default()
                    .push(&imp.short_name);
            }
        }
    }
    for (module_path, names) in &type_by_mod {
        let _ = writeln!(
            out,
            "import type {{ {} }} from \"{prefix}/runtime/{module_path}\";",
            names.join(", ")
        );
    }
}

/// Warn if a short name is not listed in `module_exports` for the given module path.
fn validate_module_export(
    short_name: &str,
    module_path: &str,
    module_exports: &BTreeMap<String, Vec<String>>,
) {
    if module_exports.is_empty() {
        return;
    }
    match module_exports.get(module_path) {
        Some(exports) => {
            if !exports.iter().any(|e| e == short_name) {
                eprintln!(
                    "warning: '{short_name}' is not exported from runtime module '{module_path}'"
                );
            }
        }
        None => {
            eprintln!(
                "warning: runtime module '{module_path}' has no declared exports \
                 (referenced: {short_name})"
            );
        }
    }
}

/// Compute the transitive closure of the value-import graph.
///
/// For each class, find all classes reachable through value imports (extends,
/// interfaces, static method owners, etc.). Used to detect cycles: if target T
/// transitively imports source S, then adding S→T would create a circular
/// dependency.
pub(super) fn compute_transitive_value_imports(
    direct_imports: &HashMap<String, BTreeSet<String>>,
) -> HashMap<String, HashSet<String>> {
    let mut result: HashMap<String, HashSet<String>> = HashMap::new();
    for start in direct_imports.keys() {
        let mut visited = HashSet::new();
        let mut stack = Vec::new();
        // Seed with direct imports of the start node.
        if let Some(direct) = direct_imports.get(start) {
            for dep in direct {
                if dep != start && visited.insert(dep.clone()) {
                    stack.push(dep.clone());
                }
            }
        }
        while let Some(current) = stack.pop() {
            if let Some(deps) = direct_imports.get(&current) {
                for dep in deps {
                    if dep != start && visited.insert(dep.clone()) {
                        stack.push(dep.clone());
                    }
                }
            }
        }
        result.insert(start.clone(), visited);
    }
    result
}

/// Emit `import` / `import type` statements for intra-module class references.
///
/// Returns the set of short names that are late-bound (circular TypeCheck/NullableCoerce
/// refs that should use `getDefinitionByName()` instead of a static import).
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_intra_imports(
    group: &ClassGroup,
    module: &Module,
    source_segments: &[String],
    registry: &ClassRegistry,
    static_method_owners: &HashMap<String, String>,
    static_field_owners: &HashMap<String, String>,
    global_names: &HashSet<String>,
    unique_static_field_map: &HashMap<String, String>,
    mutable_global_names: &HashSet<String>,
    module_exports: &BTreeMap<String, Vec<String>>,
    transitive_value_imports: &HashMap<String, HashSet<String>>,
    short_to_qualified: &HashMap<String, String>,
    depth: usize,
    engine: EngineKind,
    self_ts_name: &str,
    out: &mut String,
) -> HashSet<String> {
    let refs = collect_class_references(
        group,
        module,
        registry,
        &module.external_imports,
        static_method_owners,
        static_field_owners,
        global_names,
        unique_static_field_map,
        engine,
        self_ts_name,
    );

    // Compute late-bound set: typecheck refs whose targets transitively import
    // this class (i.e. adding a static import would create a cycle), and that
    // are NOT also referenced as regular value refs (e.g. `new X()` or `extends X`).
    // Use self_ts_name for cycle detection since transitive_value_imports is keyed
    // by disambiguated TypeScript identifier.
    let mut late_bound: HashSet<String> = HashSet::new();
    for name in &refs.typecheck_value_refs {
        if !refs.value_refs.contains(name) {
            // Target T transitively imports self → adding self→T would be circular.
            let target_imports_self = transitive_value_imports
                .get(name.as_str())
                .is_some_and(|reachable| reachable.contains(self_ts_name));
            if target_imports_self {
                // Only late-bind if we have a qualified name to resolve at runtime.
                if short_to_qualified.contains_key(name.as_str()) {
                    late_bound.insert(name.clone());
                }
            }
        }
    }

    // Merge non-late-bound typecheck refs back into value_refs for import generation.
    let mut effective_value_refs = refs.value_refs.clone();
    for name in &refs.typecheck_value_refs {
        if !late_bound.contains(name) {
            effective_value_refs.insert(name.clone());
        }
    }

    let has_intra = !effective_value_refs.is_empty() || !refs.type_refs.is_empty();
    let has_ext = !refs.ext_value_refs.is_empty() || !refs.ext_type_refs.is_empty();
    let has_globals = !refs.globals_used.is_empty();
    if !has_intra && !has_ext && !has_globals {
        return late_bound;
    }

    // External runtime imports — grouped by sub-module.
    if has_ext {
        let prefix = "../".repeat(depth + 1);
        let prefix = prefix.trim_end_matches('/');
        emit_external_imports(
            &refs.ext_value_refs,
            &refs.ext_type_refs,
            &module.external_imports,
            module_exports,
            prefix,
            out,
        );
    }

    // Intra-module value imports.
    for short_name in &effective_value_refs {
        if let Some(entry) = registry.classes.get(short_name) {
            let rel = relative_import_path(source_segments, &entry.path_segments);
            let _ = writeln!(out, "import {{ {short_name} }} from \"{rel}\";");
        }
    }
    // Intra-module type-only imports (names not already in value_refs).
    for short_name in &refs.type_refs {
        if effective_value_refs.contains(short_name) {
            continue;
        }
        if let Some(entry) = registry.classes.get(short_name) {
            let rel = relative_import_path(source_segments, &entry.path_segments);
            let _ = writeln!(out, "import type {{ {short_name} }} from \"{rel}\";");
        }
    }
    // Module-level globals.
    if has_globals {
        let globals_path = if depth == 0 {
            "./_globals".to_string()
        } else {
            let prefix = "../".repeat(depth);
            format!("{}_globals", prefix)
        };
        let mut import_names: Vec<String> = Vec::new();
        for name in &refs.globals_used {
            import_names.push(sanitize_ident(name));
            // Also import the setter for mutable globals.
            if mutable_global_names.contains(name.as_str()) {
                import_names.push(format!("$set_{}", sanitize_ident(name)));
            }
        }
        let _ = writeln!(
            out,
            "import {{ {} }} from \"{globals_path}\";",
            import_names.join(", ")
        );
    }
    out.push('\n');
    late_bound
}

/// Emit IR-level imports (Module.imports).
pub(super) fn emit_imports(module: &Module, out: &mut String) {
    for import in &module.imports {
        let name = match &import.alias {
            Some(alias) => {
                format!(
                    "{} as {}",
                    sanitize_ident(&import.name),
                    sanitize_ident(alias)
                )
            }
            None => sanitize_ident(&import.name),
        };
        let _ = writeln!(out, "import {{ {name} }} from \"./{}\";", import.module);
    }
    if !module.imports.is_empty() {
        out.push('\n');
    }
}
