#!/usr/bin/env python3
"""Kaitai Struct fixture validator for the datawin crate.

Parses each `.bin` fixture with the Kaitai-compiled Python parser and validates
the structural fields that Kaitai CAN check against the paired `.json` expected-
value file.  Fields prefixed with `_` in the JSON are Rust-only (require full-
file context or imperative logic) and are SKIPPED here; those fields are
validated in `tests/fixture_tests.rs` instead.

Prerequisites
-------------
1.  Compile the .ksy specs to Python (run from `crates/formats/datawin/`):

        ksc -t python game_maker_data.ksy gml_bytecode.ksy

2.  Install the Python runtime:

        pip install kaitaistruct

3.  Run this script (from `crates/formats/datawin/`):

        python3 tests/kaitai_validate.py

The script exits non-zero if any assertion fails.
"""

import json
import os
import sys

FIXTURES_DIR = os.path.join(os.path.dirname(__file__), "fixtures")
KSY_DIR = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))

# ── Kaitai import ─────────────────────────────────────────────────────────────

def load_kaitai():
    """Import the kaitai-compiled game_maker_data module.

    ksc places the generated .py file in the current working directory.
    We look there first, then fall back to the ksy source directory.
    """
    for search_dir in [os.getcwd(), KSY_DIR]:
        if os.path.exists(os.path.join(search_dir, "game_maker_data.py")):
            sys.path.insert(0, search_dir)
            break
    else:
        print("ERROR: game_maker_data.py not found.")
        print("  Run: ksc -t python game_maker_data.ksy gml_bytecode.ksy")
        sys.exit(1)

    try:
        from kaitaistruct import KaitaiStream, BytesIO
        import game_maker_data as gmd_mod
        return gmd_mod, KaitaiStream, BytesIO
    except ImportError as e:
        print(f"ERROR: {e}")
        print("  Run: pip install kaitaistruct")
        sys.exit(1)


# ── Validation helpers ────────────────────────────────────────────────────────

PASS = "\033[32mPASS\033[0m"
FAIL = "\033[31mFAIL\033[0m"
SKIP = "\033[33mSKIP\033[0m"

failures = []


def check(label, actual, expected, skip=False):
    if skip:
        print(f"  {SKIP}  {label}")
        return
    if actual == expected:
        print(f"  {PASS}  {label}: {actual!r}")
    else:
        msg = f"{label}: expected {expected!r}, got {actual!r}"
        print(f"  {FAIL}  {msg}")
        failures.append(msg)


def check_chunks(gmd, expected_chunks):
    """Validate chunk count, magics, and data sizes."""
    actual_count = len(gmd.chunks)
    exp_count = len(expected_chunks)
    check("chunk count", actual_count, exp_count)

    for i, exp in enumerate(expected_chunks):
        if i >= actual_count:
            break
        chunk = gmd.chunks[i]
        magic = chunk.magic if isinstance(chunk.magic, str) else chunk.magic.decode("ascii")
        check(f"chunks[{i}].magic", magic, exp["magic"])
        check(f"chunks[{i}].data_size", chunk.size, exp["data_size"])


def check_gen8(gen8_kaitai, exp_gen8):
    """Validate GEN8 numeric fields (Kaitai can read all of these directly)."""
    check("gen8.bytecode_version", gen8_kaitai.bytecode_version, exp_gen8["bytecode_version"])
    check("gen8.is_debug_disabled", bool(gen8_kaitai.is_debug_disabled), exp_gen8["is_debug_disabled"])
    check("gen8.game_id", gen8_kaitai.game_id, exp_gen8["game_id"])
    # ksy uses ide_version_major/minor; JSON uses major/minor
    if "major" in exp_gen8:
        check("gen8.ide_version_major", gen8_kaitai.ide_version_major, exp_gen8["major"])
    if "minor" in exp_gen8:
        check("gen8.ide_version_minor", gen8_kaitai.ide_version_minor, exp_gen8["minor"])
    if "room_count" in exp_gen8:
        check("gen8.room_count", gen8_kaitai.room_count, exp_gen8["room_count"])


