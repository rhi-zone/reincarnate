// ---------------------------------------------------------------------------
// Class grouping, class emission, and function emission
// ---------------------------------------------------------------------------

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fmt::Write;

use reincarnate_core::entity::PrimaryMap;
use reincarnate_core::error::CoreError;
use reincarnate_core::ir::module::TypeDecl;
use reincarnate_core::ir::{
    structurize, ClassDef, Constant, FuncId, Function, MethodKind, Module, StructDef, Type, TypeId,
};
use reincarnate_core::pipeline::{DebugConfig, Diagnostic, LoweringConfig};
use reincarnate_core::project::{ExternalMethodSig, RuntimeConfig};

use crate::js_ast::{JsExpr, JsFunction, JsStmt};
use crate::types::{
    find_type_id, flash_ts_type_with_names_and_module, ts_type_with_names_and_module,
};

use super::rewrites;
use super::sanitize::resolve_sprite_constant;
use super::scaffold::ClassMeta;
use super::{lowering_config_for_engine, qualified_class_name, sanitize_ident, EngineKind};

/// Build a map from short class name → TypeId for all Object TypeDecls in `module_types`.
///
/// Used to populate `FlashRewriteCtx::class_type_ids` so that Flash rewrites can
/// construct `Type::Instance(id)` casts without needing `Type::Struct(name)`.
fn build_class_type_ids(module_types: &PrimaryMap<TypeId, TypeDecl>) -> HashMap<String, TypeId> {
    module_types
        .iter()
        .filter_map(|(id, decl)| {
            decl.name().map(|name| {
                let short = name.rsplit("::").next().unwrap_or(name).to_string();
                (short, id)
            })
        })
        .collect()
}

/// Map raw GML object names (indexed by OBJT index) to their disambiguated
/// TypeScript class identifiers.  When two objects share the same sanitized
/// name (e.g. two `TOTCLeaderboard` entries in the OBJT chunk), both indexes
/// resolve to the first object's ts_name rather than the raw name that no
/// longer matches any exported identifier.
///
/// For non-GML modules `object_names` is empty and an empty vec is returned.
pub(super) fn resolve_object_ts_names(
    object_names: &[String],
    class_names: &HashMap<String, String>,
) -> Vec<String> {
    object_names
        .iter()
        .map(|name| {
            // GML frontend always places objects in the "objects" namespace.
            let qualified = format!("objects::{name}");
            class_names
                .get(&qualified)
                .cloned()
                .unwrap_or_else(|| name.clone())
        })
        .collect()
}

pub(crate) struct ClassGroup {
    pub(crate) class_def: ClassDef,
    pub(crate) struct_def: StructDef,
    pub(crate) methods: Vec<FuncId>,
}

/// Partition module contents into class groups and free functions.
///
/// Classes are returned in topological order (superclass before subclass) so
/// that barrel-file exports work correctly with bundlers like esbuild that
/// flatten modules into a single scope.
pub(super) fn group_by_class(module: &Module) -> (Vec<ClassGroup>, Vec<FuncId>) {
    let mut claimed: HashSet<FuncId> = HashSet::new();
    let mut groups = Vec::new();

    for class in &module.classes {
        let struct_def = module.structs[class.struct_index].clone();
        let methods: Vec<FuncId> = class
            .methods
            .iter()
            .filter(|&&fid| {
                if module.functions.get(fid).is_some() {
                    claimed.insert(fid);
                    true
                } else {
                    false
                }
            })
            .copied()
            .collect();
        groups.push(ClassGroup {
            class_def: class.clone(),
            struct_def,
            methods,
        });
    }

    let free: Vec<FuncId> = module
        .functions
        .keys()
        .filter(|fid| !claimed.contains(fid))
        .collect();

    (groups, free)
}

