use datawin::bytecode::decode;
use datawin::chunks::code::Code;
use datawin::chunks::func::Func;
use datawin::chunks::gen8::Gen8;
use datawin::chunks::objt::Objt;
use datawin::chunks::scpt::Scpt;
use datawin::chunks::vari::Vari;
use datawin::reader::ChunkIndex;
use datawin::string_table::StringTable;
use datawin::version::BytecodeVersion;

fn load_if_exists(path: &str) -> Option<Vec<u8>> {
    std::fs::read(path).ok()
}

fn bounty_path() -> String {
    format!("{}/Bounty/data.win", env!("HOME"))
}

const UNDERTALE_PATH: &str = "/mnt/ssd/steam/steamapps/common/Undertale/assets/game.unx";
const CHRONICON_PATH: &str = "/mnt/ssd/steam/steamapps/common/Chronicon/data.win";

// ── Phase 1: ChunkIndex ─────────────────────────────────────────────

#[test]
fn parse_bounty_chunks() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping: Bounty/data.win not found");
        return;
    };
    let index = ChunkIndex::parse(&data).expect("failed to parse Bounty data.win");
    assert_eq!(index.len(), 22);

    let magics: Vec<&str> = index.chunks().iter().map(|c| c.magic_str()).collect();
    assert_eq!(
        magics,
        [
            "GEN8", "OPTN", "EXTN", "SOND", "AGRP", "SPRT", "BGND", "PATH", "SCPT", "SHDR",
            "FONT", "TMLN", "OBJT", "ROOM", "DAFL", "TPAG", "CODE", "VARI", "FUNC", "STRG",
            "TXTR", "AUDO",
        ]
    );

    let gen8 = index.find(b"GEN8").expect("GEN8 not found");
    assert_eq!(gen8.offset, 8);
    assert_eq!(gen8.size, 252);
}

#[test]
fn parse_undertale_chunks() {
    let Some(data) = load_if_exists(UNDERTALE_PATH) else {
        eprintln!("skipping: Undertale game.unx not found");
        return;
    };
    let index = ChunkIndex::parse(&data).expect("failed to parse Undertale game.unx");
    assert_eq!(index.len(), 24);
    assert!(index.find(b"LANG").is_some());
    assert!(index.find(b"GLOB").is_some());
}

#[test]
fn parse_chronicon_chunks() {
    let Some(data) = load_if_exists(CHRONICON_PATH) else {
        eprintln!("skipping: Chronicon data.win not found");
        return;
    };
    let index = ChunkIndex::parse(&data).expect("failed to parse Chronicon data.win");
    assert_eq!(index.len(), 31);
    assert!(index.find(b"TGIN").is_some());
    assert!(index.find(b"FEAT").is_some());
    assert!(index.find(b"FEDS").is_some());
    assert!(index.find(b"EMBI").is_some());
    assert!(index.find(b"CODE").is_none());
}

#[test]
fn chunk_data_extraction() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping: Bounty/data.win not found");
        return;
    };
    let index = ChunkIndex::parse(&data).expect("failed to parse");
    let gen8_data = index.chunk_data(&data, b"GEN8").expect("GEN8 data");
    assert_eq!(gen8_data.len(), 252);
    assert_eq!(gen8_data[0], 1); // debug
    assert_eq!(gen8_data[1], 15); // bytecode version
}

// ── Phase 2: String Table + GEN8 ────────────────────────────────────

#[test]
fn bounty_string_table() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let strg_entry = index.find(b"STRG").unwrap();
    let strg_data = index.chunk_data(&data, b"STRG").unwrap();
    let table = StringTable::parse(strg_data, strg_entry.data_offset()).unwrap();

    assert_eq!(table.len(), 2281);

    // First few strings from hex verification
    assert_eq!(table.get(0, &data).unwrap(), "prototype");
    assert_eq!(table.get(1, &data).unwrap(), "@@array@@");
    assert_eq!(table.get(2, &data).unwrap(), "arguments");
    assert_eq!(table.get(3, &data).unwrap(), "active");
    assert_eq!(table.get(4, &data).unwrap(), "mouse_check_button_pressed");
}

#[test]
fn undertale_string_table() {
    let Some(data) = load_if_exists(UNDERTALE_PATH) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let strg_entry = index.find(b"STRG").unwrap();
    let strg_data = index.chunk_data(&data, b"STRG").unwrap();
    let table = StringTable::parse(strg_data, strg_entry.data_offset()).unwrap();

    // Undertale should have many strings
    assert!(table.len() > 1000, "expected >1000 strings, got {}", table.len());

    // First string should be "prototype" (same GMS convention)
    assert_eq!(table.get(0, &data).unwrap(), "prototype");
}

