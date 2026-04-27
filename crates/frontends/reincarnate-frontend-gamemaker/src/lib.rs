mod assets;
mod bool_arith_coerce;
mod builtins_generated;
mod call_site_arity_widen;
mod classref_resolve;
mod data;
mod default_arg;
mod gml_constructor_parent;
mod instance_type_flow;
mod logical_op;
pub(crate) mod naming;
mod object;
mod runtime_bodies;
mod translate;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::fs;

use datawin::DataWin;
use reincarnate_core::error::CoreError;
use reincarnate_core::ir::builder::{FunctionBuilder, ModuleBuilder};
use reincarnate_core::ir::func::{
    FuncId, Function, InlineHint, IntrinsicKind, MethodKind, Visibility,
};
use reincarnate_core::ir::module::{FieldDef, Global, Module, StructDef, SystemCallTypeRule};
use reincarnate_core::ir::ty::{FunctionSig, Type, TypeId};
use reincarnate_core::pipeline::{Frontend, FrontendInput, FrontendOutput};
use reincarnate_core::project::EngineOrigin;

use crate::translate::TranslateCtx;

/// GameMaker frontend — translates data.win files into reincarnate IR.
pub struct GameMakerFrontend;

impl Frontend for GameMakerFrontend {
    fn supported_engines(&self) -> &[EngineOrigin] {
        &[EngineOrigin::GameMaker]
    }