// ---------------------------------------------------------------------------
// Functions
// ---------------------------------------------------------------------------

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_functions(
    module: &mut Module,
    class_names: &HashMap<String, String>,
    known_classes: &HashSet<String>,
    mutable_global_names: &HashSet<String>,
    lowering_config: &LoweringConfig,
    engine: EngineKind,
    stateful_system_aliases: &BTreeMap<String, String>,
    runtime_config: Option<&RuntimeConfig>,
    unique_static_fields: &HashMap<String, String>,
    debug: &DebugConfig,
    out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), CoreError> {
    let all_ids: Vec<_> = module.functions.keys().collect();
    let closure_fids: Vec<FuncId> = all_ids
        .iter()
        .copied()
        .filter(|&fid| module.functions[fid].method_kind == MethodKind::Closure)
        .collect();
    // Pre-build effective lowering config with GML intrinsic_calls populated from
    // the module so that Op::Call with intrinsic functions is lowered to
    // Expr::SystemCall by the linear emitter. This config is passed to all
    // downstream emit functions so they don't need direct module access.
    let effective_lowering = lowering_config_for_engine(lowering_config, engine, Some(module));
    let effective_lowering_ref: &LoweringConfig = &effective_lowering;
    let closure_bodies =
        compile_closures(&closure_fids, module, effective_lowering_ref, engine, debug);
    let object_ts_names = resolve_object_ts_names(&module.object_names, class_names);
    let name_map: HashMap<String, String> = module
        .object_names
        .iter()
        .zip(object_ts_names.iter())
        .filter(|(raw, ts)| raw != ts)
        .map(|(raw, ts)| (raw.clone(), ts.clone()))
        .collect();
    for id in all_ids {
        if module.functions[id].method_kind != MethodKind::Closure {
            let no_stateful = BTreeSet::new();
            let no_free_fns = HashSet::new();
            emit_function(
                &mut module.functions[id],
                &module.types,
                class_names,
                known_classes,
                mutable_global_names,
                effective_lowering_ref,
                engine,
                &module.sprite_names,
                &object_ts_names,
                &closure_bodies,
                &no_stateful,
                &no_free_fns,
                stateful_system_aliases,
                runtime_config,
                unique_static_fields,
                &name_map,
                debug,
                out,
                diagnostics,
            )?;
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
pub(super) fn emit_function(
    func: &mut Function,
    module_types: &reincarnate_core::entity::PrimaryMap<
        reincarnate_core::ir::TypeId,
        reincarnate_core::ir::module::TypeDecl,
    >,
    class_names: &HashMap<String, String>,
    known_classes: &HashSet<String>,
    mutable_global_names: &HashSet<String>,
    lowering_config: &LoweringConfig,
    engine: EngineKind,
    sprite_names: &[String],
    object_names: &[String],
    closure_bodies: &HashMap<String, JsFunction>,
    stateful_names: &BTreeSet<String>,
    free_func_names: &HashSet<String>,
    stateful_system_aliases: &BTreeMap<String, String>,
    runtime_config: Option<&RuntimeConfig>,
    unique_static_fields: &HashMap<String, String>,
    name_map: &HashMap<String, String>,
    debug: &DebugConfig,
    out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), CoreError> {
    use reincarnate_core::ir::linear;

    if debug.dump_ir && debug.should_dump(&func.name) {
        eprintln!("=== IR: {} ===\n{}\n=== end IR ===\n", func.name, func);
    }

    func.hoist_allocs();
    let shape = structurize::structurize(func);
    let effective_config = lowering_config_for_engine(lowering_config, engine, None);
    let ast = linear::lower_function_linear(
        func,
        &func.name,
        &shape,
        &effective_config,
        debug,
        Some(module_types),
    );
    let ctx = crate::lower::LowerCtx {
        self_param_name: None,
    };
    let mut js_func = crate::lower::lower_function(&ast, &ctx);
    // Resolve Type::Instance(TypeId) → Type::Struct(name) so that all
    // downstream type-processing (ts_type, rename_type_with_map, etc.)
    // only see the string-keyed Struct form.
    crate::types::resolve_js_function_types(&mut js_func, module_types);
    let mut js_func = match engine {
        EngineKind::GameMaker => crate::rewrites::gamemaker::rewrite_gamemaker_function(
            js_func,
            sprite_names,
            object_names,
            closure_bodies,
            None,
            name_map,
        ),
        EngineKind::Flash => {
            let class_type_ids = build_class_type_ids(module_types);
            let rewrite_ctx = crate::rewrites::flash::FlashRewriteCtx {
                class_names: class_names.clone(),
                class_type_ids,
                ancestors: HashSet::new(),
                method_names: HashSet::new(),
                instance_fields: HashSet::new(),
                has_self: false,
                suppress_super: false,
                parent_is_runtime: false,
                is_cinit: false,
                is_constructor: false,
                is_static: false,
                static_fields: HashSet::new(),
                static_method_owners: HashMap::new(),
                static_field_owners: HashMap::new(),
                const_instance_fields: HashSet::new(),
                class_short_name: None,
                bindable_methods: HashSet::new(),
                closure_bodies: HashMap::new(),
                known_classes: known_classes.clone(),
                unique_static_fields: unique_static_fields.clone(),
                activation_var: None,
                activation_slots: std::collections::HashSet::new(),
            };
            crate::rewrites::flash::rewrite_flash_function(js_func, &rewrite_ctx)
        }
        EngineKind::Twine => {
            crate::rewrites::twine::rewrite_twine_function(js_func, closure_bodies)
        }
    };
    // Coerce numeric arguments to boolean at call sites where the signature expects boolean.
    if engine == EngineKind::GameMaker {
        let empty_sigs = BTreeMap::new();
        let func_sigs = runtime_config
            .map(|c| &c.function_signatures)
            .unwrap_or(&empty_sigs);
        crate::rewrites::gamemaker::coerce_bool_args(&mut js_func, func_sigs);
    }
    rewrites::rewrite_global_assignments(&mut js_func.body, mutable_global_names);
    crate::ast_passes::dedup_object_keys(&mut js_func, &func.name, diagnostics);
    crate::ast_passes::recover_switch_statements(&mut js_func.body, &func.name, diagnostics);
    crate::ast_passes::strip_redundant_casts(&mut js_func);
    crate::ast_passes::coalesce_text_calls(&mut js_func.body);
    crate::ast_passes::coalesce_array_strings(&mut js_func.body);
    crate::ast_passes::simplify_boolean_returns(&mut js_func.body);
    crate::ast_passes::hoist_else_after_terminal(&mut js_func.body);
    // For void functions, rewrite `return <expr>;` into `<expr>; return;` (or
    // `return;` for pure values) and strip the trailing bare `return;`.
    if js_func.return_ty == Type::Void {
        crate::ast_passes::strip_void_returns(&mut js_func);
    }
    // Rewrite calls to free functions: prepend `_rt` as first argument.
    // Includes recursive self-calls — do NOT remove self from the set.
    if !free_func_names.is_empty() {
        rewrites::prepend_rt_arg_to_free_calls(&mut js_func.body, free_func_names, false);
    }
    // Rewrite stateful runtime calls: `foo(args)` → `_rt.foo(args)`.
    // Also prepend `_rt` parameter when any stateful names are used.
    if !stateful_names.is_empty() {
        let rt_type_name = runtime_config
            .and_then(|c| c.runtime_type.as_ref())
            .map(|t| t.name.as_str())
            .unwrap_or("GameRuntime");
        // Runtime types are pre-interned by intern_runtime_types() in emit_module_to_string/dir.
        let rt_ty = find_type_id(module_types, rt_type_name)
            .map(Type::Instance)
            .unwrap_or(Type::Unknown);
        js_func.params.insert(0, ("_rt".into(), rt_ty));
        js_func.param_defaults.insert(0, None);
        rewrites::rewrite_stateful_calls(&mut js_func.body, stateful_names, false);
    }
    // Build preamble for Twine stateful system aliases (unrelated to GML _rt.foo pattern).
    let preamble = if !stateful_system_aliases.is_empty() {
        // Twine: alias stateful system modules from `_rt` properties.
        let rt_type_name = runtime_config
            .and_then(|c| c.runtime_type.as_ref())
            .map(|t| t.name.as_str())
            .unwrap_or("SugarCubeRuntime");
        // Runtime types are pre-interned by intern_runtime_types() in emit_module_to_string/dir.
        let rt_ty = find_type_id(module_types, rt_type_name)
            .map(Type::Instance)
            .unwrap_or(Type::Unknown);
        js_func.params.insert(0, ("_rt".into(), rt_ty));
        js_func.param_defaults.insert(0, None);
        // If a context_type is configured, retype the first Unknown param after `_rt`
        // (e.g. `h: any` → `h: HarloweContext`).
        if let Some(ctx_type) = runtime_config.and_then(|c| c.context_type.as_ref()) {
            let ctx_type_name = ctx_type.name.as_str();
            let ctx_ty = find_type_id(module_types, ctx_type_name)
                .map(Type::Instance)
                .unwrap_or(Type::Unknown);
            for (_, ty) in &mut js_func.params {
                if *ty == Type::Unknown {
                    *ty = ctx_ty.clone();
                    break;
                }
            }
        }
        let lines: Vec<String> = stateful_system_aliases
            .iter()
            .map(|(ident, prop)| format!("const {ident} = _rt.{prop};"))
            .collect();
        Some(lines.join("\n  "))
    } else {
        None
    };
    ensure_trailing_unreachable(func, &mut js_func);
    crate::ast_printer::NULL_ASSERT.set(engine == EngineKind::Flash);
    crate::ast_printer::print_function(&js_func, preamble.as_deref(), module_types, out);
    crate::ast_printer::NULL_ASSERT.set(false);
    Ok(())
}

// ---------------------------------------------------------------------------
// Class emission
// ---------------------------------------------------------------------------

/// Emit a TypeScript class from a `ClassGroup`.
#[allow(clippy::too_many_arguments)]
pub(super) fn emit_class(
    group: &ClassGroup,
    module: &mut Module,
    class_names: &HashMap<String, String>,
    class_meta: &ClassMeta,
    mutable_global_names: &HashSet<String>,
    late_bound: &HashSet<String>,
    short_to_qualified: &HashMap<String, String>,
    known_classes: &HashSet<String>,
    lowering_config: &LoweringConfig,
    engine: EngineKind,
    stateful_names: &BTreeSet<String>,
    free_func_names: &HashSet<String>,
    func_sigs: &BTreeMap<String, ExternalMethodSig>,
    debug: &DebugConfig,
    out: &mut String,
    traits_out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
    override_class_name: Option<&str>,
) -> Result<(), CoreError> {
    let qualified = qualified_class_name(&group.class_def);
    // Use the (possibly disambiguated) TypeScript class identifier.
    // `override_class_name` is set when two classes in the same namespace share
    // the same sanitized name — the second one gets a unique suffix assigned by
    // the caller.
    let class_name = override_class_name
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            class_names
                .get(&qualified)
                .cloned()
                .unwrap_or_else(|| sanitize_ident(&group.class_def.name))
        });
    let vis = rewrites::visibility_prefix(group.class_def.visibility);

    let extends = match &group.class_def.super_class {
        Some(sc) => {
            let base = sc.rsplit("::").next().unwrap_or(sc);
            // `extends Object` is redundant in JS — all classes extend Object implicitly.
            if base == "Object" {
                String::new()
            } else {
                // Use disambiguated name for the superclass if it's a duplicate.
                let super_ts = class_names
                    .get(sc.as_str())
                    .cloned()
                    .unwrap_or_else(|| sanitize_ident(base));
                format!(" extends {super_ts}")
            }
        }
        None => String::new(),
    };

    let abstract_kw = if group.class_def.is_interface {
        "abstract "
    } else {
        ""
    };
    let _ = writeln!(out, "{vis}{abstract_kw}class {class_name}{extends} {{");
    if engine == EngineKind::Flash {
        // Add `override` when the immediate in-module parent also declares [QN_KEY].
        // External runtime parents (Proxy, MovieClip, etc.) don't have [QN_KEY].
        let parent_is_in_module = group
            .class_def
            .super_class
            .as_deref()
            .filter(|sc| *sc != "Object")
            .is_some_and(|sc| {
                let base = sc.rsplit("::").next().unwrap_or(sc);
                module.classes.iter().any(|c| c.name == base)
            });
        out.push_str(&crate::emit_flash_traits::emit_flash_class_header(
            &qualified,
            parent_is_in_module,
        ));
    }

    // Hoist parent member set lookup so it's available for field override detection below.
    let empty_set_early: HashSet<String> = HashSet::new();
    let parent_method_names_early = class_meta
        .parent_method_name_sets
        .get(&qualified)
        .unwrap_or(&empty_set_early);

    // Static fields from ClassDef (class-level Slot/Const + promoted instance Consts).
    for f in &group.class_def.static_fields {
        let ident = sanitize_ident(&f.name);
        let mut ts = if engine == EngineKind::Flash {
            flash_ts_type_with_names_and_module(&f.ty, class_names, &module.types)
        } else {
            ts_type_with_names_and_module(&f.ty, class_names, &module.types)
        };
        let ov = if parent_method_names_early.contains(f.name.as_str()) {
            "override "
        } else {
            ""
        };
        // AS3 `const` is not enforced as immutable at runtime — Flash Player
        // allows reassignment of static const fields.  Emitting `readonly`
        // causes TS2540 when game code does reassign them, so we omit it.
        let ro = "";
        if let Some(val) = &f.default {
            if matches!(val, Constant::Null) && engine != EngineKind::Flash {
                widen_type_for_null(&f.ty, &mut ts);
            }
            let val_str = if matches!(val, Constant::Null) && engine == EngineKind::Flash {
                "null!".to_string()
            } else {
                crate::ast_printer::emit_constant(val)
            };
            let _ = writeln!(out, "  static {ov}{ro}{ident}: {ts} = {val_str};");
        } else {
            let _ = writeln!(out, "  static {ov}{ident}: {ts};");
        }
    }

    // Instance fields from struct def.
    for field in &group.struct_def.fields {
        let ident = sanitize_ident(&field.name);
        let mut ts = if engine == EngineKind::Flash {
            flash_ts_type_with_names_and_module(&field.ty, class_names, &module.types)
        } else {
            ts_type_with_names_and_module(&field.ty, class_names, &module.types)
        };
        // A field that shadows a parent-class field/method needs `override`.
        let ov = if parent_method_names_early.contains(field.name.as_str()) {
            "override "
        } else {
            ""
        };
        if let Some(val) = &field.default {
            if matches!(val, Constant::Null) && engine != EngineKind::Flash {
                // Non-Flash: widen type to T | null for strictNullChecks.
                // Flash uses null! instead (see below), avoiding cascading errors.
                widen_type_for_null(&field.ty, &mut ts);
            }
            if let Some(resolved) = resolve_sprite_constant(&field.name, val, &module.sprite_names)
            {
                let _ = writeln!(out, "  {ov}{ident}: {ts} = {resolved};");
            } else {
                // Flash: emit null! so the field type stays non-null (no cascading).
                let val_str = if matches!(val, Constant::Null) && engine == EngineKind::Flash {
                    "null!".to_string()
                } else {
                    crate::ast_printer::emit_constant(val)
                };
                let _ = writeln!(out, "  {ov}{ident}: {ts} = {val_str};");
            }
        } else {
            // AS3 instance fields with no initializer are semantically zero-initialized,
            // but TypeScript's strictPropertyInitialization doesn't know that. Use `!`
            // (definite assignment assertion) to suppress TS2564 for AS3-compiled code.
            let bang = if group.class_def.zero_initialized {
                "!"
            } else {
                ""
            };
            let _ = writeln!(out, "  {ov}{ident}{bang}: {ts};");
        }
    }
    // Index signatures for AS3 `dynamic` classes and Proxy subclasses — these allow
    // arbitrary property access by string or number key.
    // The Flash frontend sets needs_index_signature on ClassDef; the Proxy class itself
    // is excluded because it declares the virtual methods, not the index interface.
    let needs_index_sig = group.class_def.needs_index_signature && group.class_def.name != "Proxy";
    if needs_index_sig {
        let _ = writeln!(out, "  [key: string]: any;");
        let _ = writeln!(out, "  [key: number]: any;");
    }
    // Abstract member declarations for interface classes (getters, setters, methods
    // that have no body in the ABC and are not emitted as real methods).
    for m in &group.class_def.abstract_members {
        emit_abstract_member(m, engine, &module.types, out);
    }
    let has_fields = !group.struct_def.fields.is_empty()
        || !group.class_def.static_fields.is_empty()
        || needs_index_sig
        || !group.class_def.abstract_members.is_empty();
    if has_fields && !group.methods.is_empty() {
        out.push('\n');
    }

    // Methods — sorted: constructor first, then instance, static, getters, setters.
    // For interfaces, skip the constructor (AS3 interfaces have no constructor bodies).
    // Closures are separated and compiled for inlining as arrow functions.
    let mut sorted_methods: Vec<FuncId> = group
        .methods
        .iter()
        .copied()
        .filter(|&fid| {
            let mk = module.functions[fid].method_kind;
            if group.class_def.is_interface && mk == MethodKind::Constructor {
                return false;
            }
            mk != MethodKind::Closure
        })
        .collect();
    let closure_fids: Vec<FuncId> = group
        .methods
        .iter()
        .copied()
        .filter(|&fid| module.functions[fid].method_kind == MethodKind::Closure)
        .collect();
    sorted_methods.sort_by_key(|&fid| match module.functions[fid].method_kind {
        MethodKind::Constructor => 0,
        MethodKind::Instance => 1,
        MethodKind::Getter => 2,
        MethodKind::Setter => 3,
        MethodKind::Static | MethodKind::StaticInit => 4,
        MethodKind::Free => 5,
        MethodKind::Closure => 6,
    });

    let empty_set = HashSet::new();
    let empty_map = HashMap::new();
    let ancestors = class_meta
        .ancestor_sets
        .get(&qualified)
        .unwrap_or(&empty_set);
    let method_names = class_meta
        .method_name_sets
        .get(&qualified)
        .unwrap_or(&empty_set);
    let parent_method_names = class_meta
        .parent_method_name_sets
        .get(&qualified)
        .unwrap_or(&empty_set);
    let instance_fields = class_meta
        .instance_field_sets
        .get(&qualified)
        .unwrap_or(&empty_set);
    let static_method_owners = class_meta
        .static_method_owner_map
        .get(&qualified)
        .unwrap_or(&empty_map);
    let static_field_owners = class_meta
        .static_field_owner_map
        .get(&qualified)
        .unwrap_or(&empty_map);
    let bindable_methods = class_meta
        .bindable_method_sets
        .get(&qualified)
        .unwrap_or(&empty_set);
    let static_fields: HashSet<String> = group
        .class_def
        .static_fields
        .iter()
        .map(|f| f.name.clone())
        .collect();
    // Const static fields with scalar defaults: their cinit assignments are redundant
    // (the default already appears on the declaration).  Only filter fields that
    // actually have a default — const fields without defaults (e.g. CockTypesEnum.HUMAN)
    // need their cinit assignments to survive.
    let const_instance_fields: HashSet<String> = group
        .class_def
        .static_fields
        .iter()
        .filter(|f| f.is_const && f.default.is_some())
        .map(|f| f.name.clone())
        .collect();

    let suppress_super = extends.is_empty();
    // True when the class extends a Flash runtime class (e.g. MovieClip, Font) that is
    // NOT defined in the user module.  The runtime constructor doesn't accept `_shims`,
    // so `constructSuper` must emit `super()` without injecting it.
    let parent_is_runtime = engine == EngineKind::Flash
        && !extends.is_empty()
        && !group
            .class_def
            .super_class
            .as_deref()
            .filter(|sc| *sc != "Object")
            .is_some_and(|sc| {
                let base = sc.rsplit("::").next().unwrap_or(sc);
                module.classes.iter().any(|c| c.name == base)
            });

    // Pre-build effective lowering config with GML intrinsic_calls so that
    // Op::Call with intrinsic functions is lowered to Expr::SystemCall.
    let effective_lowering_for_class =
        lowering_config_for_engine(lowering_config, engine, Some(module));
    let effective_lowering_class_ref: &LoweringConfig = &effective_lowering_for_class;
    // Compile closure bodies for inlining as arrow functions.
    let closure_bodies = compile_closures(
        &closure_fids,
        module,
        effective_lowering_class_ref,
        engine,
        debug,
    );
    let object_ts_names = resolve_object_ts_names(&module.object_names, class_names);
    let name_map: HashMap<String, String> = module
        .object_names
        .iter()
        .zip(object_ts_names.iter())
        .filter(|(raw, ts)| raw != ts)
        .map(|(raw, ts)| (raw.clone(), ts.clone()))
        .collect();

    // Detect getter overrides without matching setter overrides (TS2540 fix).
    // Flash-specific: relies on `get_`/`set_` naming convention.
    let forwarding_setters = if engine == EngineKind::Flash {
        crate::emit_flash_traits::flash_forwarding_setters(
            module,
            &sorted_methods,
            parent_method_names,
        )
    } else {
        Vec::new()
    };

    for (i, &fid) in sorted_methods.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        emit_class_method(
            &mut module.functions[fid],
            &module.types,
            class_names,
            ancestors,
            method_names,
            parent_method_names,
            instance_fields,
            &static_fields,
            static_method_owners,
            static_field_owners,
            suppress_super,
            parent_is_runtime,
            &const_instance_fields,
            &class_name,
            mutable_global_names,
            late_bound,
            short_to_qualified,
            bindable_methods,
            &closure_bodies,
            known_classes,
            &class_meta.unique_static_field_map,
            effective_lowering_class_ref,
            engine,
            &module.sprite_names,
            &object_ts_names,
            stateful_names,
            free_func_names,
            func_sigs,
            &name_map,
            debug,
            out,
            diagnostics,
        )?;
    }

    // Emit forwarding setters for getter overrides without corresponding setter overrides.
    for (prop, ty) in &forwarding_setters {
        let _ = writeln!(
            out,
            "\n  override set {prop}(value: {ty}) {{ super.{prop} = value; }}"
        );
    }

    let _ = writeln!(out, "}}\n");
    // Flash-specific class registration → written to traits_out for companion file split.
    if engine == EngineKind::Flash {
        let _ = writeln!(traits_out, "registerClass({class_name});\n");
        // Skip registerClassTraits for interfaces (they have no runtime traits).
        if !group.class_def.is_interface {
            crate::emit_flash_traits::emit_class_registration(
                group,
                module,
                class_names,
                traits_out,
            );
        }
        // Emit registerInterface for implementing classes.
        if !group.class_def.interfaces.is_empty() {
            let iface_names: Vec<String> = group
                .class_def
                .interfaces
                .iter()
                .map(|name| {
                    let short = name.rsplit("::").next().unwrap_or(name);
                    sanitize_ident(short)
                })
                .collect();
            let _ = writeln!(
                traits_out,
                "registerInterface({class_name}, {});\n",
                iface_names.join(", ")
            );
        }
    }
    Ok(())
}