#[test]
fn bounty_gen8() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let gen8_data = index.chunk_data(&data, b"GEN8").unwrap();
    let gen8 = Gen8::parse(gen8_data).unwrap();

    assert_eq!(gen8.bytecode_version, BytecodeVersion::V15);
    assert_eq!(gen8.major, 1);
    assert_eq!(gen8.default_window_width, 640);
    assert_eq!(gen8.default_window_height, 480);
    assert_eq!(gen8.room_order.len(), 30);
    assert!(gen8.gms2_data.is_empty());

    // Resolve game name
    let name = gen8.name.resolve(&data).unwrap();
    assert!(!name.is_empty(), "game name should not be empty");
}

#[test]
fn undertale_gen8() {
    let Some(data) = load_if_exists(UNDERTALE_PATH) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let gen8_data = index.chunk_data(&data, b"GEN8").unwrap();
    let gen8 = Gen8::parse(gen8_data).unwrap();

    assert_eq!(gen8.bytecode_version, BytecodeVersion::V16);
    assert_eq!(gen8.major, 1);
    assert_eq!(gen8.default_window_width, 640);
    assert_eq!(gen8.default_window_height, 480);
    assert_eq!(gen8.room_order.len(), 336);
    assert!(gen8.gms2_data.is_empty());
}

#[test]
fn chronicon_gen8() {
    let Some(data) = load_if_exists(CHRONICON_PATH) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let gen8_data = index.chunk_data(&data, b"GEN8").unwrap();
    let gen8 = Gen8::parse(gen8_data).unwrap();

    assert_eq!(gen8.bytecode_version, BytecodeVersion::V17);
    assert_eq!(gen8.major, 2); // GMS2
    assert_eq!(gen8.default_window_width, 960);
    assert_eq!(gen8.default_window_height, 540);
    assert_eq!(gen8.room_order.len(), 6);
    assert!(!gen8.gms2_data.is_empty(), "GMS2 data should be present");
    assert_eq!(gen8.gms2_data.len(), 68);
}

// ── Phase 3: CODE + Bytecode Decoder ────────────────────────────────

fn parse_code_for(data: &[u8]) -> (Code, Gen8) {
    let index = ChunkIndex::parse(data).unwrap();
    let gen8_data = index.chunk_data(data, b"GEN8").unwrap();
    let gen8 = Gen8::parse(gen8_data).unwrap();
    let code_entry = index.find(b"CODE").unwrap();
    let code_data = index.chunk_data(data, b"CODE").unwrap();
    let code = Code::parse(code_data, code_entry.data_offset(), gen8.bytecode_version).unwrap();
    (code, gen8)
}

#[test]
fn bounty_code_entries() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping");
        return;
    };
    let (code, _gen8) = parse_code_for(&data);

    assert_eq!(code.entries.len(), 197);

    // First entry should be a gml_Script
    let first = &code.entries[0];
    let name = first.name.resolve(&data).unwrap();
    assert_eq!(name, "gml_Script_button_click");
    assert_eq!(first.length, 324);
    assert_eq!(first.locals_count, 1);
}

#[test]
fn bounty_decode_all_bytecode() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping");
        return;
    };
    let (code, _gen8) = parse_code_for(&data);

    let mut total_instructions = 0;
    for (i, entry) in code.entries.iter().enumerate() {
        let bc = code
            .entry_bytecode(i, &data)
            .unwrap_or_else(|| panic!("bytecode for entry {}", i));
        let instructions = decode::decode(bc)
            .unwrap_or_else(|e| {
                let name = entry.name.resolve(&data).unwrap_or_default();
                panic!("decode entry {} ({}): {}", i, name, e)
            });
        total_instructions += instructions.len();
    }

    // Bounty should have a reasonable number of instructions
    assert!(
        total_instructions > 1000,
        "expected >1000 instructions total, got {}",
        total_instructions
    );
    eprintln!(
        "Bounty: {} entries, {} total instructions",
        code.entries.len(),
        total_instructions
    );
}

#[test]
fn undertale_code_entries() {
    let Some(data) = load_if_exists(UNDERTALE_PATH) else {
        eprintln!("skipping");
        return;
    };
    let (code, _gen8) = parse_code_for(&data);

    // Undertale should have many code entries
    assert!(
        code.entries.len() > 100,
        "expected >100 code entries, got {}",
        code.entries.len()
    );
}