    fn extract(&self, input: FrontendInput) -> Result<FrontendOutput, CoreError> {
        let data = fs::read(&input.source)?;
        let dw = DataWin::parse(data).map_err(|e| CoreError::Parse {
            file: input.source.clone(),
            message: e.to_string(),
        })?;

        let parse_err = |e: datawin::Error| CoreError::Parse {
            file: input.source.clone(),
            message: e.to_string(),
        };

        let gen8 = dw.gen8().map_err(parse_err)?;
        let game_name = dw.resolve_string(gen8.name).map_err(|e| CoreError::Parse {
            file: input.source.clone(),
            message: format!("failed to resolve game name: {e}"),
        })?;

        eprintln!("[gamemaker] extracting: {game_name}");

        let code = dw.code().map_err(parse_err)?;
        let func = dw.func().map_err(parse_err)?;
        let scpt = dw.scpt().map_err(parse_err)?;
        let vari = dw.vari().map_err(parse_err)?;
        let objt = dw.objt().map_err(parse_err)?;

        // Build function name lookup: function_id → resolved name.
        let function_names = build_function_names(&dw, func)?;

        // Build variable lookup: variable_id → (name, instance_type).
        let variables = build_variable_table(&dw, vari)?;

        // Build linked-list reference maps for correct name resolution.
        // In GMS2.x (BC >= 17), FUNC first_address points to the Call operand
        // (4 bytes into the instruction), while earlier formats and VARI always
        // use instruction-word addressing. build_func_ref_map normalises to
        // instruction-word addresses so lookups match bytecode_offset + inst.offset.
        let bc_version = dw
            .bytecode_version()
            .unwrap_or(datawin::BytecodeVersion(15));
        let func_ref_map = build_func_ref_map(func, dw.data(), bc_version);
        let vari_ref_map = build_vari_ref_map(vari, dw.data());

        // Build code_locals lookup: code entry name → CodeLocals.
        let code_locals_map = build_code_locals_map(&dw, func)?;

        // Pre-resolve object names for event naming and parent resolution.
        let obj_names = resolve_object_names(&dw, objt)?;

        // Build set of clean script names (for self-injection at call sites).
        let script_names: HashSet<String> = scpt
            .scripts
            .iter()
            .filter_map(|s| {
                dw.resolve_string(s.name)
                    .ok()
                    .map(|n| strip_script_prefix(&n).to_string())
            })
            .collect();

        // Pre-resolve string table once — passed to all translators instead of &DataWin.
        let string_table = resolve_string_table(&dw);

        let mut mb = ModuleBuilder::new(&game_name);

        // Register global variables from VARI.
        register_globals(&dw, vari, &mut mb);

        // Build code-name → index map for GMS2.3+ constructor script lookup.
        let code_name_map = build_code_name_map(&dw, code);

        // Build GMS2.3+ pushref asset name map: (type_tag << 24) | idx → raw GML name.
        let asset_ref_names = build_asset_ref_names(&dw, scpt);

        // === Register all builtins BEFORE translation so FuncIds exist for Op::Call ===

        // Populate extern sigs from the GML builtin signature table.
        // `Type::Unknown` in the generated table means the generator didn't
        // have enough information to determine the type — these are inference
        // gaps, not genuine source-language opacity.  Replace them with fresh
        // type variables so the solver can attempt to infer them from call
        // sites.  Genuinely opaque types (e.g. explicitly typed enum returns)
        // are represented with concrete types in the generated file and are
        // not affected.
        for (name, mut sig, aliases) in builtins_generated::gml_builtins() {
            freshen_unknown_types_in_sig(&mut sig, mb.module_mut());
            let fid = mb.register_runtime(name.to_string(), sig);
            for alias in aliases {
                mb.module_mut().register_alias(*alias, fid);
            }
        }

        // Register GML-specific polymorphic `_any` arithmetic stubs.
        // The GML VM uses `DataType::Variable` for most arithmetic, so the
        // translator emits `add_any` etc. when operand types are not
        // yet known.  `BuiltinOverloadSelect` replaces these with typed variants
        // once HM inference resolves the operand types.  These stubs are not in
        // `register_core_builtins()` because no other frontend needs them.
        register_arithmetic_any_builtins(mb.module_mut());

        // Register GML syscall intrinsics.  Each intrinsic is a typed Op::Call
        // whose IntrinsicKind encodes the (system, method) pair.  The linear
        // lowering pass maps them back to Expr::SystemCall so all downstream
        // rewrite passes see the same patterns as before.
        let rt_type_id = mb.intern_type("GameRuntime");
        let rt_ty = Type::Instance(rt_type_id);
        register_gml_syscall_intrinsics(mb.module_mut(), rt_ty.clone());

        // Generate throw-stubs for extension functions (EXTN chunk).
        // These resolve TS2304 "Cannot find name 'FS_*'" errors.
        // Must run after builtins are registered so the stub bodies can call
        // `extension_stubfunc_real` / `extension_stubfunc_string` by name.
        if let Ok(Some(extn)) = dw.extn() {
            add_extension_stubs(&dw, extn, &mb.existing_function_names(), &mut mb);
        }

        // Register GML compiler-synthesized functions with correct return types before
        // the stub-registration loop.  These appear as Call references in the FUNC
        // chunk, so the loop below would otherwise register them as void stubs.
        //
        // @@NewGMLArray@@(v0, v1, ...) — variadic array literal constructor.
        // Return type is Array(Unknown): element type is not inferrable without
        // HasElement constraints, and Unknown correctly signals an inference gap.
        // A fresh Var would be incorrect here: the same FunctionSig instance is
        // shared across all call sites, so a single Var would unify across ALL
        // array literals and produce wrong cross-call constraints.
        //
        // @@NewGMLObject@@() — anonymous struct constructor.
        // Return type is Instance(GMLObject).
        mb.register_runtime(
            "@@NewGMLArray@@".to_string(),
            FunctionSig {
                params: vec![],
                return_ty: Type::Array(Box::new(Type::Unknown)),
                has_rest_param: true,
                ..Default::default()
            },
        );
        {
            let gml_object_id = mb.intern_type("GMLObject");
            mb.register_runtime(
                "@@NewGMLObject@@".to_string(),
                FunctionSig {
                    params: vec![],
                    return_ty: Type::Instance(gml_object_id),
                    ..Default::default()
                },
            );
        }

        // Pre-register user function stubs so their FuncIds exist before translation.
        // Translators resolve Call opcodes to named functions — all reachable function
        // names must be in the registry before the first Call opcode is translated.
        // Skip names already in the runtime registry — FUNC chunk entries for runtime
        // builtins are call references, not user definitions.  Registering stubs for
        // them would shadow the runtime's typed signatures with void/empty stubs.
        let runtime_reg = mb.runtime_registry().clone();
        let mut sorted_entries: Vec<(&u32, &String)> = function_names.iter().collect();
        sorted_entries.sort_by_key(|(k, _)| **k);
        let user_func_registry: HashMap<String, FuncId> = sorted_entries
            .into_iter()
            .map(|(_, name)| name)
            .filter(|name| !runtime_reg.contains_key(name.as_str()))
            .map(|name| (name.clone(), mb.register_function_stub(name)))
            .collect();

        // Build combined registry: runtime builtins + user function stubs.
        let combined_registry: HashMap<String, FuncId> = mb
            .runtime_registry()
            .iter()
            .map(|(k, v)| (k.clone(), *v))
            .chain(user_func_registry.iter().map(|(k, v)| (k.clone(), *v)))
            .collect();

        // Translate scripts.
        let (script_ok, script_err) = translate_scripts(
            &dw,
            code,
            scpt,
            &code_name_map,
            &function_names,
            &asset_ref_names,
            &variables,
            &func_ref_map,
            &vari_ref_map,
            &code_locals_map,
            &string_table,
            &mut mb,
            &input,
            &obj_names,
            &script_names,
            bc_version,
            &combined_registry,
            &user_func_registry,
            &rt_ty,
        )?;
        eprintln!("[gamemaker] translated {script_ok} scripts ({script_err} errors)");

        // Translate objects → ClassDefs with event handler methods.
        let (obj_ok, obj_err) = object::translate_objects(
            &dw,
            code,
            &function_names,
            &asset_ref_names,
            &variables,
            &func_ref_map,
            &vari_ref_map,
            &code_locals_map,
            &string_table,
            &mut mb,
            &obj_names,
            &script_names,
            bc_version,
            &combined_registry,
            &rt_ty,
        )
        .map_err(|e| CoreError::Translate {
            file: input.source.clone(),
            message: e,
        })?;
        eprintln!(
            "[gamemaker] translated {obj_ok} event handlers ({obj_err} errors) across {} objects",
            obj_names.len()
        );

        // Translate global init scripts (GLOB chunk).
        let glob_count = translate_global_inits(
            &dw,
            code,
            &function_names,
            &asset_ref_names,
            &variables,
            &func_ref_map,
            &vari_ref_map,
            &code_locals_map,
            &string_table,
            &mut mb,
            &obj_names,
            &script_names,
            bc_version,
            &combined_registry,
            &user_func_registry,
            &rt_ty,
        );
        if glob_count > 0 {
            eprintln!("[gamemaker] translated {glob_count} global init scripts");
        }

        // Translate room creation code.
        let (room_count, room_creation_code) = translate_room_creation(
            &dw,
            code,
            &function_names,
            &asset_ref_names,
            &variables,
            &func_ref_map,
            &vari_ref_map,
            &code_locals_map,
            &string_table,
            &mut mb,
            &obj_names,
            &script_names,
            bc_version,
            &combined_registry,
            &user_func_registry,
            &rt_ty,
        );
        if room_count > 0 {
            eprintln!("[gamemaker] translated {room_count} room creation scripts");
        }
        mb.set_room_creation_code(room_creation_code);

        // Translate unreplaced function stubs.
        //
        // Some functions in the FUNC chunk have no corresponding SCPT entry
        // (e.g. `string`, `max`, `min` in Dead Estate).  Their pre-registered
        // stubs were never replaced by `translate_scripts`.  Look for CODE
        // entries named `gml_Script_<name>` and translate those.
        {
            // Pre-intern ClassRef TypeIds for all object names.
            let classref_types: HashMap<String, TypeId> = obj_names
                .iter()
                .filter_map(|name| {
                    if let Type::ClassRef(id) = mb.intern_type_classref(name) {
                        Some((name.clone(), id))
                    } else {
                        None
                    }
                })
                .collect();
            let mut instance_types: HashMap<String, TypeId> = obj_names
                .iter()
                .map(|name| (name.clone(), mb.intern_type(name)))
                .collect();
            let gml_object_id = mb.intern_type("GMLObject");
            ensure_gml_object_struct(&mut mb);
            instance_types.insert("GMLObject".to_string(), gml_object_id);

            let obj_name_set: HashSet<&str> = obj_names.iter().map(String::as_str).collect();
            let mut missing_owners_warned: HashSet<String> = HashSet::new();

            // Collect stub FuncIds: stubs have an empty params list (no self param).
            let stub_funcs: Vec<(String, FuncId)> = user_func_registry
                .iter()
                .filter(|(_, &fid)| mb.module_mut().functions[fid].sig.params.is_empty())
                .map(|(name, &fid)| (name.clone(), fid))
                .collect();

            let mut stub_count = 0usize;
            for (func_name, fid) in &stub_funcs {
                let code_entry_name = format!("gml_Script_{func_name}");
                let code_idx = match code_name_map
                    .get(&code_entry_name)
                    .or_else(|| code_name_map.get(func_name.as_str()))
                {
                    Some(&idx) => idx,
                    None => continue,
                };
                let bytecode = match code.entry_bytecode(code_idx, dw.data()) {
                    Some(bc) => bc,
                    None => continue,
                };
                let code_entry = &code.entries[code_idx];
                let code_name = dw.resolve_string(code_entry.name).unwrap_or_default();
                let locals = code_locals_map.get(&code_name).copied();

                let owner_raw = extract_script_owner(&code_name, &obj_name_set);
                let owner_pascal = owner_raw.map(naming::object_name_to_pascal);
                if let Some(owner) = owner_pascal.as_deref() {
                    if !instance_types.contains_key(owner)
                        && missing_owners_warned.insert(owner.to_string())
                    {
                        eprintln!(
                            "[gamemaker] warn: stub {code_name} references unknown owner class {owner}; falling back to GMLObject"
                        );
                    }
                }
                let class_name = owner_pascal
                    .as_deref()
                    .filter(|o| instance_types.contains_key(*o));

                let ctx = TranslateCtx {
                    function_names: &function_names,
                    asset_ref_names: &asset_ref_names,
                    variables: &variables,
                    func_ref_map: &func_ref_map,
                    vari_ref_map: &vari_ref_map,
                    bytecode_offset: code_entry.bytecode_offset,
                    local_names: &resolve_local_names(locals, dw.data()),
                    string_table: &string_table,
                    has_self: true,
                    has_other: false,
                    arg_count: code_entry.args_count & 0x7FFF,
                    obj_names: &obj_names,
                    class_name,
                    self_object_index: None,
                    ancestor_indices: HashSet::new(),
                    script_names: &script_names,
                    is_event_handler: false,
                    is_with_body: false,
                    with_body_has_return: false,
                    bytecode_version: bc_version,
                    classref_types: &classref_types,
                    instance_types: &instance_types,
                    gml_object_type_id: gml_object_id,
                    registry: &combined_registry,
                    rt_ty: rt_ty.clone(),
                };

                match translate::translate_code_entry(bytecode, func_name, &ctx) {
                    Ok((func, extra_funcs)) => {
                        mb.replace_function(*fid, func);
                        for extra in extra_funcs {
                            if let Some(&efid) = user_func_registry.get(&extra.name) {
                                mb.replace_function(efid, extra);
                            } else {
                                mb.add_function(extra);
                            }
                        }
                        stub_count += 1;
                    }
                    Err(e) => {
                        eprintln!("[gamemaker] error translating stub {func_name}: {e}");
                    }
                }
            }
            if stub_count > 0 {
                eprintln!("[gamemaker] translated {stub_count} unreplaced stubs");
            }
        }

        // Extract assets (textures, audio, icon from sibling .exe if present).
        let source_dir = input.source.parent().unwrap_or(std::path::Path::new("."));
        let mut assets = assets::extract_assets(&dw, source_dir);
        if !assets.assets.is_empty() {
            eprintln!("[gamemaker] extracted {} assets", assets.assets.len());
        }

        // Generate data files (sprites, textures, fonts, rooms, objects).
        data::generate_data_files(&dw, &mut assets, &obj_names);
        eprintln!("[gamemaker] generated data files");

        // Populate sprite names for constant resolution at emit time.
        mb.set_sprite_names(data::extract_sprite_names(&dw));
        // Populate object names for backend rewrite resolution (int → class name).
        mb.set_object_names(obj_names.to_vec());
        // Populate initial room name so the scaffold can emit `initialRoom: Rooms.<name>`.
        if let Some(name) = data::extract_initial_room_name(&dw) {
            mb.set_initial_room_name(name);
        }

        let mut module = mb.build();

        // GML implicitly returns 0.0 from every function even without an
        // explicit `return` statement.  Type inference must not narrow
        // functions with no value-bearing returns to Void, because callers
        // may still use the result.
        module.implicit_return_value = true;

        // Attach IR bodies to closed-form math runtime stubs.  The stubs were
        // registered above by the builtins_generated loop; these bodies replace
        // their empty entry blocks so the IR inliner can expand them later.
        runtime_bodies::register_runtime_bodies(&mut module);

        // Register callback-return system calls for the GML engine.
        // withInstances callbacks hide the real return value from the outer function.
        module
            .callback_return_calls
            .insert(("GameMaker.Instance".into(), "withInstances".into()), ());

        // Register array-like functions: `array_length(arr)` is emitted as
        // `arr.length` in TypeScript, but in the IR it is a Call op, not a
        // GetField.  Core passes use this set to suppress narrowing of the
        // argument to a scalar type, exactly as they would for `.length` access.
        module.array_like_fns.insert("array_length".to_string());

        let obj_names_set: HashSet<String> = obj_names.iter().cloned().collect();

        Ok(FrontendOutput {
            modules: vec![module],
            assets,
            runtime_variant: None,
            frontend_passes: vec![
                Box::new(call_site_arity_widen::CallSiteArityWiden),
                Box::new(default_arg::GmlDefaultArgRecovery),
                Box::new(reincarnate_core::transforms::IntToBoolPromotion),
                Box::new(logical_op::GmlLogicalOpNormalize),
                Box::new(bool_arith_coerce::GmlBoolArithCoerce),
                Box::new(instance_type_flow::GmlInstanceTypeFlow {
                    obj_names: obj_names_set,
                }),
                Box::new(classref_resolve::GmlClassRefResolve),
                Box::new(gml_constructor_parent::GmlConstructorParent),
            ],
        })
    }
}