def check_strg(strg_kaitai, exp_strg):
    """Validate STRG string count."""
    # strg_kaitai.strings is a pointer_list; count is stored in .strings.count
    check("strg.count", strg_kaitai.strings.count, exp_strg["count"])
    # NOTE: string content resolution requires following the pointer to STRG char
    # data — this is listed in _kaitai_limitations.  The _strings field is
    # validated in Rust fixture tests instead.
    print(f"  {SKIP}  strg._strings (requires StringRef resolution — Kaitai limitation)")


def check_code(code_kaitai, exp_code):
    """Validate CODE chunk entry count.

    code_body.entries is a pointer_list (count + raw offsets).  The actual
    code_entry_v14 / code_entry_v15 structs are accessed via absolute seek —
    Kaitai stores only the count and raw offset array here, not parsed entries.
    Per-entry fields (locals_count, args_count, instructions) are validated in
    the Rust fixture tests instead.
    """
    check("code.count", code_kaitai.entries.count, exp_code["count"])
    # Per-entry validation requires following pointer_list offsets (not Kaitai-native):
    for i, exp_entry in enumerate(exp_code.get("entries", [])):
        prefix = f"code.entries[{i}]"
        if "locals_count" in exp_entry:
            print(f"  {SKIP}  {prefix}.locals_count (pointer-based entry — Kaitai limitation)")
        if "args_count" in exp_entry:
            print(f"  {SKIP}  {prefix}.args_count (pointer-based entry — Kaitai limitation)")
        if "_instructions" in exp_entry:
            print(f"  {SKIP}  {prefix}._instructions (push_body size:0 — Kaitai limitation)")


def check_vari(vari_kaitai, exp_vari):
    """Validate VARI chunk structural metadata where accessible."""
    # The VARI body is stored as a raw size-eos blob in game_maker_data.ksy
    # (Kaitai limitation: needs cross-chunk GEN8.bytecode_version to decode).
    print(f"  {SKIP}  vari fields (stored as raw blob — Kaitai limitation)")


def check_func(func_kaitai, exp_func):
    """Validate FUNC chunk structural metadata where accessible."""
    print(f"  {SKIP}  func fields (stored as raw blob — Kaitai limitation)")


def check_scpt(scpt_kaitai, exp_scpt):
    """Validate SCPT chunk entry count.

    scpt_body.scripts is a pointer_list; actual script entries are accessed via
    absolute seek (same limitation as CODE entries).  We can only check count here.
    """
    check("scpt.count", scpt_kaitai.scripts.count, exp_scpt["count"])
    for i, exp_entry in enumerate(exp_scpt.get("entries", [])):
        prefix = f"scpt.entries[{i}]"
        if "code_id" in exp_entry:
            print(f"  {SKIP}  {prefix}.code_id (pointer-based entry — Kaitai limitation)")
        print(f"  {SKIP}  {prefix}._name (StringRef — Kaitai limitation)")


def check_glob(glob_kaitai, exp_glob):
    """Validate GLOB chunk: flat count + script_ids array (all Kaitai-accessible)."""
    check("glob.count", glob_kaitai.count, exp_glob["count"])
    for i, expected_id in enumerate(exp_glob.get("script_ids", [])):
        check(f"glob.script_ids[{i}]", glob_kaitai.script_ids[i], expected_id)


def check_lang(lang_kaitai, exp_lang):
    """Validate LANG chunk: flat entry_count + count + entries (StringRefs skipped)."""
    check("lang.entry_count", lang_kaitai.entry_count, exp_lang["entry_count"])
    check("lang.count", lang_kaitai.actual_count, exp_lang["count"])
    for i, exp_entry in enumerate(exp_lang.get("entries", [])):
        prefix = f"lang.entries[{i}]"
        # name and region are raw u32 StringRef offsets — cannot resolve without STRG seek
        if "_name" in exp_entry:
            print(f"  {SKIP}  {prefix}._name (StringRef — Kaitai limitation)")
        if "_region" in exp_entry:
            print(f"  {SKIP}  {prefix}._region (StringRef — Kaitai limitation)")