#[test]
fn undertale_decode_all_bytecode() {
    let Some(data) = load_if_exists(UNDERTALE_PATH) else {
        eprintln!("skipping");
        return;
    };
    let (code, _gen8) = parse_code_for(&data);

    let mut total_instructions = 0;
    let mut errors = 0;
    for (i, entry) in code.entries.iter().enumerate() {
        let bc = code
            .entry_bytecode(i, &data)
            .unwrap_or_else(|| panic!("bytecode for entry {}", i));
        match decode::decode(bc) {
            Ok(insts) => total_instructions += insts.len(),
            Err(e) => {
                let name = entry.name.resolve(&data).unwrap_or_default();
                eprintln!("  decode error in {}: {}", name, e);
                errors += 1;
            }
        }
    }

    eprintln!(
        "Undertale: {} entries, {} total instructions, {} errors",
        code.entries.len(),
        total_instructions,
        errors
    );
    assert_eq!(errors, 0, "all entries should decode without errors");
}

// ── Phase 4: FUNC + VARI ───────────────────────────────────────────

#[test]
fn bounty_func() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let gen8 = Gen8::parse(index.chunk_data(&data, b"GEN8").unwrap()).unwrap();
    let func_data = index.chunk_data(&data, b"FUNC").unwrap();
    let func = Func::parse(func_data, gen8.bytecode_version).unwrap();

    assert_eq!(func.functions.len(), 101);
    assert_eq!(func.code_locals.len(), 197);

    // First function
    let f0 = &func.functions[0];
    let name = f0.name.resolve(&data).unwrap();
    assert_eq!(name, "mouse_check_button_pressed");
    assert_eq!(f0.occurrences, 2);

    // First code locals entry
    let cl0 = &func.code_locals[0];
    let cl_name = cl0.name.resolve(&data).unwrap();
    assert_eq!(cl_name, "gml_Script_button_click");
    assert_eq!(cl0.locals.len(), 1);
    assert_eq!(cl0.locals[0].name.resolve(&data).unwrap(), "arguments");
}

#[test]
fn bounty_vari() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let gen8 = Gen8::parse(index.chunk_data(&data, b"GEN8").unwrap()).unwrap();
    let vari_data = index.chunk_data(&data, b"VARI").unwrap();
    let vari = Vari::parse(vari_data, gen8.bytecode_version).unwrap();

    assert_eq!(vari.variables.len(), 610);
    assert_eq!(vari.instance_var_count_max, 206);
    assert_eq!(vari.max_local_var_count, 12);

    // First variable
    let v0 = &vari.variables[0];
    assert_eq!(v0.name.resolve(&data).unwrap(), "prototype");
    assert_eq!(v0.instance_type, -1); // self
    assert_eq!(v0.var_id, 0);
}

#[test]
fn undertale_func() {
    let Some(data) = load_if_exists(UNDERTALE_PATH) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let gen8 = Gen8::parse(index.chunk_data(&data, b"GEN8").unwrap()).unwrap();
    let func_data = index.chunk_data(&data, b"FUNC").unwrap();
    let func = Func::parse(func_data, gen8.bytecode_version).unwrap();

    assert!(
        func.functions.len() > 100,
        "expected >100 functions, got {}",
        func.functions.len()
    );
    // Code locals should match code entry count
    assert!(func.code_locals.len() > 100);

    eprintln!(
        "Undertale: {} functions, {} code_locals entries",
        func.functions.len(),
        func.code_locals.len()
    );
}

#[test]
fn undertale_vari() {
    let Some(data) = load_if_exists(UNDERTALE_PATH) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let gen8 = Gen8::parse(index.chunk_data(&data, b"GEN8").unwrap()).unwrap();
    let vari_data = index.chunk_data(&data, b"VARI").unwrap();
    let vari = Vari::parse(vari_data, gen8.bytecode_version).unwrap();

    assert!(
        vari.variables.len() > 100,
        "expected >100 variables, got {}",
        vari.variables.len()
    );
    eprintln!(
        "Undertale: {} variables, instance_var_max={}, max_local={}",
        vari.variables.len(),
        vari.instance_var_count_max,
        vari.max_local_var_count
    );
}

// ── Phase 5: SCPT + OBJT ─────────────────────────────────────────

#[test]
fn bounty_scpt() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let scpt_entry = index.find(b"SCPT").unwrap();
    let scpt_data = index.chunk_data(&data, b"SCPT").unwrap();
    let scpt = Scpt::parse(scpt_data, scpt_entry.data_offset(), &data).unwrap();

    assert_eq!(scpt.scripts.len(), 61);

    // First script
    let s0 = &scpt.scripts[0];
    assert_eq!(s0.name.resolve(&data).unwrap(), "button_click");
    assert_eq!(s0.code_id, 0);

    // Scripts should map to sequential code IDs
    for (i, s) in scpt.scripts.iter().enumerate() {
        assert_eq!(
            s.code_id, i as u32,
            "script {} code_id mismatch",
            s.name.resolve(&data).unwrap_or_default()
        );
    }
}