/// Appends `| null` to a type string when a field has a `null` default initializer,
/// unless the type already accommodates null (Unknown → `any`, Option → `T | null`).
fn widen_type_for_null(ty: &Type, ts: &mut String) {
    if !matches!(ty, Type::Unknown | Type::Option(_)) {
        ts.push_str(" | null");
    }
}

fn emit_abstract_member(
    m: &reincarnate_core::ir::AbstractMember,
    engine: EngineKind,
    module_types: &reincarnate_core::entity::PrimaryMap<
        reincarnate_core::ir::TypeId,
        reincarnate_core::ir::module::TypeDecl,
    >,
    out: &mut String,
) {
    use crate::types::ts_type_with_module;
    let ident = sanitize_ident(&m.name);
    let ret_ts = if engine == EngineKind::Flash {
        // flash_ts_type special-cases Map(Unknown,_) and Array; otherwise falls through.
        // For Instance types, use ts_type_with_module which handles name lookup.
        use crate::types::flash_ts_type;
        match &m.return_ty {
            reincarnate_core::ir::Type::Instance(_) => {
                ts_type_with_module(&m.return_ty, module_types)
            }
            other => flash_ts_type(other),
        }
    } else {
        ts_type_with_module(&m.return_ty, module_types)
    };
    match m.kind {
        MethodKind::Getter => {
            let _ = writeln!(out, "  abstract get {ident}(): {ret_ts};");
        }
        MethodKind::Setter => {
            let param_ts = if let Some(p) = m.params.first() {
                if engine == EngineKind::Flash {
                    use crate::types::flash_ts_type;
                    match p {
                        reincarnate_core::ir::Type::Instance(_) => {
                            ts_type_with_module(p, module_types)
                        }
                        other => flash_ts_type(other),
                    }
                } else {
                    ts_type_with_module(p, module_types)
                }
            } else {
                "any".to_string()
            };
            // TypeScript disallows return type annotation on set accessors (TS1095).
            let _ = writeln!(out, "  abstract set {ident}(v: {param_ts});");
        }
        MethodKind::Instance => {
            let param_strs: Vec<String> = m
                .params
                .iter()
                .enumerate()
                .map(|(i, p)| {
                    let pts = if engine == EngineKind::Flash {
                        use crate::types::flash_ts_type;
                        match p {
                            reincarnate_core::ir::Type::Instance(_) => {
                                ts_type_with_module(p, module_types)
                            }
                            other => flash_ts_type(other),
                        }
                    } else {
                        ts_type_with_module(p, module_types)
                    };
                    format!("p{i}: {pts}")
                })
                .collect();
            let _ = writeln!(
                out,
                "  abstract {ident}({}): {ret_ts};",
                param_strs.join(", ")
            );
        }
        // Abstract members are only Getter, Setter, or Instance — other kinds
        // (Free, Constructor, Static, StaticInit, Closure) don't appear here.
        MethodKind::Free
        | MethodKind::Constructor
        | MethodKind::Static
        | MethodKind::StaticInit
        | MethodKind::Closure => {}
    }
}