def check_seqn(seqn_kaitai, exp_seqn):
    """Validate SEQN chunk: version field + count (entries are pointer-based)."""
    check("seqn.version", seqn_kaitai.version, exp_seqn["version"])
    check("seqn.count", seqn_kaitai.sequences.count, exp_seqn["count"])
    for i, exp_entry in enumerate(exp_seqn.get("_entries", [])):
        prefix = f"seqn.entries[{i}]"
        if "_name" in exp_entry:
            print(f"  {SKIP}  {prefix}._name (pointer-based entry — Kaitai limitation)")


def check_shdr(shdr_kaitai, exp_shdr):
    """Validate SHDR chunk entry count."""
    check("shdr.count", shdr_kaitai.shaders.count, exp_shdr["count"])
    for i, exp_entry in enumerate(exp_shdr.get("_entries", [])):
        prefix = f"shdr.entries[{i}]"
        if "_name" in exp_entry:
            print(f"  {SKIP}  {prefix}._name (pointer-based entry — Kaitai limitation)")


def check_bgnd(bgnd_kaitai, exp_bgnd):
    """Validate BGND chunk entry count."""
    check("bgnd.count", bgnd_kaitai.backgrounds.count, exp_bgnd["count"])
    for i, exp_entry in enumerate(exp_bgnd.get("_entries", [])):
        prefix = f"bgnd.entries[{i}]"
        if "_name" in exp_entry:
            print(f"  {SKIP}  {prefix}._name (pointer-based entry — Kaitai limitation)")


def check_sond(sond_kaitai, exp_sond):
    """Validate SOND chunk entry count (entries are pointer-based)."""
    check("sond.count", sond_kaitai.sounds.count, exp_sond["count"])
    for i, exp_entry in enumerate(exp_sond.get("entries", [])):
        prefix = f"sond.entries[{i}]"
        for field in ("_name", "flags", "_type_name", "_file_name", "effects", "volume", "pitch", "group_id", "audio_id"):
            if field in exp_entry:
                print(f"  {SKIP}  {prefix}.{field} (pointer-based entry — Kaitai limitation)")


def check_audo(audo_kaitai, exp_audo):
    """Validate AUDO chunk entry count (entries are pointer-based)."""
    check("audo.count", audo_kaitai.entries.count, exp_audo["count"])
    for i, exp_entry in enumerate(exp_audo.get("entries", [])):
        prefix = f"audo.entries[{i}]"
        if "length" in exp_entry:
            print(f"  {SKIP}  {prefix}.length (pointer-based entry — Kaitai limitation)")


def check_txtr(txtr_kaitai, exp_txtr):
    """Validate TXTR chunk entry count (entries are pointer-based)."""
    check("txtr.count", txtr_kaitai.textures.count, exp_txtr["count"])
    for i, exp_entry in enumerate(exp_txtr.get("_entries", [])):
        prefix = f"txtr.entries[{i}]"
        if "data_offset" in exp_entry:
            print(f"  {SKIP}  {prefix}.data_offset (pointer-based entry — Kaitai limitation)")


def check_tpag(tpag_kaitai, exp_tpag):
    """Validate TPAG chunk entry count (entries are pointer-based)."""
    check("tpag.count", tpag_kaitai.items.count, exp_tpag["count"])
    for i, exp_entry in enumerate(exp_tpag.get("entries", [])):
        prefix = f"tpag.entries[{i}]"
        for field in ("source_x", "source_y", "source_width", "source_height",
                      "target_x", "target_y", "target_width", "target_height",
                      "render_width", "render_height", "texture_page_id"):
            if field in exp_entry:
                print(f"  {SKIP}  {prefix}.{field} (pointer-based entry — Kaitai limitation)")


