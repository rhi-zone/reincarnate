mod class;
mod imports;
mod rewrites;
mod sanitize;
mod scaffold;
#[cfg(test)]
mod tests;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write;
use std::fs;
use std::path::Path;

use reincarnate_core::error::CoreError;
use reincarnate_core::ir::{ClassDef, FuncId, MethodKind, Module, StructDef};
use reincarnate_core::pipeline::{DebugConfig, Diagnostic, LoweringConfig};
use reincarnate_core::project::{ExternalMethodSig, ExternalTypeDef, RuntimeConfig};

use crate::runtime::SYSTEM_NAMES;
use crate::types::ts_type;

// Re-export public items from sub-modules.
pub(crate) use class::ClassGroup;
pub(crate) use sanitize::sanitize_ident;

// Use items from sub-modules within this module.
use class::{compile_closures, emit_class, emit_function, emit_functions, group_by_class};
use imports::{
    collect_all_struct_names, collect_call_names_from_funcs, collect_class_references,
    collect_global_type_imports, collect_system_names_from_funcs, collect_type_refs_from_function,
    compute_transitive_value_imports, emit_external_imports, emit_free_function_imports,
    emit_function_imports_with_prefix, emit_imports, emit_intra_imports, emit_runtime_imports,
    emit_runtime_imports_for, relative_import_path, strip_unused_namespace_imports,
};
use rewrites::visibility_prefix;
use sanitize::{rename_colliding_free_funcs, rename_shadowing_locals};
use scaffold::{validate_member_accesses, ClassMeta};

/// Which engine's rewrite pass to use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EngineKind {
    Flash,
    GameMaker,
    Twine,
}

/// Detect engine from runtime config system_modules keys.
fn detect_engine(runtime_config: Option<&RuntimeConfig>) -> EngineKind {
    if let Some(cfg) = runtime_config {
        if cfg
            .system_modules
            .keys()
            .any(|k| k.starts_with("SugarCube.") || k.starts_with("Harlowe."))
        {
            return EngineKind::Twine;
        }
        if cfg
            .system_modules
            .keys()
            .any(|k| k.starts_with("GameMaker."))
        {
            return EngineKind::GameMaker;
        }
    }
    EngineKind::Flash
}

/// Build a `LoweringConfig` augmented with engine-specific settings.
///
/// - Flash: sets `scope_lookup_systems = ["Flash.Scope"]` so that scope-lookup
///   SystemCalls are always-inlined for chain resolution in flash.rs.
///   Also enables `foreach_rewrite` (Flash's HasNext2 for-of pattern).
/// - GML: sets `wrap_class_refs_as_any = true` so that `ClassRef`-typed GlobalRef
///   values get `as any` at each use site (GML object names double as integer indices).
fn lowering_config_for_engine(
    config: &LoweringConfig,
    engine: EngineKind,
) -> std::borrow::Cow<'_, LoweringConfig> {
    let needs_flash = engine == EngineKind::Flash
        && (config.scope_lookup_systems.is_empty()
            || !config.foreach_rewrite
            || !config.construct_string_coerce
            || !config.coerce_index_types);
    let needs_gml = engine == EngineKind::GameMaker && !config.wrap_class_refs_as_any;
    if needs_flash || needs_gml {
        let mut c = config.clone();
        if needs_flash {
            c.scope_lookup_systems = vec!["Flash.Scope".to_string()];
            c.foreach_rewrite = true;
            c.construct_string_coerce = true;
            c.coerce_index_types = true;
        }
        if needs_gml {
            c.wrap_class_refs_as_any = true;
        }
        std::borrow::Cow::Owned(c)
    } else {
        std::borrow::Cow::Borrowed(config)
    }
}

/// Emit a single module into `output_dir`.
///
/// If the module has classes, emits a directory with one file per class plus
/// a barrel `index.ts`. Otherwise emits a flat `.ts` file.
pub fn emit_module(
    module: &mut Module,
    output_dir: &Path,
    lowering_config: &LoweringConfig,
    runtime_config: Option<&RuntimeConfig>,
    debug: &DebugConfig,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), CoreError> {
    if module.classes.is_empty() {
        let out =
            emit_module_to_string(module, lowering_config, runtime_config, debug, diagnostics)?;
        let path = output_dir.join(format!("{}.ts", module.name));
        fs::write(&path, &out).map_err(CoreError::Io)?;
    } else {
        emit_module_to_dir(
            module,
            output_dir,
            lowering_config,
            runtime_config,
            debug,
            diagnostics,
        )?;
    }
    Ok(())
}