/// Compile closure functions into JS AST form for inlining as arrow functions.
///
/// Closures are lowered through the same pipeline (structurize → linear → JS lower)
/// but WITHOUT the flash rewrite pass — that happens when the closure is inlined
/// into its parent method via the `newFunction` SystemCall rewrite.
pub(super) fn compile_closures(
    closure_fids: &[FuncId],
    module: &mut Module,
    lowering_config: &LoweringConfig,
    engine: EngineKind,
    debug: &DebugConfig,
) -> HashMap<String, JsFunction> {
    use reincarnate_core::ir::linear;

    let effective_config = lowering_config_for_engine(lowering_config, engine, Some(module));
    let mut result = HashMap::new();
    for &fid in closure_fids {
        let func = &mut module.functions[fid];
        let short = func
            .name
            .rsplit("::")
            .next()
            .unwrap_or(&func.name)
            .to_string();

        func.hoist_allocs();
        let shape = structurize::structurize(func);
        let ast = linear::lower_function_linear(
            func,
            &func.name,
            &shape,
            &effective_config,
            debug,
            Some(&module.types),
        );

        // Closures: self_param_name = None — the first param is the activation
        // scope, NOT `this`. This prevents the lowering pass from substituting
        // it with JsExpr::This.
        let ctx = crate::lower::LowerCtx {
            self_param_name: None,
        };
        let mut js_func = crate::lower::lower_function(&ast, &ctx);
        crate::types::resolve_js_function_types(&mut js_func, &module.types);
        result.insert(short, js_func);
    }
    result
}