/// Declare the built-in `GMLObject` struct fields on `mb` if not already present.
///
/// Called after every `mb.intern_type("GMLObject")` site.  The guard prevents
/// duplicate entries when the same `mb` is reused across multiple translation
/// functions (`translate_scripts`, `translate_global_inits`, `translate_room_creation`).
fn ensure_gml_object_struct(mb: &mut ModuleBuilder) {
    if mb.has_struct("GMLObject") {
        return;
    }
    mb.add_struct(StructDef {
        name: "GMLObject".to_string(),
        namespace: Vec::new(),
        fields: vec![
            FieldDef {
                name: "x".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "y".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "xprevious".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "yprevious".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "xstart".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "ystart".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "direction".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "speed".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "hspeed".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "vspeed".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "gravity".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "gravity_direction".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "friction".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "visible".to_string(),
                ty: Type::Bool,
                default: None,
            },
            FieldDef {
                name: "solid".to_string(),
                ty: Type::Bool,
                default: None,
            },
            FieldDef {
                name: "persistent".to_string(),
                ty: Type::Bool,
                default: None,
            },
            FieldDef {
                name: "depth".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "sprite_index".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "image_index".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "image_number".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "image_speed".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "image_xscale".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "image_yscale".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "image_angle".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "image_alpha".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "image_blend".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "mask_index".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "object_index".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "id".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "alarm".to_string(),
                ty: Type::Array(Box::new(Type::Float(64))),
                default: None,
            },
            FieldDef {
                name: "path_index".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "path_position".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "path_speed".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "path_scale".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "path_orientation".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "path_endaction".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "timeline_index".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "timeline_position".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "timeline_speed".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "timeline_running".to_string(),
                ty: Type::Bool,
                default: None,
            },
            FieldDef {
                name: "timeline_loop".to_string(),
                ty: Type::Bool,
                default: None,
            },
            FieldDef {
                name: "bbox_left".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "bbox_right".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "bbox_top".to_string(),
                ty: Type::Float(64),
                default: None,
            },
            FieldDef {
                name: "bbox_bottom".to_string(),
                ty: Type::Float(64),
                default: None,
            },
        ],
        visibility: reincarnate_core::ir::func::Visibility::Public,
    });
}

/// Register typed arithmetic and logic builtins on the module.
///
/// The GML frontend emits `Op::Call { func: "add_f64", ... }` etc.
/// Translate scripts from SCPT chunk.
#[allow(clippy::too_many_arguments)]
fn translate_scripts(
    dw: &DataWin,
    code: &datawin::chunks::code::Code,
    scpt: &datawin::chunks::scpt::Scpt,
    code_name_map: &HashMap<String, usize>,
    function_names: &HashMap<u32, String>,
    asset_ref_names: &HashMap<u32, String>,
    variables: &[(String, i32)],
    func_ref_map: &HashMap<usize, usize>,
    vari_ref_map: &HashMap<usize, usize>,
    code_locals_map: &HashMap<String, &datawin::chunks::func::CodeLocals>,
    string_table: &[String],
    mb: &mut ModuleBuilder,
    input: &FrontendInput,
    obj_names: &[String],
    script_names: &HashSet<String>,
    bc_version: datawin::BytecodeVersion,
    registry: &HashMap<String, FuncId>,
    user_func_registry: &HashMap<String, FuncId>,
    rt_ty: &Type,
) -> Result<(usize, usize), CoreError> {
    let mut translated = 0;
    let mut errors = 0;

    // Pre-intern ClassRef TypeIds for all object names so translators can emit
    // correct Type::ClassRef(TypeId) for Break -11 (GMS2.3+ pushref) instructions.
    let classref_types: HashMap<String, TypeId> = obj_names
        .iter()
        .filter_map(|name| {
            if let Type::ClassRef(id) = mb.intern_type_classref(name) {
                Some((name.clone(), id))
            } else {
                None
            }
        })
        .collect();

    // Pre-intern Instance TypeIds for all object names so translators can type
    // self/with-body parameters as Type::Instance(TypeId).
    let mut instance_types: HashMap<String, TypeId> = obj_names
        .iter()
        .map(|name| (name.clone(), mb.intern_type(name)))
        .collect();
    // GMLObject is the runtime base class for all GML instances; it is not an
    // OBJT entry, so it must be interned explicitly for with-body fallback typing.
    let gml_object_id = mb.intern_type("GMLObject");
    ensure_gml_object_struct(mb);
    instance_types.insert("GMLObject".to_string(), gml_object_id);

    // Borrowed lookup set for extract_script_owner — owner strings extracted
    // from CODE names are matched against real object names here.
    let obj_name_set: HashSet<&str> = obj_names.iter().map(String::as_str).collect();
    let mut missing_owners_warned: HashSet<String> = HashSet::new();

    for script in &scpt.scripts {
        let script_name = dw
            .resolve_string(script.name)
            .map_err(|e| CoreError::Parse {
                file: input.source.clone(),
                message: format!("failed to resolve script name: {e}"),
            })?;

        // In GMS2.3+ native games, constructor/nested-function SCPT entries have
        // code_id with the high bit set (>= 0x80000000).  The lower bits are NOT a
        // valid CODE chunk index.  Look up the CODE entry by canonical name instead.
        let is_constructor = script.code_id & 0x8000_0000 != 0;
        let code_idx = if is_constructor {
            let clean = strip_script_prefix(&script_name);
            let code_name = if clean == script_name {
                // Name has no gml_Script_ prefix — try both forms.
                code_name_map
                    .get(&format!("gml_Script_{clean}"))
                    .or_else(|| code_name_map.get(clean))
                    .copied()
            } else {
                // strip_script_prefix removed the prefix; reconstruct canonical name.
                code_name_map.get(&format!("gml_Script_{clean}")).copied()
            };
            match code_name {
                Some(idx) => idx,
                None => {
                    eprintln!(
                        "[gamemaker] warn: constructor script {script_name} has no CODE entry"
                    );
                    continue;
                }
            }
        } else {
            let idx = script.code_id as usize;
            if idx >= code.entries.len() {
                eprintln!(
                    "[gamemaker] warn: script {script_name} references invalid code entry {idx}"
                );
                continue;
            }
            idx
        };

        let bytecode = match code.entry_bytecode(code_idx, dw.data()) {
            Some(bc) => bc,
            None => {
                eprintln!("[gamemaker] warn: no bytecode for script {script_name}");
                continue;
            }
        };

        let code_entry = &code.entries[code_idx];
        let code_name = dw.resolve_string(code_entry.name).unwrap_or_default();

        // In GMS2.3+ games migrated from GMS1, the SCPT chunk contains both:
        //   1. A legacy entry whose code name starts with "gml_GlobalScript_" — an
        //      empty stub kept for backward compatibility. The 0x8000 bit is set in
        //      args_count for these entries.
        //   2. A modern entry whose code name starts with "gml_Script_" — the real
        //      implementation using GMS2.3+ named-function syntax.
        // Emitting both produces duplicate TypeScript function exports. Skip the
        // legacy stub; the real implementation comes from the gml_Script_ entry.
        if code_name.starts_with("gml_GlobalScript_") {
            continue;
        }

        let clean_name = strip_script_prefix(&script_name);
        let func_name = clean_name.to_string();

        let locals = code_locals_map.get(&code_name).copied();

        // Extract the owning object class from the CODE entry name so lifted
        // nested/event/struct scripts get `self: <OwnerClass>` rather than the
        // `GMLObject` fallback. Top-level scripts stay as None.
        let owner_raw = extract_script_owner(&code_name, &obj_name_set);
        let owner_pascal = owner_raw.map(naming::object_name_to_pascal);
        if let Some(owner) = owner_pascal.as_deref() {
            if !instance_types.contains_key(owner)
                && missing_owners_warned.insert(owner.to_string())
            {
                eprintln!(
                    "[gamemaker] warn: script {code_name} references unknown owner class {owner}; falling back to GMLObject"
                );
            }
        }
        let class_name = owner_pascal
            .as_deref()
            .filter(|o| instance_types.contains_key(*o));

        let ctx = TranslateCtx {
            function_names,
            asset_ref_names,
            variables,
            func_ref_map,
            vari_ref_map,
            bytecode_offset: code_entry.bytecode_offset,
            local_names: &resolve_local_names(locals, dw.data()),
            string_table,
            has_self: true,
            has_other: false,
            arg_count: code_entry.args_count & 0x7FFF,
            obj_names,
            class_name,
            self_object_index: None,
            ancestor_indices: HashSet::new(),
            script_names,
            is_event_handler: false,
            is_with_body: false,
            with_body_has_return: false,
            bytecode_version: bc_version,
            classref_types: &classref_types,
            instance_types: &instance_types,
            gml_object_type_id: gml_object_id,
            registry,
            rt_ty: rt_ty.clone(),
        };

        match translate::translate_code_entry(bytecode, &func_name, &ctx) {
            Ok((mut func, extra_funcs)) => {
                // Tag GMS2.3+ constructors — but skip anonymous/nested ones
                // (names like `___struct___0_*`) whose inferred struct names
                // would be used as TypeScript type annotations.
                if is_constructor && !func_name.starts_with("___struct___") {
                    func.method_kind = MethodKind::Constructor;
                }
                // Use replace_function for pre-registered stubs, add_function for new ones.
                if let Some(&fid) = user_func_registry.get(&func.name) {
                    mb.replace_function(fid, func);
                } else {
                    mb.add_function(func);
                }
                for extra in extra_funcs {
                    if let Some(&fid) = user_func_registry.get(&extra.name) {
                        mb.replace_function(fid, extra);
                    } else {
                        mb.add_function(extra);
                    }
                }
                translated += 1;
            }
            Err(e) => {
                eprintln!("[gamemaker] error translating {clean_name}: {e}");
                errors += 1;
            }
        }
    }

    Ok((translated, errors))
}

