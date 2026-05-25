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
use reincarnate_core::ir::module::TypeDecl;
use reincarnate_core::ir::{ClassDef, FieldDef, FuncId, MethodKind, Module, Visibility};
use reincarnate_core::pipeline::{DebugConfig, Diagnostic, LoweringConfig};
use reincarnate_core::project::{ExternalMethodSig, ExternalTypeDef, RuntimeConfig};

use crate::runtime::SYSTEM_NAMES;
use crate::types::{ts_type, ts_type_with_module};

// Re-export public items from sub-modules.
pub(crate) use class::ClassGroup;
pub(crate) use sanitize::sanitize_ident;

// Use items from sub-modules within this module.
use class::{compile_closures, emit_class, emit_function, emit_functions, group_by_class};
use imports::{
    build_intrinsic_calls_map, build_intrinsic_to_system, collect_all_struct_names,
    collect_call_names_from_funcs, collect_class_references, collect_global_type_imports,
    collect_system_names_from_funcs, collect_type_refs_from_function,
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
///   Also sets `foreach_iterator_system = "Flash.Iterator"` (HasNext2 for-of pattern).
/// - GML: sets `wrap_class_refs_as_any = true` so that `ClassRef`-typed GlobalRef
///   values get `as any` at each use site (GML object names double as integer indices).
///   When `module` is provided, also populates `intrinsic_calls` from registered intrinsics
///   so that `Op::Call` with an intrinsic function is lowered to `Expr::SystemCall`.
///   When `module` is `None` and `intrinsic_calls` is already populated (e.g. caller
///   pre-built the config), the existing map is preserved.
/// - Twine: sets `output_node_system = ("Harlowe.H", "h")` so that Harlowe output-node
///   SystemCalls are lowered to `h.method()` MethodCalls before optimization.
pub(crate) fn lowering_config_for_engine<'a>(
    config: &'a LoweringConfig,
    engine: EngineKind,
    module: Option<&Module>,
) -> std::borrow::Cow<'a, LoweringConfig> {
    // Build func_names map from the module's name table so the linear emitter
    // can resolve FuncId → function name for builtin prefix checks and
    // intrinsic_calls lookups.  Only when we have the module AND the map
    // isn't already populated.
    let module_func_names: Option<std::collections::HashMap<FuncId, String>> =
        if config.func_names.is_empty() {
            module.map(|m| {
                m.functions
                    .iter()
                    .map(|(fid, _func)| (fid, m.name_table.func_name(fid).to_string()))
                    .collect()
            })
        } else {
            None
        };

    // Build pure_fids set from core_builtin_fids — these map to native operators
    // and have no side effects.
    // Only when we have the module AND the set isn't already populated.
    let module_pure_fids: Option<std::collections::HashSet<FuncId>> = if config.pure_fids.is_empty()
    {
        module.map(|m| m.core_builtin_fids.clone())
    } else {
        None
    };

    let needs_flash = engine == EngineKind::Flash
        && (config.scope_lookup_systems.is_empty()
            || config.foreach_iterator_system.is_none()
            || !config.construct_string_coerce
            || !config.coerce_index_types);
    // Build intrinsic_calls map for GML: name → (system, method).
    // Only when we have the module AND the map isn't already populated.
    let gml_intrinsic_calls: Option<std::collections::HashMap<String, (String, String)>> =
        if engine == EngineKind::GameMaker && config.intrinsic_calls.is_empty() {
            if let Some(m) = module {
                let map: std::collections::HashMap<String, (String, String)> = m
                    .runtime_registry
                    .iter()
                    .filter_map(|(name, &fid)| {
                        m.functions[fid].intrinsic.as_ref().map(|k| {
                            let (sys, meth) = k.system_method();
                            (name.clone(), (sys.to_string(), meth.to_string()))
                        })
                    })
                    .collect();
                if map.is_empty() {
                    None
                } else {
                    Some(map)
                }
            } else {
                None
            }
        } else {
            None
        };
    let needs_gml = engine == EngineKind::GameMaker
        && (!config.wrap_class_refs_as_any || gml_intrinsic_calls.is_some());
    let needs_twine = engine == EngineKind::Twine
        && (config.output_node_system.is_none()
            || config.cast_narrowed_syscall_results_for.is_empty()
            || !config.cast_unknown_indirect_callee);
    let needs_func_names = module_func_names.is_some() || module_pure_fids.is_some();
    if needs_flash || needs_gml || needs_twine || needs_func_names {
        let mut c = config.clone();
        if let Some(fnames) = module_func_names {
            c.func_names = fnames;
        }
        if let Some(pfids) = module_pure_fids {
            c.pure_fids = pfids;
        }
        if needs_flash {
            c.scope_lookup_systems = vec!["Flash.Scope".to_string()];
            c.foreach_iterator_system = Some("Flash.Iterator".to_string());
            c.construct_string_coerce = true;
            c.coerce_index_types = true;
        }
        if needs_gml {
            c.wrap_class_refs_as_any = true;
            if let Some(map) = gml_intrinsic_calls {
                c.intrinsic_calls = map;
            }
            // Inject `as <type>` casts for scalar and struct results from GameMaker
            // instance field getters narrowed by type inference. HasField narrowing
            // is now conservative (a field is only a discriminant if no non-leaf type
            // defines it in its own fields), so struct/array casts are safe to inject.
            let gml_instance_getters = vec![
                ("GameMaker.Instance".to_string(), "getOn".to_string()),
                ("GameMaker.Instance".to_string(), "getAll".to_string()),
                ("GameMaker.Instance".to_string(), "getField".to_string()),
                ("GameMaker.Instance".to_string(), "getOther".to_string()),
            ];
            c.cast_narrowed_syscall_results_for = gml_instance_getters.clone();
            c.cast_struct_syscall_results_for = gml_instance_getters;
        }
        if needs_twine {
            c.output_node_system = Some(("Harlowe.H".to_string(), "h".to_string()));
            // Inject `as <type>` casts on State.get() results that have been
            // narrowed by type inference.  The SugarCube runtime declares
            // `State.get(name): unknown`; type inference can narrow the actual
            // type, and the cast surfaces it in the emitted TypeScript.
            c.cast_narrowed_syscall_results_for = vec![
                ("SugarCube.State".to_string(), "get".to_string()),
                ("SugarCube.Setup".to_string(), "get".to_string()),
            ];
            // Engine.resolve() is used for bare-identifier lookups in SugarCube
            // expressions — both story variables and JS builtins (Date, Math, …).
            // Only inject struct casts so that named TypeScript overloads for
            // builtins (DateConstructor, etc.) are not shadowed by a wrong type.
            c.cast_struct_syscall_results_for =
                vec![("SugarCube.Engine".to_string(), "resolve".to_string())];
            // Cast Unknown-typed indirect callees to a function type so that
            // Engine.resolve("fn")(...) doesn't produce TS2571.
            c.cast_unknown_indirect_callee = true;
        }
        std::borrow::Cow::Owned(c)
    } else {
        std::borrow::Cow::Borrowed(config)
    }
}