/// Emit a single method inside a class body.
fn emit_class_method(
    func: &mut Function,
    module_types: &reincarnate_core::entity::PrimaryMap<
        reincarnate_core::ir::TypeId,
        reincarnate_core::ir::module::TypeDecl,
    >,
    class_names: &HashMap<String, String>,
    ancestors: &HashSet<String>,
    method_names: &HashSet<String>,
    parent_method_names: &HashSet<String>,
    instance_fields: &HashSet<String>,
    static_fields: &HashSet<String>,
    static_method_owners: &HashMap<String, String>,
    static_field_owners: &HashMap<String, String>,
    suppress_super: bool,
    parent_is_runtime: bool,
    const_instance_fields: &HashSet<String>,
    class_short_name: &str,
    mutable_global_names: &HashSet<String>,
    late_bound: &HashSet<String>,
    short_to_qualified: &HashMap<String, String>,
    bindable_methods: &HashSet<String>,
    closure_bodies: &HashMap<String, JsFunction>,
    known_classes: &HashSet<String>,
    unique_static_fields: &HashMap<String, String>,
    lowering_config: &LoweringConfig,
    engine: EngineKind,
    sprite_names: &[String],
    object_names: &[String],
    stateful_names: &BTreeSet<String>,
    free_func_names: &HashSet<String>,
    func_sigs: &BTreeMap<String, ExternalMethodSig>,
    name_map: &HashMap<String, String>,
    debug: &DebugConfig,
    out: &mut String,
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), CoreError> {
    #![allow(clippy::too_many_arguments)]
    use reincarnate_core::ir::linear;

    let raw_name = func
        .name
        .rsplit("::")
        .next()
        .unwrap_or(&func.name)
        .to_string();

    let skip_self = matches!(
        func.method_kind,
        MethodKind::Constructor
            | MethodKind::Instance
            | MethodKind::Getter
            | MethodKind::Setter
            | MethodKind::Static
            | MethodKind::StaticInit
            | MethodKind::Closure
    );

    if debug.dump_ir && debug.should_dump(&func.name) {
        eprintln!("=== IR: {} ===\n{}\n=== end IR ===\n", func.name, func);
    }

    func.hoist_allocs();
    let shape = structurize::structurize(func);
    let effective_config = lowering_config_for_engine(lowering_config, engine, None);
    let ast = linear::lower_function_linear(
        func,
        &func.name,
        &shape,
        &effective_config,
        debug,
        Some(module_types),
    );

    // Determine self_param_name for `this` substitution.
    let is_cinit = matches!(func.method_kind, MethodKind::StaticInit);
    let self_param_name = if is_cinit {
        ast.params.first().map(|(n, _)| n.clone())
    } else if skip_self && !ast.params.is_empty() {
        Some(ast.params[0].0.clone())
    } else {
        None
    };

    let ctx = crate::lower::LowerCtx { self_param_name };
    let mut js_func = crate::lower::lower_function(&ast, &ctx);
    // Resolve nested Type::Instance(TypeId) inside compound types before downstream processing.
    crate::types::resolve_js_function_types(&mut js_func, module_types);
    let mut js_func = match engine {
        EngineKind::GameMaker => crate::rewrites::gamemaker::rewrite_gamemaker_function(
            js_func,
            sprite_names,
            object_names,
            closure_bodies,
            Some(&raw_name),
            name_map,
        ),
        EngineKind::Flash => {
            let is_constructor = matches!(func.method_kind, MethodKind::Constructor);
            let is_static_method = matches!(func.method_kind, MethodKind::Static);
            let class_type_ids = build_class_type_ids(module_types);
            let rewrite_ctx = crate::rewrites::flash::FlashRewriteCtx {
                class_names: class_names.clone(),
                class_type_ids,
                ancestors: ancestors.clone(),
                method_names: method_names.clone(),
                instance_fields: instance_fields.clone(),
                has_self: true,
                suppress_super,
                parent_is_runtime,
                is_cinit,
                is_constructor,
                is_static: is_static_method,
                static_fields: static_fields.clone(),
                static_method_owners: static_method_owners.clone(),
                static_field_owners: static_field_owners.clone(),
                const_instance_fields: const_instance_fields.clone(),
                class_short_name: Some(class_short_name.to_string()),
                bindable_methods: if is_cinit
                    || matches!(func.method_kind, MethodKind::Static | MethodKind::Closure)
                {
                    HashSet::new()
                } else {
                    bindable_methods.clone()
                },
                closure_bodies: closure_bodies.clone(),
                known_classes: known_classes.clone(),
                unique_static_fields: unique_static_fields.clone(),
                activation_var: None,
                activation_slots: std::collections::HashSet::new(),
            };
            let mut jf = crate::rewrites::flash::rewrite_flash_function(js_func, &rewrite_ctx);
            crate::rewrites::flash::eliminate_dead_activations(&mut jf.body);
            jf
        }
        EngineKind::Twine => {
            crate::rewrites::twine::rewrite_twine_function(js_func, closure_bodies)
        }
    };
    // Coerce numeric arguments to boolean at call sites where the signature expects boolean.
    if engine == EngineKind::GameMaker {
        crate::rewrites::gamemaker::coerce_bool_args(&mut js_func, func_sigs);
    }
    rewrites::rewrite_global_assignments(&mut js_func.body, mutable_global_names);
    rewrites::rewrite_late_bound_types(&mut js_func.body, late_bound, short_to_qualified);
    // Hoist super() to top of constructor body (after rewrite produces SuperCall nodes).
    if engine == EngineKind::Flash && func.method_kind == MethodKind::Constructor {
        crate::rewrites::flash::hoist_super_call(&mut js_func.body, Some(class_short_name));
    }
    // Filter cinit: remove assignments that duplicate static readonly field defaults,
    // and skip emitting entirely if the body is empty after filtering.
    if is_cinit {
        js_func
            .body
            .retain(|stmt| !rewrites::is_redundant_static_assign(stmt, const_instance_fields));
        if js_func.body.is_empty() {
            return Ok(());
        }
    }
    crate::ast_passes::dedup_object_keys(&mut js_func, &func.name, diagnostics);
    crate::ast_passes::recover_switch_statements(&mut js_func.body, &func.name, diagnostics);
    crate::ast_passes::strip_redundant_casts(&mut js_func);
    crate::ast_passes::coalesce_text_calls(&mut js_func.body);
    crate::ast_passes::coalesce_array_strings(&mut js_func.body);
    crate::ast_passes::simplify_boolean_returns(&mut js_func.body);
    crate::ast_passes::hoist_else_after_terminal(&mut js_func.body);
    // Rewrite calls to free functions: prepend `this._rt` as first argument.
    if !free_func_names.is_empty() {
        rewrites::prepend_rt_arg_to_free_calls(&mut js_func.body, free_func_names, true);
    }
    // Rewrite stateful runtime calls: `foo(args)` → `this._rt.foo(args)`.
    if !stateful_names.is_empty()
        && !is_cinit
        && !matches!(
            func.method_kind,
            MethodKind::Static | MethodKind::StaticInit
        )
    {
        rewrites::rewrite_stateful_calls(&mut js_func.body, stateful_names, true);
    }
    let preamble: Option<String> = None;
    // A method needs `override` if a parent class defines a method with the same name.
    // Constructors and cinit blocks are excluded — TypeScript forbids `override` on them.
    // Static methods ARE eligible for override when the parent class has a same-named static.
    //
    // For getters/setters, also check the bare property name — a parent may expose only a
    // getter (recorded as "timeQ") while the child adds a setter ("set_timeQ").  TypeScript
    // requires `override` on the setter because the getter is inherited, so we must check
    // both the prefixed name and the un-prefixed property name.
    let bare_prop_name = raw_name
        .strip_prefix("get_")
        .or_else(|| raw_name.strip_prefix("set_"))
        .map(str::to_string);
    let is_override = !is_cinit
        && !matches!(func.method_kind, MethodKind::Constructor)
        && (parent_method_names.contains(&raw_name)
            || bare_prop_name
                .as_deref()
                .is_some_and(|p| parent_method_names.contains(p)));
    // Flash constructors receive a `_shims: FlashShims` parameter so each game
    // instance carries its own shim set.  Base classes (suppress_super = true) AND
    // classes that extend runtime types (parent_is_runtime = true) use `readonly` to
    // store the value as a field; user-defined-parent derived classes accept a plain
    // param and thread it to `super(_shims, ...)`.
    let flash_ctor_extra_param: Option<String> =
        if engine == EngineKind::Flash && func.method_kind == MethodKind::Constructor {
            Some(crate::emit_flash_traits::flash_ctor_shims_param(
                suppress_super,
                parent_is_runtime,
            ))
        } else {
            None
        };
    // Add `throw new Error("unreachable")` when the function has a non-void return type
    // but TypeScript cannot prove all paths return (e.g. exhaustive switch without default).
    // Silences TS2366 without changing observable behaviour.
    ensure_trailing_unreachable(func, &mut js_func);
    // Flash/AS3: null is valid for any reference type. Under strictNullChecks,
    // bare `null` causes TS2322/TS2345. Enable null! assertion for Flash output.
    crate::ast_printer::NULL_ASSERT.set(engine == EngineKind::Flash);
    crate::ast_printer::print_class_method(
        &js_func,
        &raw_name,
        skip_self,
        preamble.as_deref(),
        is_override,
        flash_ctor_extra_param.as_deref(),
        module_types,
        out,
    );
    crate::ast_printer::NULL_ASSERT.set(false);
    Ok(())
}