/// Translate global init scripts from GLOB chunk.
#[allow(clippy::too_many_arguments)]
fn translate_global_inits(
    dw: &DataWin,
    code: &datawin::chunks::code::Code,
    function_names: &HashMap<u32, String>,
    asset_ref_names: &HashMap<u32, String>,
    variables: &[(String, i32)],
    func_ref_map: &HashMap<usize, usize>,
    vari_ref_map: &HashMap<usize, usize>,
    code_locals_map: &HashMap<String, &datawin::chunks::func::CodeLocals>,
    string_table: &[String],
    mb: &mut ModuleBuilder,
    obj_names: &[String],
    script_names: &HashSet<String>,
    bc_version: datawin::BytecodeVersion,
    registry: &HashMap<String, FuncId>,
    user_func_registry: &HashMap<String, FuncId>,
    rt_ty: &Type,
) -> usize {
    let glob = match dw.glob() {
        Ok(Some(g)) => g,
        _ => return 0,
    };

    // Pre-intern ClassRef TypeIds for all object names.
    let classref_types: HashMap<String, TypeId> = obj_names
        .iter()
        .filter_map(|name| {
            if let Type::ClassRef(id) = mb.intern_type_classref(name) {
                Some((name.clone(), id))
            } else {
                None
            }
        })
        .collect();

    // Pre-intern Instance TypeIds for all object names so translators can type
    // self/with-body parameters as Type::Instance(TypeId).
    let mut instance_types: HashMap<String, TypeId> = obj_names
        .iter()
        .map(|name| (name.clone(), mb.intern_type(name)))
        .collect();
    let gml_object_id = mb.intern_type("GMLObject");
    ensure_gml_object_struct(mb);
    instance_types.insert("GMLObject".to_string(), gml_object_id);

    let mut count = 0;
    for &script_id in &glob.script_ids {
        let code_idx = script_id as usize;
        if code_idx >= code.entries.len() {
            continue;
        }
        let bytecode = match code.entry_bytecode(code_idx, dw.data()) {
            Some(bc) => bc,
            None => continue,
        };
        let code_entry = &code.entries[code_idx];
        let code_name = dw.resolve_string(code_entry.name).unwrap_or_default();
        let clean_name = strip_script_prefix(&code_name);
        let func_name = format!("_globalInit{}", naming::snake_to_pascal(clean_name));
        let locals = code_locals_map.get(&code_name).copied();

        let ctx = TranslateCtx {
            function_names,
            asset_ref_names,
            variables,
            func_ref_map,
            vari_ref_map,
            bytecode_offset: code_entry.bytecode_offset,
            local_names: &resolve_local_names(locals, dw.data()),
            string_table,
            has_self: false,
            has_other: false,
            arg_count: code_entry.args_count & 0x7FFF,
            obj_names,
            class_name: None,
            self_object_index: None,
            ancestor_indices: HashSet::new(),
            script_names,
            is_event_handler: false,
            is_with_body: false,
            with_body_has_return: false,
            bytecode_version: bc_version,
            classref_types: &classref_types,
            instance_types: &instance_types,
            gml_object_type_id: gml_object_id,
            registry,
            rt_ty: rt_ty.clone(),
        };

        if let Ok((func, extra_funcs)) = translate::translate_code_entry(bytecode, &func_name, &ctx)
        {
            mb.add_function(func);
            for extra in extra_funcs {
                if let Some(&fid) = user_func_registry.get(&extra.name) {
                    mb.replace_function(fid, extra);
                } else {
                    mb.add_function(extra);
                }
            }
            count += 1;
        }
    }
    count
}

/// Translate room creation code from ROOM chunk.
///
/// Returns `(count, room_creation_code)` where `room_creation_code` maps
/// room index → function name for rooms that have creation code.
#[allow(clippy::too_many_arguments)]
fn translate_room_creation(
    dw: &DataWin,
    code: &datawin::chunks::code::Code,
    function_names: &HashMap<u32, String>,
    asset_ref_names: &HashMap<u32, String>,
    variables: &[(String, i32)],
    func_ref_map: &HashMap<usize, usize>,
    vari_ref_map: &HashMap<usize, usize>,
    code_locals_map: &HashMap<String, &datawin::chunks::func::CodeLocals>,
    string_table: &[String],
    mb: &mut ModuleBuilder,
    obj_names: &[String],
    script_names: &HashSet<String>,
    bc_version: datawin::BytecodeVersion,
    registry: &HashMap<String, FuncId>,
    user_func_registry: &HashMap<String, FuncId>,
    rt_ty: &Type,
) -> (usize, BTreeMap<usize, String>) {
    let room = match dw.room() {
        Ok(r) => r,
        Err(_) => return (0, BTreeMap::new()),
    };

    // Pre-intern ClassRef TypeIds for all object names.
    let classref_types: HashMap<String, TypeId> = obj_names
        .iter()
        .filter_map(|name| {
            if let Type::ClassRef(id) = mb.intern_type_classref(name) {
                Some((name.clone(), id))
            } else {
                None
            }
        })
        .collect();

    // Pre-intern Instance TypeIds for all object names so translators can type
    // self/with-body parameters as Type::Instance(TypeId).
    let mut instance_types: HashMap<String, TypeId> = obj_names
        .iter()
        .map(|name| (name.clone(), mb.intern_type(name)))
        .collect();
    let gml_object_id = mb.intern_type("GMLObject");
    ensure_gml_object_struct(mb);
    instance_types.insert("GMLObject".to_string(), gml_object_id);

    let mut count = 0;
    let mut creation_code_map = BTreeMap::new();
    for (room_idx, room_entry) in room.rooms.iter().enumerate() {
        if room_entry.creation_code_id < 0 {
            continue;
        }
        let code_idx = room_entry.creation_code_id as usize;
        if code_idx >= code.entries.len() {
            continue;
        }
        let bytecode = match code.entry_bytecode(code_idx, dw.data()) {
            Some(bc) => bc,
            None => continue,
        };
        let code_entry = &code.entries[code_idx];
        let code_name = dw.resolve_string(code_entry.name).unwrap_or_default();
        let room_name = dw
            .resolve_string(room_entry.name)
            .unwrap_or_else(|_| format!("room_{code_idx}"));
        let func_name = format!("room{}Create", naming::room_name_to_pascal(&room_name));
        let locals = code_locals_map.get(&code_name).copied();

        let ctx = TranslateCtx {
            function_names,
            asset_ref_names,
            variables,
            func_ref_map,
            vari_ref_map,
            bytecode_offset: code_entry.bytecode_offset,
            local_names: &resolve_local_names(locals, dw.data()),
            string_table,
            has_self: false,
            has_other: false,
            arg_count: code_entry.args_count & 0x7FFF,
            obj_names,
            class_name: None,
            self_object_index: None,
            ancestor_indices: HashSet::new(),
            script_names,
            is_event_handler: false,
            is_with_body: false,
            with_body_has_return: false,
            bytecode_version: bc_version,
            classref_types: &classref_types,
            instance_types: &instance_types,
            gml_object_type_id: gml_object_id,
            registry,
            rt_ty: rt_ty.clone(),
        };

        if let Ok((func, extra_funcs)) = translate::translate_code_entry(bytecode, &func_name, &ctx)
        {
            mb.add_function(func);
            for extra in extra_funcs {
                if let Some(&fid) = user_func_registry.get(&extra.name) {
                    mb.replace_function(fid, extra);
                } else {
                    mb.add_function(extra);
                }
            }
            creation_code_map.insert(room_idx, func_name);
            count += 1;
        }
    }
    (count, creation_code_map)
}