#[test]
fn bounty_objt() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let objt_data = index.chunk_data(&data, b"OBJT").unwrap();
    let objt = Objt::parse(objt_data, &data).unwrap();

    assert_eq!(objt.objects.len(), 86);

    // First object
    let obj0 = &objt.objects[0];
    assert_eq!(obj0.name.resolve(&data).unwrap(), "obj_button_base");
    assert_eq!(obj0.sprite_index, 0);
    assert!(obj0.visible);
    assert!(!obj0.solid);
    assert_eq!(obj0.depth, 0);
    assert!(!obj0.persistent);
    assert_eq!(obj0.parent_index, -100);
    assert_eq!(obj0.mask_index, -1);

    // Default physics values
    assert!(!obj0.physics_enabled);
    assert!((obj0.physics_density - 0.5).abs() < f32::EPSILON);
    assert!((obj0.physics_restitution - 0.1).abs() < f32::EPSILON);
    assert!((obj0.physics_friction - 0.2).abs() < f32::EPSILON);
    assert!(obj0.physics_awake);
    assert!(!obj0.physics_kinematic);
    assert!(obj0.physics_vertices.is_empty());

    // Event structure
    assert_eq!(obj0.events.len(), 12);

    // Create event (type 0): 1 sub-entry with subtype 0
    assert_eq!(obj0.events[0].len(), 1);
    assert_eq!(obj0.events[0][0].subtype, 0);
    assert_eq!(obj0.events[0][0].actions.len(), 1);

    // Mouse event (type 6): 2 sub-entries (mouse enter=11, mouse leave=10)
    assert_eq!(obj0.events[6].len(), 2);
    assert_eq!(obj0.events[6][0].subtype, 11); // mouse enter
    assert_eq!(obj0.events[6][1].subtype, 10); // mouse leave

    // All events should have valid code IDs
    let (code, _) = parse_code_for(&data);
    for event_list in &obj0.events {
        for event in event_list {
            for action in &event.actions {
                assert!(
                    (action.code_id as usize) < code.entries.len(),
                    "code_id {} out of range (max {})",
                    action.code_id,
                    code.entries.len()
                );
            }
        }
    }
}

#[test]
fn bounty_objt_code_linkage() {
    let Some(data) = load_if_exists(&bounty_path()) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let objt_data = index.chunk_data(&data, b"OBJT").unwrap();
    let objt = Objt::parse(objt_data, &data).unwrap();
    let (code, _) = parse_code_for(&data);

    // Verify code entry names match object+event naming convention
    let obj0 = &objt.objects[0];
    let obj_name = obj0.name.resolve(&data).unwrap();

    let event_type_names = [
        "Create", "Destroy", "Alarm", "Step", "Collision", "Keyboard", "Mouse", "Other", "Draw",
        "KeyPress", "KeyRelease", "Trigger",
    ];

    for (type_idx, event_list) in obj0.events.iter().enumerate() {
        for event in event_list {
            for action in &event.actions {
                let code_name = code.entries[action.code_id as usize]
                    .name
                    .resolve(&data)
                    .unwrap();
                let expected_suffix = format!(
                    "gml_Object_{}_{}_{}",
                    obj_name, event_type_names[type_idx], event.subtype
                );
                assert_eq!(
                    code_name, expected_suffix,
                    "code entry name mismatch for {}.{}.{}",
                    obj_name, event_type_names[type_idx], event.subtype
                );
            }
        }
    }
}

#[test]
fn undertale_scpt() {
    let Some(data) = load_if_exists(UNDERTALE_PATH) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let scpt_entry = index.find(b"SCPT").unwrap();
    let scpt_data = index.chunk_data(&data, b"SCPT").unwrap();
    let scpt = Scpt::parse(scpt_data, scpt_entry.data_offset(), &data).unwrap();

    assert!(
        scpt.scripts.len() > 100,
        "expected >100 scripts, got {}",
        scpt.scripts.len()
    );
    eprintln!("Undertale: {} scripts", scpt.scripts.len());
}

#[test]
fn undertale_objt() {
    let Some(data) = load_if_exists(UNDERTALE_PATH) else {
        eprintln!("skipping");
        return;
    };
    let index = ChunkIndex::parse(&data).unwrap();
    let objt_data = index.chunk_data(&data, b"OBJT").unwrap();
    let objt = Objt::parse(objt_data, &data).unwrap();

    assert!(
        objt.objects.len() > 100,
        "expected >100 objects, got {}",
        objt.objects.len()
    );

    // All objects should have at least 12 event types (13 for v16+)
    for (i, obj) in objt.objects.iter().enumerate() {
        assert!(
            obj.events.len() >= 12,
            "object {} ({}) has {} event types, expected >= 12",
            i,
            obj.name.resolve(&data).unwrap_or_default(),
            obj.events.len()
        );
    }

    eprintln!("Undertale: {} objects", objt.objects.len());
}
