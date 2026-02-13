use datawin::chunks::gen8::Gen8;
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