/// Pre-resolve the STRG string table into a `Vec<String>` indexed by string id.
///
/// This decouples the translator from `DataWin` — callers pass the resulting
/// slice rather than the full `DataWin`, enabling unit tests without real files.
fn resolve_string_table(dw: &DataWin) -> Vec<String> {
    let Ok(table) = dw.strings() else {
        return vec![];
    };
    (0..table.len())
        .map(|i| table.get(i, dw.data()).unwrap_or_default())
        .collect()
}

/// Pre-resolve local variable names from a `CodeLocals` entry.
///
/// `pub(crate)` so `object.rs` can call it without duplicating the logic.
///
/// Returns `(local_index, name)` pairs. Called per code entry so the
/// translator doesn't need raw file bytes.
pub(crate) fn resolve_local_names(
    locals: Option<&datawin::chunks::func::CodeLocals>,
    data: &[u8],
) -> Vec<(u32, String)> {
    let Some(cl) = locals else { return vec![] };
    cl.locals
        .iter()
        .filter_map(|lv| lv.name.resolve(data).ok().map(|n| (lv.index, n)))
        .collect()
}

/// Register global variables from VARI.
fn register_globals(dw: &DataWin, vari: &datawin::chunks::vari::Vari, mb: &mut ModuleBuilder) {
    for entry in &vari.variables {
        // instance_type == -5 means global.
        if entry.instance_type == -5 {
            if let Ok(name) = dw.resolve_string(entry.name) {
                let ty = mb.fresh_var();
                mb.add_global(Global {
                    name,
                    ty,
                    visibility: Visibility::Public,
                    mutable: true,
                    init: None,
                });
            }
        }
    }
}

/// Replace `Type::Unknown` in a [`FunctionSig`] with fresh type variables.
///
/// `Type::Unknown` in `builtins_generated.rs` means the generator did not
/// have enough type information from the GameMaker manual HTML — these are
/// inference gaps.  This function replaces them with `module.fresh_var()`
/// so the constraint solver can attempt to resolve them from call sites.
fn freshen_unknown_types_in_sig(sig: &mut FunctionSig, module: &mut Module) {
    for ty in &mut sig.params {
        if *ty == Type::Unknown {
            *ty = module.fresh_var();
        }
    }
    if sig.return_ty == Type::Unknown {
        sig.return_ty = module.fresh_var();
    }
}

/// Build function_id → resolved name mapping from FUNC entries.
fn build_function_names(
    dw: &DataWin,
    func: &datawin::chunks::func::Func,
) -> Result<HashMap<u32, String>, CoreError> {
    let mut names = HashMap::new();
    for (idx, entry) in func.functions.iter().enumerate() {
        let raw = dw
            .resolve_string(entry.name)
            .unwrap_or_else(|_| format!("func_{idx}"));
        // Strip the gml_Script_/gml_GlobalScript_ prefix so resolved names
        // match the exported identifiers and the script_names lookup set.
        let name = strip_script_prefix(&raw).to_string();
        names.insert(idx as u32, name);
    }
    Ok(names)
}

/// Walk FUNC linked lists to build: absolute_instruction_address → func_entry_index.
///
/// BC < 17: `first_address` points to the Call instruction word; the function_id
/// operand is at `first_address + 4`. The operand's lower 27 bits encode a
/// relative byte offset to the next instruction word occurrence.
///
/// BC >= 17 (GMS2.x): `first_address` points to the operand (4 bytes into the
/// instruction). The operand's lower 27 bits encode the byte offset to the next
/// operand occurrence. We normalise to instruction-word address so keys match
/// the `bytecode_offset + inst.offset` values computed during translation.
pub fn build_func_ref_map(
    func: &datawin::chunks::func::Func,
    data: &[u8],
    bc_version: datawin::BytecodeVersion,
) -> HashMap<usize, usize> {
    let gms2 = bc_version.func_first_address_is_operand();
    let mut map = HashMap::new();
    for (i, entry) in func.functions.iter().enumerate() {
        if entry.first_address < 0 || entry.occurrences == 0 {
            continue;
        }
        let mut addr = entry.first_address as usize;
        for _ in 0..entry.occurrences {
            // Store the instruction-word address as the key.
            let inst_addr = if gms2 { addr.saturating_sub(4) } else { addr };
            map.insert(inst_addr, i);
            // Read next-pointer from the operand bytes.
            let operand_addr = if gms2 { addr } else { addr + 4 };
            if operand_addr + 4 > data.len() {
                break;
            }
            let raw = u32::from_le_bytes(data[operand_addr..operand_addr + 4].try_into().unwrap());
            // Lower 27 bits = additive byte offset to next occurrence's addr.
            let offset = (raw & 0x07FF_FFFF) as usize;
            if offset == 0 {
                break;
            }
            addr += offset;
        }
    }
    map
}

/// Walk VARI linked lists to build: absolute_instruction_address → vari_entry_index.
///
/// `first_address` points to the Push/Pop instruction word; the variable operand
/// is at `first_address + 4`. The operand's lower 27 bits encode a relative
/// offset to the next occurrence: `next_addr = addr + offset`.
pub fn build_vari_ref_map(
    vari: &datawin::chunks::vari::Vari,
    data: &[u8],
) -> HashMap<usize, usize> {
    let mut map = HashMap::new();
    for (i, entry) in vari.variables.iter().enumerate() {
        if entry.first_address < 0 || entry.occurrences == 0 {
            continue;
        }
        let mut addr = entry.first_address as usize;
        for _ in 0..entry.occurrences {
            map.insert(addr, i);
            // The operand (next-pointer) is at addr + 4.
            let operand_addr = addr + 4;
            if operand_addr + 4 > data.len() {
                break;
            }
            let raw = u32::from_le_bytes(data[operand_addr..operand_addr + 4].try_into().unwrap());
            // Lower 27 bits = additive offset to next occurrence.
            let offset = (raw & 0x07FF_FFFF) as usize;
            if offset == 0 {
                break;
            }
            addr += offset;
        }
    }
    map
}

/// Build variable_id → (name, instance_type) from VARI entries.
fn build_variable_table(
    dw: &DataWin,
    vari: &datawin::chunks::vari::Vari,
) -> Result<Vec<(String, i32)>, CoreError> {
    let mut vars = Vec::with_capacity(vari.variables.len());
    for entry in &vari.variables {
        let name = dw
            .resolve_string(entry.name)
            .unwrap_or_else(|_| "???".to_string());
        vars.push((name, entry.instance_type));
    }
    Ok(vars)
}

/// Build code entry name → index mapping.
///
/// In GMS2.3+, SCPT entries for constructor scripts have `code_id` with the
/// high bit set (≥ 0x80000000), meaning the code_id is not a direct CODE index.
/// We look up the CODE entry by name (`gml_Script_<ScriptName>`) instead.
fn build_code_name_map(dw: &DataWin, code: &datawin::chunks::code::Code) -> HashMap<String, usize> {
    let mut map = HashMap::new();
    for (i, entry) in code.entries.iter().enumerate() {
        if let Ok(name) = dw.resolve_string(entry.name) {
            map.insert(name, i);
        }
    }
    map
}

/// Build code entry name → CodeLocals mapping.
fn build_code_locals_map<'a>(
    dw: &DataWin,
    func: &'a datawin::chunks::func::Func,
) -> Result<HashMap<String, &'a datawin::chunks::func::CodeLocals>, CoreError> {
    let mut map = HashMap::new();
    for entry in &func.code_locals {
        let name = dw.resolve_string(entry.name).unwrap_or_default();
        map.insert(name, entry);
    }
    Ok(map)
}

/// Resolve all object names from OBJT, converting to PascalCase.
fn resolve_object_names(
    dw: &DataWin,
    objt: &datawin::chunks::objt::Objt,
) -> Result<Vec<String>, CoreError> {
    let mut names = Vec::with_capacity(objt.objects.len());
    for obj in &objt.objects {
        let raw = dw
            .resolve_string(obj.name)
            .unwrap_or_else(|_| "???".to_string());
        names.push(naming::object_name_to_pascal(&raw));
    }
    Ok(names)
}

/// Strip common GML script prefixes to get a clean function name.
fn strip_script_prefix(name: &str) -> &str {
    name.strip_prefix("gml_GlobalScript_")
        .or_else(|| name.strip_prefix("gml_Script_"))
        .unwrap_or(name)
}

