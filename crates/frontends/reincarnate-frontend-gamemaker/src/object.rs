use std::collections::HashMap;

use datawin::chunks::objt::{event_type, ObjectEntry};
use datawin::DataWin;
use reincarnate_core::ir::builder::ModuleBuilder;
use reincarnate_core::ir::func::{MethodKind, Visibility};
use reincarnate_core::ir::module::{ClassDef, StructDef};

use crate::translate::{self, TranslateCtx};

/// Translate all objects from the OBJT chunk into ClassDefs.
pub fn translate_objects(
    dw: &DataWin,
    code: &datawin::chunks::code::Code,
    function_names: &HashMap<u32, String>,
    variables: &[(String, i32)],
    code_locals_map: &HashMap<String, &datawin::chunks::func::CodeLocals>,
    mb: &mut ModuleBuilder,
    obj_names: &[String],
) -> Result<(usize, usize), String> {
    let objt = dw.objt().map_err(|e| e.to_string())?;
    let mut translated = 0;
    let mut errors = 0;

    for (obj_idx, obj) in objt.objects.iter().enumerate() {
        let obj_name = &obj_names[obj_idx];

        // Create empty StructDef for instance fields.
        // GML is dynamically typed — fields populated from VARI in lib.rs.
        let struct_index = mb.struct_count();
        mb.add_struct(StructDef {
            name: obj_name.clone(),
            namespace: Vec::new(),
            fields: Vec::new(),
            visibility: Visibility::Public,
        });

        let mut method_ids = Vec::new();

        // Translate event handlers.
        for (event_type_idx, event_entries) in obj.events.iter().enumerate() {
            for event in event_entries {
                for action in &event.actions {
                    let code_idx = action.code_id as usize;
                    if code_idx >= code.entries.len() {
                        continue;
                    }
                    let bytecode = match code.entry_bytecode(code_idx, dw.data()) {
                        Some(bc) => bc,
                        None => continue,
                    };

                    let event_name = make_event_name(
                        event_type_idx,
                        event.subtype,
                        obj_names,
                    );
                    let func_name = format!("{obj_name}::{event_name}");

                    let code_entry = &code.entries[code_idx];
                    let code_name = dw.resolve_string(code_entry.name).unwrap_or_default();
                    let locals = code_locals_map.get(&code_name).copied();

                    let is_collision = event_type_idx == event_type::COLLISION;

                    let ctx = TranslateCtx {
                        dw,
                        function_names,
                        variables,
                        locals,
                        has_self: true,
                        has_other: is_collision,
                        arg_count: code_entry.args_count & 0x7FFF,
                    };

                    match translate::translate_code_entry(bytecode, &func_name, &ctx) {
                        Ok(mut func) => {
                            func.namespace = Vec::new();
                            func.class = Some(obj_name.clone());
                            func.method_kind = if event_name == "create" {
                                MethodKind::Constructor
                            } else {
                                MethodKind::Instance
                            };
                            let fid = mb.add_function(func);
                            method_ids.push(fid);
                            translated += 1;
                        }
                        Err(e) => {
                            eprintln!("[gamemaker] error translating {func_name}: {e}");
                            errors += 1;
                        }
                    }
                }
            }
        }

        // Resolve parent object.
        let super_class = resolve_parent(obj, obj_names);

        mb.add_class(ClassDef {
            name: obj_name.clone(),
            namespace: Vec::new(),
            struct_index,
            methods: method_ids,
            super_class,
            visibility: Visibility::Public,
            static_fields: Vec::new(),
            is_interface: false,
            interfaces: Vec::new(),
        });
    }

    Ok((translated, errors))
}

/// Resolve parent object index to a name.
fn resolve_parent(obj: &ObjectEntry, obj_names: &[String]) -> Option<String> {
    // parent_index < 0 means no parent (-100 in most GameMaker versions).
    if obj.parent_index < 0 {
        return None;
    }
    let idx = obj.parent_index as usize;
    obj_names.get(idx).cloned()
}

/// Produce a human-readable event handler name.
fn make_event_name(
    event_type_idx: usize,
    subtype: u32,
    obj_names: &[String],
) -> String {
    match event_type_idx {
        event_type::CREATE => "create".to_string(),
        event_type::DESTROY => "destroy".to_string(),
        event_type::ALARM => format!("alarm_{subtype}"),
        event_type::STEP => match subtype {
            0 => "step".to_string(),
            1 => "step_begin".to_string(),
            2 => "step_end".to_string(),
            _ => format!("step_{subtype}"),
        },
        event_type::COLLISION => {
            let other = obj_names
                .get(subtype as usize)
                .cloned()
                .unwrap_or_else(|| format!("obj_{subtype}"));
            format!("collision_{other}")
        }
        event_type::KEYBOARD => {
            let key = key_name(subtype);
            format!("keyboard_{key}")
        }
        event_type::MOUSE => {
            let name = mouse_event_name(subtype);
            format!("mouse_{name}")
        }
        event_type::OTHER => {
            let name = other_event_name(subtype);
            format!("other_{name}")
        }
        event_type::DRAW => match subtype {
            0 => "draw".to_string(),
            64 => "draw_gui".to_string(),
            65 => "draw_resize".to_string(),
            72 => "draw_begin".to_string(),
            73 => "draw_end".to_string(),
            74 => "draw_gui_begin".to_string(),
            75 => "draw_gui_end".to_string(),
            76 => "draw_pre".to_string(),
            77 => "draw_post".to_string(),
            _ => format!("draw_{subtype}"),
        },
        event_type::KEY_PRESS => {
            let key = key_name(subtype);
            format!("keypress_{key}")
        }
        event_type::KEY_RELEASE => {
            let key = key_name(subtype);
            format!("keyrelease_{key}")
        }
        event_type::TRIGGER => format!("trigger_{subtype}"),
        _ => format!("event_{event_type_idx}_{subtype}"),
    }
}