def check_sprt(sprt_kaitai, exp_sprt):
    """Validate SPRT chunk entry count (entries are pointer-based)."""
    check("sprt.count", sprt_kaitai.sprites.count, exp_sprt["count"])
    for i, exp_entry in enumerate(exp_sprt.get("entries", [])):
        prefix = f"sprt.entries[{i}]"
        for field in ("_name", "width", "height", "origin_x", "origin_y", "tpag_count"):
            if field in exp_entry:
                print(f"  {SKIP}  {prefix}.{field} (pointer-based entry — Kaitai limitation)")


def check_optn(optn_kaitai, exp_optn):
    """Validate OPTN chunk: flags and constant_count are directly accessible.

    The OPTN body is flat (not pointer-based): flags(u32) + reserved(56B) +
    constant_count(u32).  These three fields are fully Kaitai-accessible.
    The constant entries themselves are flat too, but their StringRef name/value
    fields require STRG resolution.
    """
    check("optn.flags", optn_kaitai.flags, exp_optn["flags"])
    check("optn.constant_count", optn_kaitai.constant_count, exp_optn["constant_count"])
    for i, exp_const in enumerate(exp_optn.get("constants", [])):
        prefix = f"optn.constants[{i}]"
        if "_name" in exp_const:
            print(f"  {SKIP}  {prefix}._name (StringRef — Kaitai limitation)")
        if "_value" in exp_const:
            print(f"  {SKIP}  {prefix}._value (StringRef — Kaitai limitation)")


def check_font(font_kaitai, exp_font):
    """Validate FONT chunk entry count (entries are pointer-based)."""
    check("font.count", font_kaitai.fonts.count, exp_font["count"])
    for i, exp_entry in enumerate(exp_font.get("entries", [])):
        prefix = f"font.entries[{i}]"
        for field in ("_name", "_display_name", "size", "bold", "italic",
                      "range_start", "charset", "antialias", "range_end",
                      "tpag_index", "scale_x", "scale_y", "glyph_count"):
            if field in exp_entry:
                print(f"  {SKIP}  {prefix}.{field} (pointer-based entry — Kaitai limitation)")
        for j, _ in enumerate(exp_entry.get("glyphs", [])):
            print(f"  {SKIP}  {prefix}.glyphs[{j}] (pointer-based glyph — Kaitai limitation)")


def check_objt(objt_kaitai, exp_objt):
    """Validate OBJT chunk entry count (entries are pointer-based)."""
    check("objt.count", objt_kaitai.objects.count, exp_objt["count"])
    for i, exp_entry in enumerate(exp_objt.get("entries", [])):
        prefix = f"objt.entries[{i}]"
        print(f"  {SKIP}  {prefix} fields (pointer-based entry — Kaitai limitation)")


def check_room(room_kaitai, exp_room):
    """Validate ROOM chunk entry count (entries are pointer-based)."""
    check("room.count", room_kaitai.rooms.count, exp_room["count"])
    for i, exp_entry in enumerate(exp_room.get("entries", [])):
        prefix = f"room.entries[{i}]"
        print(f"  {SKIP}  {prefix} fields (pointer-based entry — Kaitai limitation)")


# ── Per-fixture validation ────────────────────────────────────────────────────