/// Extract the owner object name from a canonical CODE entry name produced by
/// the GameMaker compiler.
///
/// SCPT entries carry no owner field, but the compiler encodes the owning
/// object into the CODE entry name for nested-context scripts:
///
/// - `gml_Script_anon@<n>@gml_Object_<Owner>_<Event>_<n>` (event-scoped anon)
/// - `gml_Script_anon@<n>@anon@<n>@gml_Object_<Owner>_<Event>_<n>` (nested anon)
/// - `gml_Script_anon_<Owner>_<Event>_<n>_<n>` (alternate anon form)
/// - `gml_Script____struct___<n>_gml_Object_<Owner>_<Event>_<n>_<n>`
///   (struct declared in event)
/// - `gml_Script_<Owner>_<MethodName>` (bound-method form, matched only when
///   `<Owner>` appears in the caller's object table)
///
/// Both `_` and `@` are used as part separators inside the CODE name — anon
/// parts are delimited with `@` (e.g. `anon@11762@`), while object/event parts
/// are delimited with `_`.
///
/// Top-level scripts unrelated to an object return `None`.
///
/// The owner substring in the CODE name is the raw GML object identifier
/// (e.g. `oPlayer`). Callers pass an `obj_names` set keyed by whatever form
/// they already have (usually the PascalCase post-normalization name); the
/// helper applies `naming::object_name_to_pascal` internally before lookup so
/// both camelCase and snake_case raw GML names match against the normalized
/// table.
fn extract_script_owner<'a>(code_name: &'a str, obj_names: &HashSet<&str>) -> Option<&'a str> {
    let matches = |candidate: &str| -> bool {
        obj_names.contains(candidate)
            || obj_names.contains(naming::object_name_to_pascal(candidate).as_str())
    };

    // Event-scoped forms: the substring `gml_Object_<Owner>_<Event>_...` is
    // unambiguous. The owner is everything between `gml_Object_` and the next
    // `_<Event>` delimiter — but we don't know where the event name starts, so
    // we rely on the object table to disambiguate. The preceding separator is
    // either `_` (struct-in-event form) or `@` (anon-in-event form).
    if let Some(rest) = code_name
        .split("_gml_Object_")
        .nth(1)
        .or_else(|| code_name.split("@gml_Object_").nth(1))
    {
        // `rest` is `<Owner>_<Event>_<n>...`. Walk underscore boundaries and
        // return the longest prefix that matches a known object name.
        let mut best: Option<&str> = None;
        for (i, _) in rest.match_indices('_') {
            let candidate = &rest[..i];
            if matches(candidate) {
                best = Some(candidate);
            }
        }
        return best;
    }

    // `gml_Script_anon_<Owner>_<Event>_<n>_<n>` — alternate anon form with no
    // `_gml_Object_` segment.
    if let Some(rest) = code_name.strip_prefix("gml_Script_anon_") {
        let mut best: Option<&str> = None;
        for (i, _) in rest.match_indices('_') {
            let candidate = &rest[..i];
            if matches(candidate) {
                best = Some(candidate);
            }
        }
        if best.is_some() {
            return best;
        }
    }

    // `gml_Script_<Owner>_<MethodName>` — only trust this form when `<Owner>`
    // is a known object.
    if let Some(rest) = code_name.strip_prefix("gml_Script_") {
        let mut best: Option<&str> = None;
        for (i, _) in rest.match_indices('_') {
            let candidate = &rest[..i];
            if matches(candidate) {
                best = Some(candidate);
            }
        }
        return best;
    }

    None
}

/// Build the GMS2.3+ pushref asset name map.
///
/// In GMS2.3+, the `Break -11` (pushref) instruction's `extra` field encodes both
/// an asset type tag and an asset index as `(type_tag << 24) | asset_index`.
///
/// The type tag mapping is **version-dependent** (see UndertaleModTool `AdaptAssetType`):
///
/// GM 2024.4+ layout:
///   Type 0  → OBJT objects         Type 1  → SPRT sprites
///   Type 2  → SOND sounds          Type 3  → ROOM rooms
///   Type 4  → PATH paths           Type 5  → SCPT scripts
///   Type 6  → FONT fonts           Type 7  → TMLN timelines
///   Type 8  → SHDR shaders         Type 9  → SEQN sequences
///   Type 10 → AnimCurve            Type 11 → ParticleSystem
///   Type 13 → BGND backgrounds     Type 14 → RoomInstance
///
/// Pre-2024.4 layout:
///   Type 0  → OBJT objects         Type 1  → SPRT sprites
///   Type 2  → SOND sounds          Type 3  → ROOM rooms
///   Type 4  → BGND backgrounds     Type 5  → PATH paths
///   Type 6  → SCPT scripts         Type 7  → FONT fonts
///   Type 8  → TMLN timelines       Type 10 → SHDR shaders
///   Type 11 → SEQN sequences       Type 12 → AnimCurve
///   Type 13 → ParticleSystem       Type 14 → RoomInstance
///
/// The game's IDE version is not trivially extractable from the data file, so
/// both layouts are registered simultaneously for each chunk. Types 0–3 are
/// identical across layouts. For types that shift between layouts, both the
/// old and new type tags are inserted for the same chunk, so games compiled
/// with either layout resolve correctly. Name collisions are harmless in
/// practice (AnimCurve/TMLN/PATH assets are rarely referenced via pushref and
/// have distinct naming conventions from SHDR/FONT/BGND/SEQN).
///
/// Returns a map of `(type_tag << 24) | asset_index → raw GML asset name`.
fn build_asset_ref_names(dw: &DataWin, scpt: &datawin::chunks::scpt::Scpt) -> HashMap<u32, String> {
    let mut map = HashMap::new();

    // Inline helper: insert at (type_tag << 24) | i.
    macro_rules! reg {
        ($type_tag:expr, $i:expr, $name:expr) => {
            map.insert(($type_tag << 24) | $i as u32, $name);
        };
    }

    // Type 0: objects (OBJT). Same in both layouts.
    // Use PascalCase names (same as resolve_object_names) so that GlobalRef
    // identifiers match the emitted TypeScript class names.
    if let Ok(objt) = dw.objt() {
        for (i, entry) in objt.objects.iter().enumerate() {
            if let Ok(name) = dw.resolve_string(entry.name) {
                // type_tag=0: (0 << 24) | i == i
                map.insert(i as u32, naming::object_name_to_pascal(&name));
            }
        }
    }

    // Types 1–3: sprites, sounds, rooms — same in both layouts.
    if let Ok(sprt) = dw.sprt() {
        for (i, e) in sprt.sprites.iter().enumerate() {
            if let Ok(n) = dw.resolve_string(e.name) {
                reg!(1u32, i, n);
            }
        }
    }
    if let Ok(sond) = dw.sond() {
        for (i, e) in sond.sounds.iter().enumerate() {
            if let Ok(n) = dw.resolve_string(e.name) {
                reg!(2u32, i, n);
            }
        }
    }
    if let Ok(room) = dw.room() {
        for (i, e) in room.rooms.iter().enumerate() {
            if let Ok(n) = dw.resolve_string(e.name) {
                reg!(3u32, i, n);
            }
        }
    }

    // BGND: type 4 (pre-2024.4) and type 13 (2024.4+).
    if let Ok(bgnd) = dw.bgnd() {
        for (i, e) in bgnd.backgrounds.iter().enumerate() {
            if let Ok(n) = dw.resolve_string(e.name) {
                reg!(4u32, i, n.clone());
                reg!(13u32, i, n);
            }
        }
    }

    // SCPT: type 5 (2024.4+) and type 6 (pre-2024.4).
    for (i, entry) in scpt.scripts.iter().enumerate() {
        if let Ok(name) = dw.resolve_string(entry.name) {
            let clean = strip_script_prefix(&name).to_string();
            reg!(5u32, i, clean.clone());
            reg!(6u32, i, clean);
        }
    }

    // FONT: type 6 (2024.4+) and type 7 (pre-2024.4).
    if let Ok(font) = dw.font() {
        for (i, e) in font.fonts.iter().enumerate() {
            if let Ok(n) = dw.resolve_string(e.name) {
                reg!(6u32, i, n.clone());
                reg!(7u32, i, n);
            }
        }
    }

    // SHDR: type 8 (2024.4+) and type 10 (pre-2024.4).
    if let Ok(shdr) = dw.shdr() {
        for (i, e) in shdr.shaders.iter().enumerate() {
            if let Ok(n) = dw.resolve_string(e.name) {
                reg!(8u32, i, n.clone());
                reg!(10u32, i, n);
            }
        }
    }

    // SEQN: type 9 (2024.4+) and type 11 (pre-2024.4).
    if let Ok(Some(seqn)) = dw.seqn() {
        for (i, e) in seqn.sequences.iter().enumerate() {
            if let Ok(n) = dw.resolve_string(e.name) {
                reg!(9u32, i, n.clone());
                reg!(11u32, i, n);
            }
        }
    }

    map
}