/// Map a virtual key code to a readable name.
fn key_name(vk: u32) -> String {
    match vk {
        0 => "nokey".to_string(),
        1 => "anykey".to_string(),
        8 => "backspace".to_string(),
        9 => "tab".to_string(),
        13 => "enter".to_string(),
        16 => "shift".to_string(),
        17 => "ctrl".to_string(),
        18 => "alt".to_string(),
        27 => "escape".to_string(),
        32 => "space".to_string(),
        33 => "pageup".to_string(),
        34 => "pagedown".to_string(),
        35 => "end".to_string(),
        36 => "home".to_string(),
        37 => "left".to_string(),
        38 => "up".to_string(),
        39 => "right".to_string(),
        40 => "down".to_string(),
        45 => "insert".to_string(),
        46 => "delete".to_string(),
        48..=57 => format!("{}", (vk - 48)),
        65..=90 => format!("{}", (vk as u8 as char).to_ascii_lowercase()),
        96..=105 => format!("numpad{}", vk - 96),
        112..=123 => format!("f{}", vk - 111),
        _ => format!("vk_{vk}"),
    }
}

/// Map mouse event subtypes to names.
fn mouse_event_name(subtype: u32) -> String {
    match subtype {
        0 => "left_button".to_string(),
        1 => "right_button".to_string(),
        2 => "middle_button".to_string(),
        3 => "no_button".to_string(),
        4 => "left_pressed".to_string(),
        5 => "right_pressed".to_string(),
        6 => "middle_pressed".to_string(),
        7 => "left_released".to_string(),
        8 => "right_released".to_string(),
        9 => "middle_released".to_string(),
        10 => "mouse_enter".to_string(),
        11 => "mouse_leave".to_string(),
        60 => "global_left_button".to_string(),
        61 => "global_right_button".to_string(),
        62 => "global_middle_button".to_string(),
        63 => "global_left_pressed".to_string(),
        64 => "global_right_pressed".to_string(),
        65 => "global_middle_pressed".to_string(),
        66 => "global_left_released".to_string(),
        67 => "global_right_released".to_string(),
        68 => "global_middle_released".to_string(),
        _ => format!("{subtype}"),
    }
}

/// Map "Other" event subtypes to names.
fn other_event_name(subtype: u32) -> String {
    match subtype {
        0 => "outside_room".to_string(),
        1 => "intersect_boundary".to_string(),
        2 => "game_start".to_string(),
        3 => "game_end".to_string(),
        4 => "room_start".to_string(),
        5 => "room_end".to_string(),
        6 => "no_more_lives".to_string(),
        7 => "animation_end".to_string(),
        8 => "end_of_path".to_string(),
        9 => "no_more_health".to_string(),
        10 => "user_0".to_string(),
        11 => "user_1".to_string(),
        12 => "user_2".to_string(),
        13 => "user_3".to_string(),
        14 => "user_4".to_string(),
        15 => "user_5".to_string(),
        16 => "user_6".to_string(),
        17 => "user_7".to_string(),
        18 => "user_8".to_string(),
        19 => "user_9".to_string(),
        20 => "user_10".to_string(),
        21 => "user_11".to_string(),
        22 => "user_12".to_string(),
        23 => "user_13".to_string(),
        24 => "user_14".to_string(),
        25 => "user_15".to_string(),
        30 => "close_button".to_string(),
        40 => "outside_view_0".to_string(),
        41 => "outside_view_1".to_string(),
        42 => "outside_view_2".to_string(),
        43 => "outside_view_3".to_string(),
        44 => "outside_view_4".to_string(),
        45 => "outside_view_5".to_string(),
        46 => "outside_view_6".to_string(),
        47 => "outside_view_7".to_string(),
        50 => "boundary_view_0".to_string(),
        51 => "boundary_view_1".to_string(),
        52 => "boundary_view_2".to_string(),
        53 => "boundary_view_3".to_string(),
        54 => "boundary_view_4".to_string(),
        55 => "boundary_view_5".to_string(),
        56 => "boundary_view_6".to_string(),
        57 => "boundary_view_7".to_string(),
        58 => "animation_update".to_string(),
        59 => "animation_event".to_string(),
        60 => "async_image_loaded".to_string(),
        62 => "async_http".to_string(),
        63 => "async_dialog".to_string(),
        66 => "async_iap".to_string(),
        67 => "async_cloud".to_string(),
        68 => "async_networking".to_string(),
        69 => "async_steam".to_string(),
        70 => "async_social".to_string(),
        71 => "async_push_notification".to_string(),
        72 => "async_save_load".to_string(),
        73 => "async_audio_recording".to_string(),
        74 => "async_audio_playback".to_string(),
        75 => "async_system".to_string(),
        _ => format!("{subtype}"),
    }
}