def validate_fixture(fixture_name, gmd_mod, KaitaiStream, BytesIO):
    bin_path = os.path.join(FIXTURES_DIR, f"{fixture_name}.bin")
    json_path = os.path.join(FIXTURES_DIR, f"{fixture_name}.json")

    if not os.path.exists(bin_path):
        print(f"SKIP  {fixture_name}: {bin_path} not found")
        return
    if not os.path.exists(json_path):
        print(f"SKIP  {fixture_name}: {json_path} not found")
        return

    with open(bin_path, "rb") as f:
        raw = f.read()
    with open(json_path) as f:
        exp = json.load(f)

    print(f"\n── {fixture_name} ({len(raw)} bytes) ──")

    # File size
    check("file_size", len(raw), exp["file_size"])

    # Parse with Kaitai
    try:
        gmd = gmd_mod.GameMakerData(KaitaiStream(BytesIO(raw)))
    except Exception as e:
        failures.append(f"{fixture_name}: Kaitai parse failed: {e}")
        print(f"  {FAIL}  Kaitai parse failed: {e}")
        return

    # Chunks
    if "chunks" in exp:
        check_chunks(gmd, exp["chunks"])

    # Find specific chunks by magic
    chunk_map = {(c.magic if isinstance(c.magic, str) else c.magic.decode("ascii")): c for c in gmd.chunks}

    if "gen8" in exp and "GEN8" in chunk_map:
        check_gen8(chunk_map["GEN8"].body, exp["gen8"])

    if "strg" in exp and "STRG" in chunk_map:
        check_strg(chunk_map["STRG"].body, exp["strg"])

    if "code" in exp and "CODE" in chunk_map:
        check_code(chunk_map["CODE"].body, exp["code"])

    if "vari" in exp and "VARI" in chunk_map:
        check_vari(chunk_map["VARI"].body, exp["vari"])

    if "func" in exp and "FUNC" in chunk_map:
        check_func(chunk_map["FUNC"].body, exp["func"])

    if "scpt" in exp and "SCPT" in chunk_map:
        check_scpt(chunk_map["SCPT"].body, exp["scpt"])

    if "glob" in exp and "GLOB" in chunk_map:
        check_glob(chunk_map["GLOB"].body, exp["glob"])

    if "lang" in exp and "LANG" in chunk_map:
        check_lang(chunk_map["LANG"].body, exp["lang"])

    if "seqn" in exp and "SEQN" in chunk_map:
        check_seqn(chunk_map["SEQN"].body, exp["seqn"])

    if "shdr" in exp and "SHDR" in chunk_map:
        check_shdr(chunk_map["SHDR"].body, exp["shdr"])

    if "bgnd" in exp and "BGND" in chunk_map:
        check_bgnd(chunk_map["BGND"].body, exp["bgnd"])

    if "sond" in exp and "SOND" in chunk_map:
        check_sond(chunk_map["SOND"].body, exp["sond"])

    if "audo" in exp and "AUDO" in chunk_map:
        check_audo(chunk_map["AUDO"].body, exp["audo"])

    if "txtr" in exp and "TXTR" in chunk_map:
        check_txtr(chunk_map["TXTR"].body, exp["txtr"])

    if "tpag" in exp and "TPAG" in chunk_map:
        check_tpag(chunk_map["TPAG"].body, exp["tpag"])

    if "sprt" in exp and "SPRT" in chunk_map:
        check_sprt(chunk_map["SPRT"].body, exp["sprt"])

    if "optn" in exp and "OPTN" in chunk_map:
        check_optn(chunk_map["OPTN"].body, exp["optn"])

    if "font" in exp and "FONT" in chunk_map:
        check_font(chunk_map["FONT"].body, exp["font"])

    if "objt" in exp and "OBJT" in chunk_map:
        check_objt(chunk_map["OBJT"].body, exp["objt"])

    if "room" in exp and "ROOM" in chunk_map:
        check_room(chunk_map["ROOM"].body, exp["room"])


# ── Main ──────────────────────────────────────────────────────────────────────

FIXTURES = [
    "v15_minimal",
    "v15_bytecode_variety",
    "v15_break_signals",
    "v14_minimal",
    "v15_vari_func",
    "v15_more_opcodes",
    "v15_scpt",
    "v15_shared_blob",
    "v15_simple_chunks",
    "v15_sond_audo",
    "v15_sprt_tpag_txtr",
    "v15_optn",
    "v15_font",
    "v15_objt",
    "v15_room",
]


def main():
    gmd_mod, KaitaiStream, BytesIO = load_kaitai()

    for name in FIXTURES:
        validate_fixture(name, gmd_mod, KaitaiStream, BytesIO)

    print()
    if failures:
        print(f"FAILED: {len(failures)} assertion(s) failed:")
        for f in failures:
            print(f"  - {f}")
        sys.exit(1)
    else:
        print(f"All checks passed ({len(FIXTURES)} fixtures).")


if __name__ == "__main__":
    main()