/// Add throw-stub IR functions for each extension function in the EXTN chunk.
///
/// Extension functions (e.g. `FS_unique_fname`) are called by name in GML bytecode
/// as plain function calls without `self`.  They are NOT in the SCPT chunk, so the
/// translator emits `call "FS_unique_fname"(arg0, arg1)` with no implicit self arg.
/// Without a declaration, the TypeScript emitter emits the call but no function body,
/// causing TS2304 "Cannot find name" errors.
///
/// This function creates a stub IR function for each extension function that doesn't
/// already exist in the module.  The stub body calls `extension_stubfunc_real()` or
/// `extension_stubfunc_string()` (already in the runtime) which throw at runtime.
///
/// At emit time:
/// - The stub uses a stateful call → `_rt: GameRuntime` is prepended to its params.
/// - Existing call sites get `_rt` prepended by `prepend_rt_arg_to_free_calls`.
/// - The call site `FS_unique_fname(arg0, arg1)` becomes `FS_unique_fname(_rt, arg0, arg1)`.
fn add_extension_stubs(
    dw: &DataWin,
    extn: &datawin::chunks::extn::Extn,
    existing_names: &HashSet<String>,
    mb: &mut ModuleBuilder,
) {
    use datawin::chunks::extn::ExtArgType;

    for ext_fn in extn.all_functions() {
        let name = match dw.resolve_string(ext_fn.name) {
            Ok(n) if !n.is_empty() => n,
            _ => continue,
        };

        // Skip if a function with this name already exists (e.g. a GML script
        // that wraps the extension call).
        if existing_names.contains(&name) {
            continue;
        }

        let (stub_call, ret_ty) = match ext_fn.return_type {
            ExtArgType::String => ("extension_stubfunc_string", Type::String),
            ExtArgType::Real => ("extension_stubfunc_real", Type::Float(64)),
        };

        // Build IR params matching the extension function's arity.
        let param_tys: Vec<Type> = ext_fn
            .args
            .iter()
            .map(|a| match a {
                ExtArgType::String => Type::String,
                ExtArgType::Real => Type::Float(64),
            })
            .collect();

        let sig = FunctionSig {
            params: param_tys,
            return_ty: ret_ty.clone(),
            ..Default::default()
        };

        let mut fb = FunctionBuilder::new(&name, sig, Visibility::Public);
        fb.set_registry(mb.runtime_registry().clone());
        // Call the runtime throw-stub — causes `_rt` to be injected as first param at emit.
        let result = fb.call_named(stub_call, &[], ret_ty);
        fb.ret(Some(result));

        mb.add_function(fb.build());
    }
}

/// Register all GML syscall intrinsics on the module.
///
/// Each intrinsic is assigned an [`IntrinsicKind`] so the linear lowering pass
/// can emit it as `Expr::SystemCall { system, method, args }` rather than a
/// free-function call.  The type rules are attached to the function so the
/// constraint collector can handle `Op::Call` for these exactly as it did for
/// the equivalent `Op::SystemCall` ops.
///
/// Signatures use empty param lists to avoid adding new sig-based constraints
/// (the type rules handle all necessary inference).
pub(crate) fn register_gml_syscall_intrinsics(module: &mut Module, rt_ty: Type) {
    // Getter sig: _rt as param 0 (explicit runtime handle), unknown return type.
    let getter = FunctionSig {
        params: vec![rt_ty.clone()],
        return_ty: Type::Unknown,
        ..Default::default()
    };
    // Void setter sig: _rt as param 0.
    let setter = FunctionSig {
        params: vec![rt_ty.clone()],
        ..Default::default()
    };

    // GameMaker.Instance field accessors.
    module.register_runtime_intrinsic(
        "GameMaker.Instance.getField",
        getter.clone(),
        IntrinsicKind::GameMakerGetField,
        Some(SystemCallTypeRule::ResolveInstanceField),
    );
    module.register_runtime_intrinsic(
        "GameMaker.Instance.setField",
        setter.clone(),
        IntrinsicKind::GameMakerSetField,
        None,
    );
    // GameMaker.Instance cross-object field accessors.
    module.register_runtime_intrinsic(
        "GameMaker.Instance.getOn",
        getter.clone(),
        IntrinsicKind::GameMakerGetOn,
        Some(SystemCallTypeRule::ResolveInstanceField),
    );
    module.register_runtime_intrinsic(
        "GameMaker.Instance.setOn",
        setter.clone(),
        IntrinsicKind::GameMakerSetOn,
        None,
    );
    // GameMaker.Instance other/all accessors.
    module.register_runtime_intrinsic(
        "GameMaker.Instance.getOther",
        getter.clone(),
        IntrinsicKind::GameMakerGetOther,
        None,
    );
    module.register_runtime_intrinsic(
        "GameMaker.Instance.setOther",
        setter.clone(),
        IntrinsicKind::GameMakerSetOther,
        None,
    );
    module.register_runtime_intrinsic(
        "GameMaker.Instance.getAll",
        getter.clone(),
        IntrinsicKind::GameMakerGetAll,
        None,
    );
    module.register_runtime_intrinsic(
        "GameMaker.Instance.setAll",
        setter.clone(),
        IntrinsicKind::GameMakerSetAll,
        None,
    );
    // GameMaker.Instance.withInstances — callback return type varies.
    module.register_runtime_intrinsic(
        "GameMaker.Instance.withInstances",
        getter.clone(),
        IntrinsicKind::GameMakerWithInstances,
        None,
    );
    // getInstances — returns snapshot of all active instances. Registered under the bare
    // name so the stateful-call rewrite emits `this._rt.getInstances()` directly.
    module.register_runtime("getInstances", getter.clone());
    // getInstancesOf — returns snapshot of all active instances of a given class.
    // Takes 1 param (the class reference) and returns an array of instances.
    module.register_runtime(
        "getInstancesOf",
        FunctionSig {
            params: vec![Type::Unknown],
            return_ty: Type::Unknown,
            ..Default::default()
        },
    );
    // GameMaker.Global get/set.
    module.register_runtime_intrinsic(
        "GameMaker.Global.get",
        getter.clone(),
        IntrinsicKind::GameMakerGlobalGet,
        Some(SystemCallTypeRule::ResolveGlobalType),
    );
    module.register_runtime_intrinsic(
        "GameMaker.Global.set",
        setter.clone(),
        IntrinsicKind::GameMakerGlobalSet,
        Some(SystemCallTypeRule::GlobalStore {
            name_arg: 1,
            value_arg: 2,
        }),
    );
    // GameMaker.Argument.get.
    module.register_runtime_intrinsic(
        "GameMaker.Argument.get",
        getter.clone(),
        IntrinsicKind::GameMakerArgumentGet,
        None,
    );
    // GameMaker.Debug.break.
    module.register_runtime_intrinsic(
        "GameMaker.Debug.break",
        setter,
        IntrinsicKind::GameMakerDebugBreak,
        None,
    );
}

/// Return the short suffix string used in builtin names for a given type.
///
/// Used to build names like `"add_f64"` or `"neg_i32"`.
///
/// # Panics
/// Panics if `ty` is not one of the scalar types used by arithmetic builtins.
fn type_suffix(ty: &Type) -> &'static str {
    match ty {
        Type::Float(64) => "f64",
        Type::Float(32) => "f32",
        Type::Int(32) => "i32",
        Type::Int(64) => "i64",
        Type::Bool => "bool",
        Type::String => "str",
        other => panic!("type_suffix: unsupported type {other:?}"),
    }
}