/// Append `throw new Error("unreachable")` to a non-void function body if the
/// body does not already have a guaranteed return on all paths.  This silences
/// TS2366 ("Function lacks ending return statement") for exhaustive switch/if
/// trees that TypeScript cannot prove are complete.
///
/// Special case: if the body ends with a Switch whose default body is empty but
/// all case bodies are terminal, inject the throw into the default body instead
/// of after the switch.  This prevents TS7027 ("Unreachable code") when
/// TypeScript can prove the switch is exhaustive from the discriminant type.
fn ensure_trailing_unreachable(func: &Function, js_func: &mut JsFunction) {
    if matches!(func.sig.return_ty, Type::Void) {
        return;
    }
    if crate::ast_printer::ends_with_terminal(&js_func.body) {
        return;
    }
    let unreachable_throw = JsStmt::Throw(JsExpr::New {
        callee: Box::new(JsExpr::Var("Error".into())),
        args: vec![JsExpr::Literal(Constant::String("unreachable".into()))],
    });
    // If the body ends with a switch where all cases are terminal but there's
    // no default, inject the throw as the default body so TS sees the switch
    // itself as exhaustive and doesn't flag code after it as unreachable.
    if let Some(JsStmt::Switch {
        default_body,
        cases,
        ..
    }) = js_func.body.last_mut()
    {
        let all_cases_terminal = cases
            .iter()
            .all(|(_, body)| body.is_empty() || crate::ast_printer::ends_with_terminal(body));
        if default_body.is_empty() && all_cases_terminal {
            default_body.push(unreachable_throw);
            return;
        }
    }
    js_func.body.push(unreachable_throw);
}