/// Pre-intern runtime type names into the module's type arena.
///
/// `emit_function` only receives `module_types: &PrimaryMap<TypeId, TypeDecl>` (not
/// the full `Module`), so it cannot call `module.intern_type()`. Interning the
/// runtime/context type names here — before any emit calls — ensures that
/// `find_type_id(module_types, name)` finds them during emission and they can be
/// stored as `Type::Instance(id)` rather than `Type::Struct(name)`.
fn intern_runtime_types(module: &mut Module, runtime_config: Option<&RuntimeConfig>) {
    if let Some(rc) = runtime_config {
        if let Some(rt_type) = &rc.runtime_type {
            module.intern_type(&rt_type.name);
        }
        if let Some(ctx_type) = &rc.context_type {
            module.intern_type(&ctx_type.name);
        }
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
    intern_runtime_types(module, runtime_config);
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
    // No GameGlobalState type in flat module mode (no classes, no _globals.ts).
    let no_game_global_state: Option<reincarnate_core::ir::TypeId> = None;
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
            no_game_global_state,
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
                no_game_global_state,
                debug,
                &mut out,
                &mut traits_buf,
                diagnostics,
                None,
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
        let object_ts_names = class::resolve_object_ts_names(&module.object_names, &class_names);
        let name_map: HashMap<String, String> = module
            .object_names
            .iter()
            .zip(object_ts_names.iter())
            .filter(|(raw, ts)| raw != ts)
            .map(|(raw, ts)| (raw.clone(), ts.clone()))
            .collect();
        for &fid in &free_funcs {
            if module.functions[fid].method_kind != MethodKind::Closure {
                let overloads = class::collect_overloads(&module.functions, fid);
                let func_name = module.name_table.func_name(fid).to_string();
                emit_function(
                    &mut module.functions[fid],
                    &func_name,
                    &module.types,
                    &class_names,
                    &known_classes,
                    &no_mutable_globals,
                    lowering_config,
                    engine,
                    &module.sprite_names,
                    &object_ts_names,
                    &closure_bodies,
                    &no_stateful,
                    &no_free_fns,
                    &no_sys_aliases,
                    runtime_config,
                    &class_meta.unique_static_field_map,
                    &name_map,
                    overloads,
                    no_game_global_state,
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
                short_name: ts_name.clone(),
                path_segments: segments.clone(),
            });
            // Also key by the sanitized short name so that raw GML names
            // (e.g. "TOTCLeaderboard") resolve to the disambiguated ts_name
            // even when the file-system name has a namespace prefix.
            classes.entry(base_short.clone()).or_insert(ClassEntry {
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
    // Pure structs are TypeDecl::Object entries whose TypeId does NOT appear as any
    // ClassDef's type_id.  Class TypeDecls are emitted as `class` declarations elsewhere.
    use reincarnate_core::ir::ty::TypeId;
    let class_type_ids: HashSet<TypeId> = module.classes.iter().map(|c| c.type_id).collect();
    for (id, td) in module.types.iter() {
        if class_type_ids.contains(&id) {
            continue;
        }
        if let TypeDecl::Object {
            name: Some(name),
            visibility,
            fields,
            ..
        } = td
        {
            if fields.is_empty() {
                continue;
            }
            let needs_index_sig = module.string_indexed_structs.contains(name.as_str());
            emit_struct_fields(name, *visibility, fields, needs_index_sig, out);
        }
    }
}

fn emit_struct_fields(
    name: &str,
    visibility: Visibility,
    fields: &[FieldDef],
    needs_index_signature: bool,
    out: &mut String,
) {
    let vis = visibility_prefix(visibility);
    let _ = writeln!(out, "{vis}interface {} {{", sanitize_ident(name));
    if needs_index_signature {
        let _ = writeln!(out, "  [key: string]: unknown;");
    }
    for field in fields {
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
        collect_global_type_imports(&global.ty, &module.types, registry, &mut type_imports);
        collect_all_struct_names(&global.ty, &module.types, &mut all_struct_names);
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

    // Emit companion _global_state.ts with the GameGlobalState intersection type.
    // This allows constant-key global accesses to be cast to a fully-typed shape.
    let mut state_out = String::new();
    if let Some(preamble) = runtime_config.and_then(|c| c.class_preamble.as_ref()) {
        let _ = writeln!(
            state_out,
            "import {{ GMLObject }} from \"../runtime/{}\";",
            preamble.path,
        );
        state_out.push('\n');
    }
    let _ = writeln!(state_out, "export type GameGlobalState = GMLObject & {{");
    for global in &module.globals {
        let ident = sanitize_ident(&global.name);
        let _ = writeln!(state_out, "  {ident}: unknown;");
    }
    let _ = writeln!(state_out, "}};");
    let state_path = module_dir.join("_global_state.ts");
    fs::write(&state_path, &state_out).map_err(CoreError::Io)?;

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
        let qualified = qualified_class_name(&group.class_def);
        let ts_name = class_names
            .get(&qualified)
            .cloned()
            .unwrap_or_else(|| sanitize_ident(&group.class_def.name));
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
            &ts_name,
        );
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
    generated_runtime_names: &BTreeSet<String>,
    lowering_config: &LoweringConfig,
    runtime_config: Option<&RuntimeConfig>,
    engine: EngineKind,
    game_global_state_type_id: Option<reincarnate_core::ir::TypeId>,
    debug: &DebugConfig,
    barrel_exports: &mut Vec<String>,
    seen_paths: &mut HashMap<String, usize>,
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

    // Detect file-path collisions: two classes in the same namespace with the
    // same sanitized name (e.g., a game OBJT chunk with duplicate object names).
    // When a collision is detected, append a numeric suffix (_2, _3, …) to the
    // last segment so each class gets a unique file and unique export identifier.
    let base_export_path = segments.join("/");
    let collision_n = {
        let count = seen_paths.entry(base_export_path).or_insert(0);
        *count += 1;
        *count
    };
    let ts_name_override: Option<String>;
    if collision_n > 1 {
        // Derive the ts_name from class_names (same qualified key, first one wins).
        let qualified = qualified_class_name(class_def);
        let base_ts = class_names
            .get(&qualified)
            .cloned()
            .unwrap_or_else(|| short_name.clone());
        let unique_suffix = format!("{base_ts}_{collision_n}");
        if let Some(last) = segments.last_mut() {
            *last = format!("{last}_{collision_n}");
        }
        ts_name_override = Some(unique_suffix);
    } else {
        ts_name_override = None;
    }

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
    let intrinsic_to_system = build_intrinsic_to_system(module);
    let all_systems = collect_system_names_from_funcs(class_funcs(), Some(&intrinsic_to_system));
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
    let intrinsic_calls_class = build_intrinsic_calls_map(module);
    let func_names_class: HashMap<FuncId, String> = module
        .functions
        .keys()
        .map(|fid| (fid, module.func_name(fid).to_string()))
        .collect();
    let calls = collect_call_names_from_funcs(
        class_funcs(),
        engine,
        Some(&intrinsic_calls_class),
        &func_names_class,
    );
    let func_prefix = "../".repeat(depth + 1);
    let func_prefix = func_prefix.trim_end_matches('/');
    // Game-defined scripts shadow runtime functions with the same name.
    // Only pass runtime calls (names not defined in _init.ts) to the prefix emitter.
    let runtime_calls: BTreeSet<String> = calls
        .iter()
        .filter(|n| !free_func_names.contains(n.as_str()))
        .cloned()
        .collect();

    // Calls to IR-bodied runtime functions import from `_runtime`.
    let generated_calls: BTreeSet<String> = runtime_calls
        .iter()
        .filter(|n| generated_runtime_names.contains(n.as_str()))
        .cloned()
        .collect();
    if !generated_calls.is_empty() {
        // _runtime.ts lives at module_dir; traverse `depth` parent dirs to reach it.
        // depth=0 → "./_runtime", depth=1 → "../_runtime", etc.
        let runtime_prefix = if depth == 0 {
            ".".to_string()
        } else {
            "../".repeat(depth).trim_end_matches('/').to_string()
        };
        let names: Vec<&str> = {
            let mut v: Vec<&str> = generated_calls.iter().map(|s| s.as_str()).collect();
            v.sort();
            v
        };
        let _ = writeln!(
            out,
            "import {{ {} }} from \"{runtime_prefix}/_runtime\";",
            names.join(", ")
        );
        out.push('\n');
    }

    // Calls to handwritten runtime functions still use function_modules routing.
    let handwritten_calls: BTreeSet<String> = runtime_calls
        .iter()
        .filter(|n| !generated_runtime_names.contains(n.as_str()))
        .cloned()
        .collect();
    let mut stateful_names = BTreeSet::new();
    emit_function_imports_with_prefix(
        &handwritten_calls,
        &mut out,
        func_prefix,
        runtime_config,
        &mut stateful_names,
        module,
    );
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
    // Compute the disambiguated TypeScript identifier for self (used by
    // emit_intra_imports to avoid false self-import matches when two classes
    // share the same raw GML name).
    let self_ts_name_owned;
    let self_ts_name = if let Some(ov) = ts_name_override.as_deref() {
        ov
    } else {
        self_ts_name_owned = class_names
            .get(&qualified)
            .cloned()
            .unwrap_or_else(|| short_name.clone());
        &self_ts_name_owned
    };
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
        game_global_state_type_id,
        self_ts_name,
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
            fid,
            module,
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
        game_global_state_type_id,
        debug,
        &mut out,
        &mut traits_buf,
        diagnostics,
        ts_name_override.as_deref(),
    )?;

    strip_unused_namespace_imports(&mut out);
    // File name: use the last segment of `segments` (may have a collision suffix).
    let file_name = segments.last().map(|s| s.as_str()).unwrap_or(&short_name);
    let path = file_dir.join(format!("{file_name}.ts"));
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

/// Collect the FuncIds of runtime functions that have non-stub IR bodies.
///
/// A stub is registered with a single entry block containing no instructions
/// and a `Return(None)` terminator. After `register_runtime_bodies` runs,
/// functions like `floor`, `dsin`, `point_distance`, etc. have `InlineHint::Always`
/// set and non-empty instruction maps. We use `InlineHint::Always` as the
/// canonical indicator that a body was attached.
fn collect_runtime_body_fids(module: &Module) -> Vec<FuncId> {
    use reincarnate_core::ir::func::InlineHint;
    // Deduplicate: aliased functions share a FuncId, so values() may yield
    // the same FuncId more than once.
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for &fid in module.runtime_registry.values() {
        if module.functions[fid].inline_hint == InlineHint::Always && seen.insert(fid) {
            out.push(fid);
        }
    }
    // Sort by name for stable output.
    out.sort_by_key(|&fid| module.func_name(fid));
    out
}

/// Emit `_runtime.ts` — IR-bodied runtime function definitions generated from IR.
///
/// These are pure math/string/color functions that have IR bodies attached by
/// `register_runtime_bodies`. They do NOT take `_rt` as a parameter.
/// Returns `true` if the file was written (so the caller can add a barrel export).
#[allow(clippy::too_many_arguments)]
fn emit_runtime_functions_file(
    module: &mut Module,
    module_dir: &Path,
    runtime_fids: &[FuncId],
    generated_runtime_names: &BTreeSet<String>,
    class_names: &HashMap<String, String>,
    _class_meta: &ClassMeta,
    lowering_config: &LoweringConfig,
    runtime_config: Option<&RuntimeConfig>,
    engine: EngineKind,
    debug: &DebugConfig,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<bool, CoreError> {
    if runtime_fids.is_empty() {
        return Ok(false);
    }
    let mut out = String::new();

    // Collect all function names called by the runtime body functions.
    let func_names_rt: HashMap<FuncId, String> = module
        .functions
        .keys()
        .map(|fid| (fid, module.func_name(fid).to_string()))
        .collect();
    let intrinsic_calls_rt = build_intrinsic_calls_map(module);
    let rt_fn_iter = || runtime_fids.iter().map(|&fid| &module.functions[fid]);
    let calls = collect_call_names_from_funcs(
        rt_fn_iter(),
        engine,
        Some(&intrinsic_calls_rt),
        &func_names_rt,
    );

    // Functions in `_runtime.ts` that call other functions also in `_runtime.ts`
    // don't need imports — they're in the same file. Only functions calling
    // handwritten runtime modules need imports.
    let external_calls: BTreeSet<String> = calls
        .iter()
        .filter(|n| !generated_runtime_names.contains(n.as_str()))
        .cloned()
        .collect();

    // Import any handwritten runtime functions called from within this file.
    let mut _rt_stateful_names = BTreeSet::new();
    emit_function_imports_with_prefix(
        &external_calls,
        &mut out,
        "..",
        runtime_config,
        &mut _rt_stateful_names,
        module,
    );

    let object_ts_names = class::resolve_object_ts_names(&module.object_names, class_names);
    let name_map: HashMap<String, String> = module
        .object_names
        .iter()
        .zip(object_ts_names.iter())
        .filter(|(raw, ts)| raw != ts)
        .map(|(raw, ts)| (raw.clone(), ts.clone()))
        .collect();

    let effective_lowering = lowering_config_for_engine(lowering_config, engine, Some(module));
    let effective_lowering_ref: &LoweringConfig = &effective_lowering;

    // No closures in runtime body functions.
    let closure_bodies: HashMap<String, crate::js_ast::JsFunction> = HashMap::new();
    // Runtime body functions are pure — no stateful names, no game free funcs.
    let no_stateful = BTreeSet::new();
    let no_free_fns = HashSet::new();
    let no_sys_aliases = BTreeMap::new();
    let no_unique_static = HashMap::new();
    let known_classes: HashSet<String> = class_names.values().cloned().collect();

    for &fid in runtime_fids {
        let func_name = module.name_table.func_name(fid).to_string();
        let overloads = class::collect_overloads(&module.functions, fid);
        emit_function(
            &mut module.functions[fid],
            &func_name,
            &module.types,
            class_names,
            &known_classes,
            &HashSet::new(),
            effective_lowering_ref,
            engine,
            &module.sprite_names,
            &object_ts_names,
            &closure_bodies,
            &no_stateful,
            &no_free_fns,
            &no_sys_aliases,
            runtime_config,
            &no_unique_static,
            &name_map,
            overloads,
            None,
            debug,
            &mut out,
            diagnostics,
        )?;
    }

    strip_unused_namespace_imports(&mut out);
    let path = module_dir.join("_runtime.ts");
    fs::write(&path, &out).map_err(CoreError::Io)?;
    Ok(true)
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
    generated_runtime_names: &BTreeSet<String>,
    lowering_config: &LoweringConfig,
    runtime_config: Option<&RuntimeConfig>,
    engine: EngineKind,
    game_global_state_type_id: Option<reincarnate_core::ir::TypeId>,
    debug: &DebugConfig,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<bool, CoreError> {
    if free_funcs.is_empty() {
        return Ok(false);
    }
    let mut out = String::new();
    let free_fn_iter = || free_funcs.iter().map(|&fid| &module.functions[fid]);
    let object_ts_names = class::resolve_object_ts_names(&module.object_names, class_names);
    let name_map: HashMap<String, String> = module
        .object_names
        .iter()
        .zip(object_ts_names.iter())
        .filter(|(raw, ts)| raw != ts)
        .map(|(raw, ts)| (raw.clone(), ts.clone()))
        .collect();
    let intrinsic_to_system = build_intrinsic_to_system(module);
    let all_free_systems =
        collect_system_names_from_funcs(free_fn_iter(), Some(&intrinsic_to_system));
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
    let intrinsic_calls_free = build_intrinsic_calls_map(module);
    let func_names_free: HashMap<FuncId, String> = module
        .functions
        .keys()
        .map(|fid| (fid, module.func_name(fid).to_string()))
        .collect();
    let calls = collect_call_names_from_funcs(
        free_fn_iter(),
        engine,
        Some(&intrinsic_calls_free),
        &func_names_free,
    );
    // Game-defined free functions shadow runtime functions with the same name.
    // Only pass runtime calls (names not defined in this file) to the prefix emitter.
    let free_runtime_calls: BTreeSet<String> = calls
        .iter()
        .filter(|n| !free_func_names.contains(n.as_str()))
        .cloned()
        .collect();

    // Calls to IR-bodied runtime functions import from `_runtime`.
    let generated_calls: BTreeSet<String> = free_runtime_calls
        .iter()
        .filter(|n| generated_runtime_names.contains(n.as_str()))
        .cloned()
        .collect();
    if !generated_calls.is_empty() {
        let names: Vec<&str> = {
            let mut v: Vec<&str> = generated_calls.iter().map(|s| s.as_str()).collect();
            v.sort();
            v
        };
        let _ = writeln!(
            out,
            "import {{ {} }} from \"./_runtime\";",
            names.join(", ")
        );
        out.push('\n');
    }

    // Calls to handwritten runtime functions still use function_modules routing.
    let handwritten_calls: BTreeSet<String> = free_runtime_calls
        .iter()
        .filter(|n| !generated_runtime_names.contains(n.as_str()))
        .cloned()
        .collect();
    let mut free_stateful_names = BTreeSet::new();
    emit_function_imports_with_prefix(
        &handwritten_calls,
        &mut out,
        "..",
        runtime_config,
        &mut free_stateful_names,
        module,
    );
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

    // Sprite enum import for free functions (same as per-class emit).
    if !module.sprite_names.is_empty() {
        let _ = writeln!(out, "import {{ Sprites }} from \"../data/sprites\";");
    }

    // Scan free functions for external class references.
    let intrinsic_calls_free = build_intrinsic_calls_map(module);
    let mut refs = RefSets::default();
    for &fid in free_funcs {
        let func = &module.functions[fid];
        collect_type_refs_from_function(
            func,
            "",
            "",
            &module.types,
            registry,
            &module.external_imports,
            &HashMap::new(),
            &HashMap::new(),
            global_names,
            &HashMap::new(),
            &object_ts_names,
            engine,
            Some(&intrinsic_calls_free),
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
        // GameMaker constant-key global accesses use `(_rt.global as GameGlobalState).field`.
        if game_global_state_type_id.is_some() {
            let _ = writeln!(
                out,
                "import type {{ GameGlobalState }} from \"./_global_state\";",
            );
        }
    }

    emit_imports(module, &mut out);

    // Collect the names of free constructor functions so we know which TypeDecls
    // to emit as interfaces.
    let constructor_names: HashSet<String> = free_funcs
        .iter()
        .filter(|&&fid| module.functions[fid].method_kind == MethodKind::Constructor)
        .map(|&fid| module.name_table.func_name(fid).to_string())
        .collect();

    // Emit `export interface Name extends GMLObject { … }` for each inferred
    // constructor type whose name matches a constructor function in this module.
    if !constructor_names.is_empty() {
        let mut interfaces_out = String::new();
        for (_type_id, decl) in module.types.iter() {
            let TypeDecl::Object {
                name: Some(ref raw_name),
                inferred: true,
                ref parent,
                ref fields,
                ..
            } = *decl
            else {
                continue;
            };
            // Short name after `::` — must match a constructor function name and
            // must be a valid TypeScript identifier (no `@` or other non-identifier
            // characters that appear in anonymous/internal GML constructor names).
            let short = raw_name.rsplit("::").next().unwrap_or(raw_name.as_str());
            if !constructor_names.contains(short) {
                continue;
            }
            // Skip names that are not valid identifiers — they cannot be emitted as
            // TypeScript interface names.
            if !short
                .chars()
                .all(|c| c.is_alphanumeric() || c == '_' || c == '$')
            {
                continue;
            }
            // Resolve the parent type name for `extends`, if set.
            let extends_clause = if let Some(parent_id) = parent {
                if let Some(parent_decl) = module.types.get(*parent_id) {
                    if let Some(parent_name) = parent_decl.name() {
                        let parent_short = parent_name.rsplit("::").next().unwrap_or(parent_name);
                        format!(" extends {parent_short}")
                    } else {
                        String::new()
                    }
                } else {
                    String::new()
                }
            } else {
                String::new()
            };
            let _ = writeln!(
                interfaces_out,
                "export interface {short}{extends_clause} {{"
            );
            for field in fields {
                let field_ts_type = ts_type_with_module(&field.ty, &module.types);
                let field_name = sanitize_ident(&field.name);
                let _ = writeln!(interfaces_out, "  {field_name}: {field_ts_type};");
            }
            let _ = writeln!(interfaces_out, "}}");
        }
        if !interfaces_out.is_empty() {
            out.push_str(&interfaces_out);
            out.push('\n');
        }
    }

    let closure_fids: Vec<FuncId> = free_funcs
        .iter()
        .copied()
        .filter(|&fid| module.functions[fid].method_kind == MethodKind::Closure)
        .collect();
    // Pre-build effective lowering config with intrinsic_calls populated from
    // the module so that Op::Call with intrinsic functions is lowered to
    // Expr::SystemCall by the linear emitter (same as emit_class_functions does).
    let effective_lowering = lowering_config_for_engine(lowering_config, engine, Some(module));
    let effective_lowering_ref: &LoweringConfig = &effective_lowering;
    let closure_bodies =
        compile_closures(&closure_fids, module, effective_lowering_ref, engine, debug);
    let no_sys_aliases = BTreeMap::new();
    for &fid in free_funcs {
        if module.functions[fid].method_kind != MethodKind::Closure {
            let overloads = class::collect_overloads(&module.functions, fid);
            let func_name = module.name_table.func_name(fid).to_string();
            emit_function(
                &mut module.functions[fid],
                &func_name,
                &module.types,
                class_names,
                known_classes,
                mutable_global_names,
                effective_lowering_ref,
                engine,
                &module.sprite_names,
                &object_ts_names,
                &closure_bodies,
                &free_stateful_names,
                free_func_names,
                &no_sys_aliases,
                runtime_config,
                &class_meta.unique_static_field_map,
                &name_map,
                overloads,
                game_global_state_type_id,
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
    intern_runtime_types(module, runtime_config);
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
    let intrinsic_calls_global = build_intrinsic_calls_map(module);
    let func_names_global: HashMap<FuncId, String> = module
        .runtime_registry
        .iter()
        .map(|(name, &fid)| (fid, name.clone()))
        .collect();
    let imported_names = collect_call_names_from_funcs(
        module.functions.values(),
        engine,
        Some(&intrinsic_calls_global),
        &func_names_global,
    );
    rename_shadowing_locals(module, &imported_names);

    // Rebuild free_func_names after potential renames.
    let free_func_names: HashSet<String> = free_funcs
        .iter()
        .map(|&fid| sanitize_ident(module.func_name(fid)))
        .collect();

    // Collect generated runtime function names before emitting anything so that
    // all emit steps can route imports to `_runtime` vs. handwritten modules.
    let runtime_fids = collect_runtime_body_fids(module);
    // Include ALL registry keys (canonical names + aliases) that resolve to an
    // IR-body FuncId, so the routing covers every name a call site might emit.
    let generated_runtime_names: BTreeSet<String> = {
        use reincarnate_core::ir::func::InlineHint;
        module
            .runtime_registry
            .iter()
            .filter(|(_name, &fid)| module.functions[fid].inline_hint == InlineHint::Always)
            .map(|(name, _fid)| sanitize_ident(name))
            .collect()
    };

    let mut barrel_exports: Vec<String> = Vec::new();

    // Globals → _globals.ts
    if emit_globals_file(module, &module_dir, &registry, runtime_config)? {
        barrel_exports.push("_globals".to_string());
        barrel_exports.push("_global_state".to_string());
    }

    // Intern GameGlobalState type id for use in typed cast expressions.
    let game_global_state_type_id: Option<reincarnate_core::ir::TypeId> =
        if !module.globals.is_empty() && engine == EngineKind::GameMaker {
            Some(module.intern_type("GameGlobalState"))
        } else {
            None
        };

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
    let mut seen_paths: HashMap<String, usize> = HashMap::new();
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
            &generated_runtime_names,
            lowering_config,
            runtime_config,
            engine,
            game_global_state_type_id,
            debug,
            &mut barrel_exports,
            &mut seen_paths,
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
        &generated_runtime_names,
        lowering_config,
        runtime_config,
        engine,
        game_global_state_type_id,
        debug,
        diagnostics,
    )? {
        barrel_exports.push("_init".to_string());
    }

    // Generated runtime functions → _runtime.ts
    // _runtime.ts is internal: not added to the barrel because its exports are
    // lower-level runtime helpers, not game API. Each file that needs them
    // imports directly from "./_runtime".
    emit_runtime_functions_file(
        module,
        &module_dir,
        &runtime_fids,
        &generated_runtime_names,
        &class_names,
        &class_meta,
        lowering_config,
        runtime_config,
        engine,
        debug,
        diagnostics,
    )?;

    // Barrel file: index.ts
    write_barrel_file(&module_dir, &barrel_exports)?;

    Ok(())
}