/// Emit a module to a string (flat output — for testing or class-free modules).
pub fn emit_module_to_string(
    module: &mut Module,
    lowering_config: &LoweringConfig,
    runtime_config: Option<&RuntimeConfig>,
    debug: &DebugConfig,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<String, CoreError> {
    let mut out = String::new();
    let class_names = build_class_names(module);
    let empty_type_defs = BTreeMap::new();
    let type_defs = runtime_config
        .map(|c| &c.type_definitions)
        .unwrap_or(&empty_type_defs);
    let class_meta = ClassMeta::build(module, type_defs);
    let mut known_classes: HashSet<String> = class_names.values().cloned().collect();
    if let Some(rc) = runtime_config {
        known_classes.extend(rc.type_definitions.keys().cloned());
    }

    let engine = detect_engine(runtime_config);
    let mut stateful_system_aliases = BTreeMap::new();
    emit_runtime_imports(
        module,
        &mut out,
        runtime_config,
        engine,
        &mut stateful_system_aliases,
    );
    // If any system modules are stateful, import the runtime type for `_rt` parameter.
    if !stateful_system_aliases.is_empty() {
        if let Some(rt_type) = runtime_config.and_then(|c| c.runtime_type.as_ref()) {
            let _ = writeln!(
                out,
                "import type {{ {} }} from \"./runtime/{}\";",
                rt_type.name, rt_type.path,
            );
            // Also import the context type (e.g. `HarloweContext`) for the `h` parameter.
            if let Some(ctx_type) = runtime_config.and_then(|c| c.context_type.as_ref()) {
                let _ = writeln!(
                    out,
                    "import type {{ {} }} from \"./runtime/{}\";",
                    ctx_type.name, ctx_type.path,
                );
            }
            out.push('\n');
        }
    }
    if let Some(preamble) = runtime_config.and_then(|c| c.class_preamble.as_ref()) {
        let _ = writeln!(
            out,
            "import {{ {} }} from \"./runtime/{}\";",
            preamble.names.join(", "),
            preamble.path,
        );
        out.push('\n');
    }
    emit_imports(module, &mut out);
    emit_structs(module, &mut out);
    emit_enums(module, &mut out);
    emit_globals(module, &mut out);

    // Single-file mode — globals are in the same scope, no ESM setter rewrite needed.
    let no_mutable_globals = HashSet::new();
    if module.classes.is_empty() {
        emit_functions(
            module,
            &class_names,
            &known_classes,
            &no_mutable_globals,
            lowering_config,
            engine,
            &stateful_system_aliases,
            runtime_config,
            &class_meta.unique_static_field_map,
            debug,
            &mut out,
            diagnostics,
        )?;
    } else {
        // Single-file mode: no circular imports (all classes in one scope).
        let no_late_bound = HashSet::new();
        let no_short_to_qualified = HashMap::new();
        let (class_groups, free_funcs) = group_by_class(module);
        let no_stateful = BTreeSet::new();
        let no_free_fns = HashSet::new();
        let no_sys_aliases = BTreeMap::new();
        let empty_func_sigs = BTreeMap::new();
        let func_sigs = runtime_config
            .map(|c| &c.function_signatures)
            .unwrap_or(&empty_func_sigs);
        for group in &class_groups {
            let mut traits_buf = String::new();
            emit_class(
                group,
                module,
                &class_names,
                &class_meta,
                &no_mutable_globals,
                &no_late_bound,
                &no_short_to_qualified,
                &known_classes,
                lowering_config,
                engine,
                &no_stateful,
                &no_free_fns,
                func_sigs,
                debug,
                &mut out,
                &mut traits_buf,
                diagnostics,
            )?;
            // In string mode (tests), append traits inline.
            out.push_str(&traits_buf);
        }
        let closure_fids: Vec<FuncId> = free_funcs
            .iter()
            .copied()
            .filter(|&fid| module.functions[fid].method_kind == MethodKind::Closure)
            .collect();
        let closure_bodies =
            compile_closures(&closure_fids, module, lowering_config, engine, debug);
        for &fid in &free_funcs {
            if module.functions[fid].method_kind != MethodKind::Closure {
                emit_function(
                    &mut module.functions[fid],
                    &class_names,
                    &known_classes,
                    &no_mutable_globals,
                    lowering_config,
                    engine,
                    &module.sprite_names,
                    &module.object_names,
                    &closure_bodies,
                    &no_stateful,
                    &no_free_fns,
                    &no_sys_aliases,
                    runtime_config,
                    &class_meta.unique_static_field_map,
                    debug,
                    &mut out,
                    diagnostics,
                )?;
            }
        }
    }

    strip_unused_namespace_imports(&mut out);
    Ok(out)
}

// ---------------------------------------------------------------------------
// ClassRegistry — maps qualified names to filesystem paths for imports
// ---------------------------------------------------------------------------

pub(crate) struct ClassEntry {
    pub(crate) short_name: String,
    /// Path segments from module root, e.g. ["classes", "Scenes", "Swamp", "Swamp"].
    pub(crate) path_segments: Vec<String>,
}

pub(crate) struct ClassRegistry {
    /// Keyed by both qualified name and bare name (fallback).
    pub(crate) classes: HashMap<String, ClassEntry>,
}

impl ClassRegistry {
    /// Build the registry from a module.
    ///
    /// `class_names` is the output of [`build_class_names`] — it maps qualified
    /// class names to their (possibly disambiguated) TypeScript identifiers.
    fn from_module(module: &Module, class_names: &HashMap<String, String>) -> Self {
        let mut classes = HashMap::new();
        for class in &module.classes {
            let base_short = sanitize_ident(&class.name);
            let qualified = qualified_class_name(class);
            // Use the disambiguated TypeScript identifier as the short_name.
            let ts_name = class_names
                .get(&qualified)
                .cloned()
                .unwrap_or_else(|| base_short.clone());

            // Path segments use the BASE file-system name (not the TS identifier),
            // since the file on disk is still named after the original short name.
            let mut segments: Vec<String> =
                class.namespace.iter().map(|s| sanitize_ident(s)).collect();
            segments.push(base_short.clone());

            // Key by qualified name: "classes.Scenes.Areas.Swamp::CorruptedDriderScene"
            classes.insert(
                qualified,
                ClassEntry {
                    short_name: ts_name.clone(),
                    path_segments: segments.clone(),
                },
            );
            // Key by the TypeScript identifier for import-generation lookups.
            // `or_insert` ensures the first class wins when two classes share the
            // same ts_name (shouldn't happen after disambiguation, but safe).
            classes.entry(ts_name.clone()).or_insert(ClassEntry {
                short_name: ts_name,
                path_segments: segments,
            });
        }
        Self { classes }
    }

    pub(crate) fn lookup(&self, name: &str) -> Option<&ClassEntry> {
        self.classes
            .get(name)
            .or_else(|| {
                // Try extracting the short name after `::`
                let short = name.rsplit("::").next()?;
                self.classes.get(short)
            })
            .or_else(|| {
                // Try sanitized form — raw GML names like "6parent" need the
                // underscore prefix added by sanitize_ident → "_6parent".
                let sanitized = sanitize_ident(name);
                if sanitized != name {
                    self.classes.get(&sanitized)
                } else {
                    None
                }
            })
    }
}

/// Build a map from qualified class names to TypeScript class identifiers.
///
/// When two classes share the same sanitized short name (e.g., two classes both
/// named `GooArmor` in different AS3 namespaces), both are disambiguated by
/// prepending the last namespace segment: `Armors_GooArmor` and `NPCs_GooArmor`.
/// Unique class names are kept as-is.
pub(crate) fn build_class_names(module: &Module) -> HashMap<String, String> {
    // Count occurrences of each sanitized short name.
    let mut counts: HashMap<String, usize> = HashMap::new();
    for class in &module.classes {
        *counts.entry(sanitize_ident(&class.name)).or_insert(0) += 1;
    }
    module
        .classes
        .iter()
        .map(|c| {
            let short = sanitize_ident(&c.name);
            let ts_name = if counts.get(&short).copied().unwrap_or(0) > 1 {
                // Disambiguate: prepend the last namespace segment.
                let ns_last = c
                    .namespace
                    .last()
                    .map(|s| sanitize_ident(s))
                    .unwrap_or_default();
                format!("{ns_last}_{short}")
            } else {
                short
            };
            (qualified_class_name(c), ts_name)
        })
        .collect()
}

pub(crate) fn qualified_class_name(class: &ClassDef) -> String {
    if class.namespace.is_empty() {
        class.name.clone()
    } else {
        format!("{}::{}", class.namespace.join("."), class.name)
    }
}

/// Bundled output sets for collecting class/type references.
#[derive(Default)]
pub(crate) struct RefSets {
    /// Intra-module value refs (class constructor needed at runtime).
    pub(crate) value_refs: BTreeSet<String>,
    /// Intra-module type-only refs (erased at runtime).
    pub(crate) type_refs: BTreeSet<String>,
    /// Intra-module value refs from TypeCheck/NullableCoerce only (may be late-bound).
    pub(crate) typecheck_value_refs: BTreeSet<String>,
    /// External value refs (e.g. Flash stdlib runtime classes).
    pub(crate) ext_value_refs: BTreeSet<String>,
    /// External type-only refs.
    pub(crate) ext_type_refs: BTreeSet<String>,
    /// Module-level globals referenced via scope lookups.
    pub(crate) globals_used: BTreeSet<String>,
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
    for field in &def.fields {
        let _ = writeln!(
            out,
            "  {}: {};",
            sanitize_ident(&field.name),
            ts_type(&field.ty)
        );
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
        // const without initializer is invalid JS — demote to let.
        let kw = if global.mutable || global.init.is_none() {
            "let"
        } else {
            "const"
        };
        let ident = sanitize_ident(&global.name);
        let ts = ts_type(&global.ty);
        if let Some(val) = &global.init {
            let _ = writeln!(
                out,
                "{vis}{kw} {ident}: {ts} = {};",
                crate::ast_printer::emit_constant(val)
            );
        } else {
            let _ = writeln!(out, "{vis}{kw} {ident}: {ts};");
        }
    }
    if !module.globals.is_empty() {
        out.push('\n');
    }
}

// ---------------------------------------------------------------------------
// Module directory emission
// ---------------------------------------------------------------------------

/// Emit `_globals.ts` — module-level global variable declarations.
/// Returns `true` if the file was written (so the caller can add a barrel export).
fn emit_globals_file(
    module: &Module,
    module_dir: &Path,
    registry: &ClassRegistry,
    runtime_config: Option<&RuntimeConfig>,
) -> Result<bool, CoreError> {
    if module.globals.is_empty() {
        return Ok(false);
    }
    let mut out = String::new();

    // Collect type imports for Struct-typed globals.
    let mut type_imports: BTreeSet<String> = BTreeSet::new();
    // All struct/enum names used in globals (includes runtime types not in registry).
    let mut all_struct_names: BTreeSet<String> = BTreeSet::new();
    for global in &module.globals {
        collect_global_type_imports(&global.ty, registry, &mut type_imports);
        collect_all_struct_names(&global.ty, &mut all_struct_names);
    }
    let mut any_import = false;
    // Import runtime/preamble types (e.g. GMLObject) that are used as global types
    // but are not emitted classes. _globals.ts is one level below the output root.
    if let Some(preamble) = runtime_config.and_then(|c| c.class_preamble.as_ref()) {
        let preamble_needed: Vec<&str> = preamble
            .names
            .iter()
            .filter(|n| all_struct_names.contains(n.as_str()))
            .map(|n| n.as_str())
            .collect();
        if !preamble_needed.is_empty() {
            let _ = writeln!(
                out,
                "import {{ {} }} from \"../runtime/{}\";",
                preamble_needed.join(", "),
                preamble.path,
            );
            any_import = true;
        }
    }
    for short_name in &type_imports {
        if let Some(entry) = registry.classes.get(short_name) {
            let rel = format!("./{}", entry.path_segments.join("/"));
            let _ = writeln!(out, "import type {{ {short_name} }} from \"{rel}\";");
            any_import = true;
        }
    }
    if any_import {
        out.push('\n');
    }

    for global in &module.globals {
        // const without initializer is invalid JS — demote to let.
        let kw = if global.mutable || global.init.is_none() {
            "let"
        } else {
            "const"
        };
        let ident = sanitize_ident(&global.name);
        let ts = ts_type(&global.ty);
        if let Some(val) = &global.init {
            let _ = writeln!(
                out,
                "export {kw} {ident}: {ts} = {};",
                crate::ast_printer::emit_constant(val)
            );
        } else {
            let _ = writeln!(out, "export {kw} {ident}: {ts};");
        }
        // ESM setter for mutable globals — imports are read-only bindings.
        if global.mutable {
            let _ = writeln!(
                out,
                "export function $set_{ident}(v: {ts}) {{ {ident} = v; }}"
            );
        }
    }
    let path = module_dir.join("_globals.ts");
    fs::write(&path, &out).map_err(CoreError::Io)?;
    Ok(true)
}

/// Pre-collect all classes' direct value imports for cycle detection, then
/// compute the transitive closure.
fn collect_transitive_imports(
    class_groups: &[ClassGroup],
    module: &Module,
    registry: &ClassRegistry,
    class_meta: &ClassMeta,
    class_names: &HashMap<String, String>,
    global_names: &HashSet<String>,
    engine: EngineKind,
) -> HashMap<String, HashSet<String>> {
    let mut direct_value_imports: HashMap<String, BTreeSet<String>> = HashMap::new();
    for group in class_groups {
        let qualified = qualified_class_name(&group.class_def);
        let empty_smo = HashMap::new();
        let smo = class_meta
            .static_method_owner_map
            .get(&qualified)
            .unwrap_or(&empty_smo);
        let sfo = class_meta
            .static_field_owner_map
            .get(&qualified)
            .unwrap_or(&empty_smo);
        let refs = collect_class_references(
            group,
            module,
            registry,
            &module.external_imports,
            smo,
            sfo,
            global_names,
            &class_meta.unique_static_field_map,
            engine,
        );
        let qualified = qualified_class_name(&group.class_def);
        let ts_name = class_names
            .get(&qualified)
            .cloned()
            .unwrap_or_else(|| sanitize_ident(&group.class_def.name));
        direct_value_imports.insert(ts_name, refs.value_refs);
    }
    compute_transitive_value_imports(&direct_value_imports)
}

/// Emit a single class file (and optional companion `_traits.ts` file).
/// Returns the barrel export paths added by this class.
#[allow(clippy::too_many_arguments)]
fn emit_class_file(
    group: &ClassGroup,
    module: &mut Module,
    module_dir: &Path,
    class_names: &HashMap<String, String>,
    class_meta: &ClassMeta,
    registry: &ClassRegistry,
    global_names: &HashSet<String>,
    mutable_global_names: &HashSet<String>,
    free_func_names: &HashSet<String>,
    known_classes: &HashSet<String>,
    short_to_qualified: &HashMap<String, String>,
    module_exports: &BTreeMap<String, Vec<String>>,
    transitive_value_imports: &HashMap<String, HashSet<String>>,
    type_defs: &BTreeMap<String, ExternalTypeDef>,
    func_sigs: &BTreeMap<String, ExternalMethodSig>,
    lowering_config: &LoweringConfig,
    runtime_config: Option<&RuntimeConfig>,
    engine: EngineKind,
    debug: &DebugConfig,
    barrel_exports: &mut Vec<String>,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), CoreError> {
    let class_def = &group.class_def;
    let short_name = sanitize_ident(&class_def.name);

    // Path segments for this class: namespace segments + class name.
    let mut segments: Vec<String> = class_def
        .namespace
        .iter()
        .map(|s| sanitize_ident(s))
        .collect();
    segments.push(short_name.clone());

    // Depth = number of namespace segments (directories below module_dir).
    let depth = class_def.namespace.len();

    // Create nested directory.
    let mut file_dir = module_dir.to_path_buf();
    for seg in &class_def.namespace {
        file_dir = file_dir.join(sanitize_ident(seg));
    }
    fs::create_dir_all(&file_dir).map_err(CoreError::Io)?;

    let mut out = String::new();
    let class_funcs = || group.methods.iter().map(|&fid| &module.functions[fid]);
    let all_systems = collect_system_names_from_funcs(class_funcs());
    // For Flash class files, generic shims and Flash.Memory are accessed via
    // `this._shims` — strip them from the import set (no module-level singleton
    // import needed).
    let known_generics: BTreeSet<&str> = SYSTEM_NAMES.iter().copied().collect();
    let systems = if engine == EngineKind::Flash {
        all_systems
            .into_iter()
            .filter(|s| !known_generics.contains(s.as_str()) && s != "Flash.Memory")
            .collect()
    } else {
        all_systems
    };
    let mut _class_sys_aliases = BTreeMap::new();
    emit_runtime_imports_for(
        systems,
        &mut out,
        depth,
        runtime_config,
        &mut _class_sys_aliases,
    );
    // Flash class files need the FlashShims type for constructor parameter annotations.
    if engine == EngineKind::Flash && !group.class_def.is_interface {
        let pref = "../".repeat(depth + 1);
        let pref = pref.trim_end_matches('/');
        let _ = writeln!(out, "import type {{ FlashShims }} from \"{pref}/runtime\";");
        out.push('\n');
    }
    let calls = collect_call_names_from_funcs(class_funcs(), engine);
    let func_prefix = "../".repeat(depth + 1);
    let func_prefix = func_prefix.trim_end_matches('/');
    let mut stateful_names = BTreeSet::new();
    emit_function_imports_with_prefix(
        &calls,
        &mut out,
        func_prefix,
        runtime_config,
        &mut stateful_names,
    );
    // Game-defined scripts shadow runtime functions with the same name.
    stateful_names.retain(|name| !free_func_names.contains(name.as_str()));
    emit_free_function_imports(&calls, free_func_names, depth, &mut out);
    if let Some(preamble) = runtime_config.and_then(|c| c.class_preamble.as_ref()) {
        let prefix = "../".repeat(depth + 1);
        let prefix = prefix.trim_end_matches('/');
        let _ = writeln!(
            out,
            "import {{ {} }} from \"{prefix}/runtime/{}\";",
            preamble.names.join(", "),
            preamble.path,
        );
        out.push('\n');
    }
    let qualified = qualified_class_name(&group.class_def);
    let empty_smo = HashMap::new();
    let static_method_owners = class_meta
        .static_method_owner_map
        .get(&qualified)
        .unwrap_or(&empty_smo);
    let static_field_owners = class_meta
        .static_field_owner_map
        .get(&qualified)
        .unwrap_or(&empty_smo);
    let late_bound = emit_intra_imports(
        group,
        module,
        &segments,
        registry,
        static_method_owners,
        static_field_owners,
        global_names,
        &class_meta.unique_static_field_map,
        mutable_global_names,
        module_exports,
        transitive_value_imports,
        short_to_qualified,
        depth,
        engine,
        &mut out,
    );

    // Always emit the Sprites import; strip_unused_namespace_imports removes
    // it when no `Sprites.` reference appears in the output.
    if !module.sprite_names.is_empty() {
        let prefix = "../".repeat(depth + 1);
        let prefix = prefix.trim_end_matches('/');
        let _ = writeln!(out, "import {{ Sprites }} from \"{prefix}/data/sprites\";");
    }

    // Validate member accesses before emitting (warnings only).
    for &fid in &group.methods {
        validate_member_accesses(
            &module.functions[fid],
            Some(&qualified),
            class_meta,
            registry,
            short_to_qualified,
            type_defs,
        );
    }

    let mut traits_buf = String::new();
    emit_class(
        group,
        module,
        class_names,
        class_meta,
        mutable_global_names,
        &late_bound,
        short_to_qualified,
        known_classes,
        lowering_config,
        engine,
        &stateful_names,
        free_func_names,
        func_sigs,
        debug,
        &mut out,
        &mut traits_buf,
        diagnostics,
    )?;

    strip_unused_namespace_imports(&mut out);
    let path = file_dir.join(format!("{short_name}.ts"));
    fs::write(&path, &out).map_err(CoreError::Io)?;

    // Write companion _traits.ts file for Flash registration calls.
    if !traits_buf.is_empty() {
        write_traits_file(
            group,
            &traits_buf,
            &segments,
            &short_name,
            class_names,
            registry,
            module,
            runtime_config,
            depth,
            &file_dir,
        )?;
    }

    // Barrel export path: relative from module_dir.
    let export_path = segments.join("/");
    barrel_exports.push(export_path);
    if !traits_buf.is_empty() {
        let mut traits_segments = segments.clone();
        if let Some(last) = traits_segments.last_mut() {
            *last = format!("{last}_traits");
        }
        barrel_exports.push(traits_segments.join("/"));
    }
    Ok(())
}

/// Write the companion `_traits.ts` file for Flash class registration calls.
#[allow(clippy::too_many_arguments)]
fn write_traits_file(
    group: &ClassGroup,
    traits_buf: &str,
    segments: &[String],
    short_name: &str,
    class_names: &HashMap<String, String>,
    registry: &ClassRegistry,
    module: &Module,
    runtime_config: Option<&RuntimeConfig>,
    depth: usize,
    file_dir: &Path,
) -> Result<(), CoreError> {
    let prefix = "../".repeat(depth + 1);
    let prefix = prefix.trim_end_matches('/');
    let mut traits_file = String::new();
    // Import only the registration functions actually used.
    let mut reg_names = Vec::new();
    if traits_buf.contains("registerClass(") {
        reg_names.push("registerClass");
    }
    if traits_buf.contains("registerClassTraits(") {
        reg_names.push("registerClassTraits");
    }
    if traits_buf.contains("registerInterface(") {
        reg_names.push("registerInterface");
    }
    if let Some(preamble) = runtime_config.and_then(|c| c.class_preamble.as_ref()) {
        let _ = writeln!(
            traits_file,
            "import {{ {} }} from \"{prefix}/runtime/{}\";",
            reg_names.join(", "),
            preamble.path,
        );
    }
    // Import the class itself (use disambiguated TS identifier, not filesystem name).
    let qualified = qualified_class_name(&group.class_def);
    let ts_name = class_names
        .get(&qualified)
        .cloned()
        .unwrap_or_else(|| short_name.to_string());
    let _ = writeln!(
        traits_file,
        "import {{ {ts_name} }} from \"./{short_name}\";"
    );
    // Import interface classes referenced by registerInterface.
    let mut traits_segments = segments.to_vec();
    if let Some(last) = traits_segments.last_mut() {
        *last = format!("{last}_traits");
    }
    for iface_qualified in &group.class_def.interfaces {
        let iface_ts = class_names
            .get(iface_qualified.as_str())
            .cloned()
            .unwrap_or_else(|| {
                let short = iface_qualified
                    .rsplit("::")
                    .next()
                    .unwrap_or(iface_qualified);
                sanitize_ident(short)
            });
        if let Some(entry) = registry.lookup(iface_qualified) {
            // In-module interface — relative path.
            let rel = relative_import_path(&traits_segments, &entry.path_segments);
            let _ = writeln!(traits_file, "import {{ {iface_ts} }} from \"{rel}\";");
        } else if let Some(ext) = module.external_imports.get(iface_qualified) {
            // External runtime interface — import from runtime path.
            let _ = writeln!(
                traits_file,
                "import {{ {} }} from \"{prefix}/runtime/{}\";",
                ext.short_name, ext.module_path,
            );
        }
    }
    traits_file.push('\n');
    traits_file.push_str(traits_buf);
    let traits_path = file_dir.join(format!("{short_name}_traits.ts"));
    fs::write(&traits_path, &traits_file).map_err(CoreError::Io)?;
    Ok(())
}

/// Emit `_init.ts` — free (non-class) function definitions.
/// Returns `true` if the file was written (so the caller can add a barrel export).
#[allow(clippy::too_many_arguments)]
fn emit_free_functions_file(
    module: &mut Module,
    module_dir: &Path,
    free_funcs: &[FuncId],
    free_func_names: &HashSet<String>,
    class_names: &HashMap<String, String>,
    class_meta: &ClassMeta,
    registry: &ClassRegistry,
    global_names: &HashSet<String>,
    mutable_global_names: &HashSet<String>,
    known_classes: &HashSet<String>,
    module_exports: &BTreeMap<String, Vec<String>>,
    lowering_config: &LoweringConfig,
    runtime_config: Option<&RuntimeConfig>,
    engine: EngineKind,
    debug: &DebugConfig,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<bool, CoreError> {
    if free_funcs.is_empty() {
        return Ok(false);
    }
    let mut out = String::new();
    let free_fn_iter = || free_funcs.iter().map(|&fid| &module.functions[fid]);
    let all_free_systems = collect_system_names_from_funcs(free_fn_iter());
    // Flash.Memory is per-instance (accessed via _shims) — no module-level import.
    let systems = if engine == EngineKind::Flash {
        all_free_systems
            .into_iter()
            .filter(|s| s != "Flash.Memory")
            .collect()
    } else {
        all_free_systems
    };
    let mut _free_sys_aliases = BTreeMap::new();
    emit_runtime_imports_for(systems, &mut out, 0, runtime_config, &mut _free_sys_aliases);
    let calls = collect_call_names_from_funcs(free_fn_iter(), engine);
    let mut free_stateful_names = BTreeSet::new();
    emit_function_imports_with_prefix(
        &calls,
        &mut out,
        "..",
        runtime_config,
        &mut free_stateful_names,
    );
    // Game-defined free functions shadow runtime functions with the same name.
    // Remove any runtime stateful entry that the game overrides — inside each
    // function body the `const { name } = _rt` destructuring would shadow the
    // game's version and break call sites that pass `(_rt, self, ...)`.
    free_stateful_names.retain(|name| !free_func_names.contains(name.as_str()));
    // If free functions use stateful runtime functions, import the runtime type
    // for the `_rt` parameter annotation.
    if !free_stateful_names.is_empty() {
        if let Some(preamble_cfg) = runtime_config.and_then(|c| c.class_preamble.as_ref()) {
            let _ = writeln!(
                out,
                "import type {{ GameRuntime }} from \"../runtime/{}\";",
                preamble_cfg.path
            );
        }
    }
    if let Some(preamble) = runtime_config.and_then(|c| c.class_preamble.as_ref()) {
        let prefix = "../";
        let prefix = prefix.trim_end_matches('/');
        let _ = writeln!(
            out,
            "import {{ {} }} from \"{prefix}/runtime/{}\";",
            preamble.names.join(", "),
            preamble.path,
        );
        out.push('\n');
    }

    // Scan free functions for external class references.
    let mut refs = RefSets::default();
    for &fid in free_funcs {
        let func = &module.functions[fid];
        collect_type_refs_from_function(
            func,
            "",
            registry,
            &module.external_imports,
            &HashMap::new(),
            &HashMap::new(),
            global_names,
            &HashMap::new(),
            &module.object_names,
            engine,
            &mut refs,
        );
    }
    emit_external_imports(
        &refs.ext_value_refs,
        &refs.ext_type_refs,
        &module.external_imports,
        module_exports,
        "..",
        &mut out,
    );

    // Intra-module class imports for free functions.
    let init_segments = vec!["_init".to_string()];
    for short_name in &refs.value_refs {
        if let Some(entry) = registry.classes.get(short_name) {
            let rel = relative_import_path(&init_segments, &entry.path_segments);
            let _ = writeln!(out, "import {{ {short_name} }} from \"{rel}\";");
        }
    }
    for short_name in &refs.type_refs {
        if refs.value_refs.contains(short_name) {
            continue;
        }
        if let Some(entry) = registry.classes.get(short_name) {
            let rel = relative_import_path(&init_segments, &entry.path_segments);
            let _ = writeln!(out, "import type {{ {short_name} }} from \"{rel}\";");
        }
    }

    // Globals imports for free functions.
    if !refs.globals_used.is_empty() {
        let mut import_names: Vec<String> = Vec::new();
        for name in &refs.globals_used {
            import_names.push(sanitize_ident(name));
            if mutable_global_names.contains(name.as_str()) {
                import_names.push(format!("$set_{}", sanitize_ident(name)));
            }
        }
        let _ = writeln!(
            out,
            "import {{ {} }} from \"./_globals\";",
            import_names.join(", ")
        );
    }

    emit_imports(module, &mut out);
    let closure_fids: Vec<FuncId> = free_funcs
        .iter()
        .copied()
        .filter(|&fid| module.functions[fid].method_kind == MethodKind::Closure)
        .collect();
    let closure_bodies = compile_closures(&closure_fids, module, lowering_config, engine, debug);
    let no_sys_aliases = BTreeMap::new();
    for &fid in free_funcs {
        if module.functions[fid].method_kind != MethodKind::Closure {
            emit_function(
                &mut module.functions[fid],
                class_names,
                known_classes,
                mutable_global_names,
                lowering_config,
                engine,
                &module.sprite_names,
                &module.object_names,
                &closure_bodies,
                &free_stateful_names,
                free_func_names,
                &no_sys_aliases,
                runtime_config,
                &class_meta.unique_static_field_map,
                debug,
                &mut out,
                diagnostics,
            )?;
        }
    }
    strip_unused_namespace_imports(&mut out);
    let path = module_dir.join("_init.ts");
    fs::write(&path, &out).map_err(CoreError::Io)?;
    Ok(true)
}

/// Write the barrel `index.ts` that re-exports all emitted files.
fn write_barrel_file(module_dir: &Path, barrel_exports: &[String]) -> Result<(), CoreError> {
    let mut barrel = String::new();
    for export_path in barrel_exports {
        let _ = writeln!(barrel, "export * from \"./{export_path}\";");
    }
    fs::write(module_dir.join("index.ts"), &barrel).map_err(CoreError::Io)?;
    Ok(())
}

/// Emit a module as a directory with one `.ts` file per class in nested dirs.
pub fn emit_module_to_dir(
    module: &mut Module,
    output_dir: &Path,
    lowering_config: &LoweringConfig,
    runtime_config: Option<&RuntimeConfig>,
    debug: &DebugConfig,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), CoreError> {
    let module_dir = output_dir.join(&module.name);
    fs::create_dir_all(&module_dir).map_err(CoreError::Io)?;

    let (class_groups, free_funcs) = group_by_class(module);
    let class_names = build_class_names(module);
    let registry = ClassRegistry::from_module(module, &class_names);
    let empty_type_defs = BTreeMap::new();
    let type_defs = runtime_config
        .map(|c| &c.type_definitions)
        .unwrap_or(&empty_type_defs);
    let empty_mod_exports = BTreeMap::new();
    let module_exports = runtime_config
        .map(|c| &c.module_exports)
        .unwrap_or(&empty_mod_exports);
    let empty_func_sigs = BTreeMap::new();
    let func_sigs = runtime_config
        .map(|c| &c.function_signatures)
        .unwrap_or(&empty_func_sigs);
    let class_meta = ClassMeta::build(module, type_defs);
    let global_names: HashSet<String> = module.globals.iter().map(|g| g.name.clone()).collect();
    let mutable_global_names: HashSet<String> = module
        .globals
        .iter()
        .filter(|g| g.mutable)
        .map(|g| g.name.clone())
        .collect();
    let short_to_qualified: HashMap<String, String> = module
        .classes
        .iter()
        .map(|c| (c.name.clone(), qualified_class_name(c)))
        .collect();
    let mut known_classes: HashSet<String> = class_names.values().cloned().collect();
    if let Some(rc) = runtime_config {
        known_classes.extend(rc.type_definitions.keys().cloned());
    }
    let engine = detect_engine(runtime_config);

    // Rename colliding free functions before emission.
    rename_colliding_free_funcs(module, &free_funcs, &known_classes);

    // Rename local variables that shadow imported function names (e.g. `int`).
    let imported_names = collect_call_names_from_funcs(module.functions.values(), engine);
    rename_shadowing_locals(module, &imported_names);

    // Rebuild free_func_names after potential renames.
    let free_func_names: HashSet<String> = free_funcs
        .iter()
        .map(|&fid| sanitize_ident(&module.functions[fid].name))
        .collect();
    let mut barrel_exports: Vec<String> = Vec::new();

    // Globals → _globals.ts
    if emit_globals_file(module, &module_dir, &registry, runtime_config)? {
        barrel_exports.push("_globals".to_string());
    }

    // Pre-collect transitive value imports for cycle detection.
    let transitive_value_imports = collect_transitive_imports(
        &class_groups,
        module,
        &registry,
        &class_meta,
        &class_names,
        &global_names,
        engine,
    );

    // Emit one .ts file per class.
    for group in &class_groups {
        emit_class_file(
            group,
            module,
            &module_dir,
            &class_names,
            &class_meta,
            &registry,
            &global_names,
            &mutable_global_names,
            &free_func_names,
            &known_classes,
            &short_to_qualified,
            module_exports,
            &transitive_value_imports,
            type_defs,
            func_sigs,
            lowering_config,
            runtime_config,
            engine,
            debug,
            &mut barrel_exports,
            diagnostics,
        )?;
    }

    // Free functions → _init.ts
    if emit_free_functions_file(
        module,
        &module_dir,
        &free_funcs,
        &free_func_names,
        &class_names,
        &class_meta,
        &registry,
        &global_names,
        &mutable_global_names,
        &known_classes,
        module_exports,
        lowering_config,
        runtime_config,
        engine,
        debug,
        diagnostics,
    )? {
        barrel_exports.push("_init".to_string());
    }

    // Barrel file: index.ts
    write_barrel_file(&module_dir, &barrel_exports)?;

    Ok(())
}