/// Register polymorphic `_any` arithmetic builtins and their specialization tables.
///
/// These stubs are used by the GML frontend when an arithmetic operand type is
/// not yet known at translation time (GML `DataType::Variable`).
/// `BuiltinOverloadSelect` replaces each `xxx_any` call with the
/// appropriately-typed variant (`_f64`, `_f32`, `_i32`, `_i64`) once HM
/// inference has resolved the operand types.
///
/// The typed variants (`add_f64`, etc.) must already be present in
/// `module.runtime_registry` — i.e., this must be called after `Module::new()`
/// (which calls `register_core_builtins()`).
fn register_arithmetic_any_builtins(module: &mut Module) {
    let scalar_types = [
        Type::Float(64),
        Type::Float(32),
        Type::Int(32),
        Type::Int(64),
    ];

    let bin_any = FunctionSig {
        params: vec![Type::Unknown, Type::Unknown],
        return_ty: Type::Unknown,
        ..Default::default()
    };
    let un_any = FunctionSig {
        params: vec![Type::Unknown],
        return_ty: Type::Unknown,
        ..Default::default()
    };

    for op in &["add", "sub", "mul", "div", "rem"] {
        let mut specs: HashMap<Vec<Type>, FuncId> = scalar_types
            .iter()
            .map(|ty| {
                let suffix = type_suffix(ty);
                let fid = module.runtime_registry[&format!("{op}_{suffix}")];
                (vec![ty.clone(), ty.clone()], fid)
            })
            .collect();
        if *op == "add" {
            let concat_id = module.runtime_registry["concat_str"];
            specs.insert(vec![Type::String, Type::String], concat_id);
        }
        let func_name = format!("{op}_any");

        // Register the stub with Unknown sig.
        let any_id = module.register_runtime(&func_name, bin_any.clone());

        // Build dispatch chain for binary op: check each specialization type pair.
        // Priority: Float(64) > Float(32) > Int(32) > Int(64), then String for add.
        let mut dispatch_types: Vec<Type> = scalar_types.to_vec();
        if *op == "add" {
            // Insert String after Float(64) — numeric addition is more common.
            dispatch_types.insert(1, Type::String);
        }
        let built = build_binary_any_dispatch(
            &func_name,
            &bin_any,
            &dispatch_types,
            op,
            &module.runtime_registry,
        );
        module.functions[any_id].blocks = built.blocks;
        module.functions[any_id].insts = built.insts;
        module.functions[any_id].value_types = built.value_types;
        module.functions[any_id].entry = built.entry;
        module.functions[any_id].specializations = specs;
        module.functions[any_id].inline_hint = InlineHint::Always;
    }

    {
        let specs: HashMap<Vec<Type>, FuncId> = scalar_types
            .iter()
            .map(|ty| {
                let suffix = type_suffix(ty);
                let fid = module.runtime_registry[&format!("neg_{suffix}")];
                (vec![ty.clone()], fid)
            })
            .collect();
        let func_name = "neg_any";

        // Register the stub with Unknown sig.
        let any_id = module.register_runtime(func_name, un_any.clone());

        // Build dispatch chain for unary neg: check each specialization type.
        let dispatch_types: Vec<Type> = scalar_types.to_vec();
        let built = build_unary_any_dispatch(
            func_name,
            &un_any,
            &dispatch_types,
            &module.runtime_registry,
        );
        module.functions[any_id].blocks = built.blocks;
        module.functions[any_id].insts = built.insts;
        module.functions[any_id].value_types = built.value_types;
        module.functions[any_id].entry = built.entry;
        module.functions[any_id].specializations = specs;
        module.functions[any_id].inline_hint = InlineHint::Always;
    }
}

/// Build an IR dispatch body for a binary `_any` builtin (e.g. `add_any(a, b)`).
///
/// Produces a chain of nested `br_if` blocks that check `TypeCheck(a, ty)`,
/// then `TypeCheck(b, ty)`, coerces both arguments, calls the typed variant,
/// and returns the result.  Falls through to the next type on mismatch.
/// The final fallback returns `Return(None)`.
fn build_binary_any_dispatch(
    func_name: &str,
    sig: &FunctionSig,
    dispatch_types: &[Type],
    op: &str,
    registry: &HashMap<String, FuncId>,
) -> Function {
    let mut fb = FunctionBuilder::new(func_name, sig.clone(), Visibility::Public);
    fb.set_registry(registry.clone());
    let a = fb.param(0);
    let b = fb.param(1);

    // For each dispatch type, build: check_a -> check_b -> call -> return
    // On failure, fall through to the next type's check_a block.
    let fallback_block = fb.create_block();

    let mut next_else_block = fallback_block;

    // Build in reverse so we can set `next_else_block` correctly.
    for ty in dispatch_types.iter().rev() {
        let suffix = type_suffix(ty);
        let variant_name = if *ty == Type::String {
            "concat_str".to_string()
        } else {
            format!("{op}_{suffix}")
        };

        // Create blocks for this type's dispatch.
        let check_b_block = fb.create_block();
        let call_block = fb.create_block();
        let check_a_block = fb.create_block();

        // check_a_block: TypeCheck(a, ty) -> br_if to check_b or next
        fb.switch_to_block(check_a_block);
        let check_a = fb.type_check(a, ty.clone());
        fb.br_if(check_a, check_b_block, &[], next_else_block, &[]);

        // check_b_block: TypeCheck(b, ty) -> br_if to call or next
        fb.switch_to_block(check_b_block);
        let check_b = fb.type_check(b, ty.clone());
        fb.br_if(check_b, call_block, &[], next_else_block, &[]);

        // call_block: coerce, call, return
        fb.switch_to_block(call_block);
        let a_coerced = fb.coerce(a, ty.clone());
        let b_coerced = fb.coerce(b, ty.clone());
        let result = fb.call_named(&variant_name, &[a_coerced, b_coerced], ty.clone());
        fb.ret(Some(result));

        next_else_block = check_a_block;
    }

    // Entry block: branch to the first type check.
    let entry = fb.entry_block();
    fb.switch_to_block(entry);
    fb.br(next_else_block, &[]);

    // Fallback block: return None (unreachable in practice).
    fb.switch_to_block(fallback_block);
    fb.ret(None);

    fb.build()
}

/// Build an IR dispatch body for a unary `_any` builtin (e.g. `neg_any(a)`).
///
/// Same structure as [`build_binary_any_dispatch`] but only checks one argument.
fn build_unary_any_dispatch(
    func_name: &str,
    sig: &FunctionSig,
    dispatch_types: &[Type],
    registry: &HashMap<String, FuncId>,
) -> Function {
    let mut fb = FunctionBuilder::new(func_name, sig.clone(), Visibility::Public);
    fb.set_registry(registry.clone());
    let a = fb.param(0);

    let fallback_block = fb.create_block();
    let mut next_else_block = fallback_block;

    // Build in reverse.
    for ty in dispatch_types.iter().rev() {
        let suffix = type_suffix(ty);
        let variant_name = format!("neg_{suffix}");

        let call_block = fb.create_block();
        let check_block = fb.create_block();

        // check_block: TypeCheck(a, ty) -> br_if to call or next
        fb.switch_to_block(check_block);
        let check = fb.type_check(a, ty.clone());
        fb.br_if(check, call_block, &[], next_else_block, &[]);

        // call_block: coerce, call, return
        fb.switch_to_block(call_block);
        let a_coerced = fb.coerce(a, ty.clone());
        let result = fb.call_named(&variant_name, &[a_coerced], ty.clone());
        fb.ret(Some(result));

        next_else_block = check_block;
    }

    // Entry block: branch to first type check.
    let entry = fb.entry_block();
    fb.switch_to_block(entry);
    fb.br(next_else_block, &[]);

    // Fallback block: return None.
    fb.switch_to_block(fallback_block);
    fb.ret(None);

    fb.build()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn objs<'a>(names: &'a [&'a str]) -> HashSet<&'a str> {
        names.iter().copied().collect()
    }

    #[test]
    fn extract_owner_event_scoped_anon() {
        let set = objs(&["oAnyaGalleryPaintingPuzzle"]);
        let name = "gml_Script_anon_11762_gml_Object_oAnyaGalleryPaintingPuzzle_Step_0";
        assert_eq!(
            extract_script_owner(name, &set),
            Some("oAnyaGalleryPaintingPuzzle")
        );
    }

    #[test]
    fn extract_owner_alternate_anon() {
        let set = objs(&["oPlayer"]);
        let name = "gml_Script_anon_oPlayer_Step_0_42";
        assert_eq!(extract_script_owner(name, &set), Some("oPlayer"));
    }

    #[test]
    fn extract_owner_struct_in_event() {
        let set = objs(&["oEnemy"]);
        let name = "gml_Script____struct___3_gml_Object_oEnemy_Create_0_0";
        assert_eq!(extract_script_owner(name, &set), Some("oEnemy"));
    }

    #[test]
    fn extract_owner_bound_method() {
        let set = objs(&["oPlayer"]);
        let name = "gml_Script_oPlayer_doThing";
        assert_eq!(extract_script_owner(name, &set), Some("oPlayer"));
    }

    #[test]
    fn extract_owner_anon_at_separator() {
        // Canonical GMS2.3+ form: anon parts separated by `@`, object/event by `_`.
        let set = objs(&["OAnyaGalleryPaintingPuzzle"]);
        let name = "gml_Script_anon@11762@gml_Object_oAnyaGalleryPaintingPuzzle_Step_0";
        assert_eq!(
            extract_script_owner(name, &set),
            Some("oAnyaGalleryPaintingPuzzle")
        );
    }

    #[test]
    fn extract_owner_nested_anon_at_separator() {
        let set = objs(&["OEnemy"]);
        let name = "gml_Script_anon@4040@anon@3413@gml_Object_oEnemy_Step_0";
        assert_eq!(extract_script_owner(name, &set), Some("oEnemy"));
    }

    #[test]
    fn extract_owner_top_level_script_returns_none() {
        let set = objs(&["oPlayer"]);
        let name = "gml_Script_scr_global_helper";
        assert_eq!(extract_script_owner(name, &set), None);
    }
}
