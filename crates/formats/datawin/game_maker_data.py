# This is a generated file! Please edit source .ksy file and use kaitai-struct-compiler to rebuild
# type: ignore

import kaitaistruct
from kaitaistruct import KaitaiStruct, KaitaiStream, BytesIO
from enum import IntEnum


if getattr(kaitaistruct, 'API_VERSION', (0, 9)) < (0, 11):
    raise Exception("Incompatible Kaitai Struct Python API: 0.11 or later is required, but you have %s" % (kaitaistruct.__version__))

class GameMakerData(KaitaiStruct):
    """GameMaker Studio compiled game data file (data.win or game.win).
    Contains all compiled game assets: GML bytecode, object definitions,
    room layouts, sprites, sounds, fonts, texture atlases, and more.
    
    Structure: a FORM container holding named 8-byte-headered chunks.
    Chunk order varies by version; GEN8 always appears first and contains
    the bytecode_version field that governs the format of CODE/FUNC/VARI.
    
    Bytecode version (GEN8.bytecode_version):
      13 = Early GameMaker: Studio
      14 = GMS 1.x (old instruction format)
      15 = GMS 1.4.x (new instruction format; extended CODE/FUNC/VARI headers)
      16 = GMS 1.4.9999+ (LANG and GLOB chunks may be present)
      17 = GMS 2.x (GMS2 texture layout, OBJT managed field, FUNC operand addressing)
    
    GMS2.3+ (IDE version major >= 2): adds SEQN chunk; CODE uses shared bytecode
    blobs for child functions (lambdas, struct constructors). See CODE chunk docs.
    
    String references: most string fields store a u32 absolute file offset pointing
    to the character bytes of a GameMaker string. The 4-byte length prefix is at
    offset-4. To resolve a StringRef value V: seek to (V - 4) and read a gm_string.
    See type gm_string and the STRG chunk doc for full details.
    
    PE-embedded games: some GM1 games embed data.win inside a Windows PE (.exe).
    To locate the FORM: scan for the first 0x46 0x4F 0x52 0x4D ("FORM") sequence
    where the following u32 size field satisfies (size + 8 <= file_size). Strip
    the PE prefix before parsing.
    """

    class BboxModeKind(IntEnum):
        automatic = 0
        full_image = 1
        manual = 2

    class EventType(IntEnum):
        create = 0
        destroy = 1
        alarm = 2
        step = 3
        collision = 4
        keyboard = 5
        mouse = 6
        other = 7
        draw = 8
        key_press = 9
        key_release = 10
        trigger = 11

    class PhysicsShapeKind(IntEnum):
        circle = 0
        box = 1
        custom_polygon = 2

    class SepMasksKind(IntEnum):
        precise = 0
        rectangle = 1
        rotated_rectangle = 2
        diamond = 3
    def __init__(self, _io, _parent=None, _root=None):
        super(GameMakerData, self).__init__(_io)
        self._parent = _parent
        self._root = _root or self
        self._read()

    def _read(self):
        self.magic = self._io.read_bytes(4)
        if not self.magic == b"\x46\x4F\x52\x4D":
            raise kaitaistruct.ValidationNotEqualError(b"\x46\x4F\x52\x4D", self.magic, self._io, u"/seq/0")
        self.size = self._io.read_u4le()
        self.chunks = []
        i = 0
        while not self._io.is_eof():
            self.chunks.append(GameMakerData.Chunk(self._io, self, self._root))
            i += 1



    def _fetch_instances(self):
        pass
        for i in range(len(self.chunks)):
            pass
            self.chunks[i]._fetch_instances()


    class Action(KaitaiStruct):
        """An action within an event. Modern GM games use exactly one action per
        event, with action_kind=7 and exec_type=2 (execute a CODE entry).
        Legacy drag-and-drop actions (action_kind != 7) appear in older games.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.Action, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.lib_id = self._io.read_u4le()
            self.action_id = self._io.read_u4le()
            self.action_kind = self._io.read_u4le()
            self.has_relative = self._io.read_u4le()
            self.is_question = self._io.read_u4le()
            self.applies_to = self._io.read_s4le()
            self.exec_type = self._io.read_u4le()
            self.func_name = self._io.read_u4le()
            self.code_id = self._io.read_u4le()
            self.arg_count = self._io.read_u4le()
            self.who = self._io.read_s4le()
            self.relative = self._io.read_u4le()
            self.is_not = self._io.read_u4le()
            self.padding = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class AudoBody(KaitaiStruct):
        """Embedded audio files (WAV, OGG, MP3). Indexed by SOND.audio_id.
        Sounds with audio_id == -1 are external (streamed from a file on disk).
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.AudoBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.entries = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.entries._fetch_instances()


    class AudoEntry(KaitaiStruct):
        """A single embedded audio file (length-prefixed byte blob)."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.AudoEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.length = self._io.read_u4le()
            self.data = self._io.read_bytes(self.length)


        def _fetch_instances(self):
            pass


    class BgndBody(KaitaiStruct):
        """Background and tileset asset metadata.
          GMS1: standard background images (referenced in room background layers).
          GMS2: tileset definitions (tile dimensions, border padding, tile count, etc.).
        
        Only the name field is described here; the remaining tileset geometry
        data in GMS2 varies by IDE version and is not fully parsed.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.BgndBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.backgrounds = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.backgrounds._fetch_instances()


    class BgndEntry(KaitaiStruct):
        """Background/tileset entry. Accessed via absolute pointer.
        Entry size is not stored; compute from pointer spacing if needed.
        GMS2 entries have additional tileset fields after name (tile dimensions,
        border sizes, tile count, texture page item pointer, etc.) — not parsed here.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.BgndEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class Chunk(KaitaiStruct):
        """A single named data chunk within the FORM container."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.Chunk, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.magic = (self._io.read_bytes(4)).decode(u"ASCII")
            self.size = self._io.read_u4le()
            _on = self.magic
            if _on == u"AUDO":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.AudoBody(_io__raw_body, self, self._root)
            elif _on == u"BGND":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.BgndBody(_io__raw_body, self, self._root)
            elif _on == u"CODE":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.CodeBody(_io__raw_body, self, self._root)
            elif _on == u"FONT":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.FontBody(_io__raw_body, self, self._root)
            elif _on == u"FUNC":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.FuncBody(_io__raw_body, self, self._root)
            elif _on == u"GEN8":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.Gen8Body(_io__raw_body, self, self._root)
            elif _on == u"GLOB":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.GlobBody(_io__raw_body, self, self._root)
            elif _on == u"LANG":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.LangBody(_io__raw_body, self, self._root)
            elif _on == u"OBJT":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.ObjtBody(_io__raw_body, self, self._root)
            elif _on == u"OPTN":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.OptnBody(_io__raw_body, self, self._root)
            elif _on == u"ROOM":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.RoomBody(_io__raw_body, self, self._root)
            elif _on == u"SCPT":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.ScptBody(_io__raw_body, self, self._root)
            elif _on == u"SEQN":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.SeqnBody(_io__raw_body, self, self._root)
            elif _on == u"SHDR":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.ShdrBody(_io__raw_body, self, self._root)
            elif _on == u"SOND":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.SondBody(_io__raw_body, self, self._root)
            elif _on == u"SPRT":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.SprtBody(_io__raw_body, self, self._root)
            elif _on == u"STRG":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.StrgBody(_io__raw_body, self, self._root)
            elif _on == u"TPAG":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.TpagBody(_io__raw_body, self, self._root)
            elif _on == u"TXTR":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.TxtrBody(_io__raw_body, self, self._root)
            elif _on == u"VARI":
                pass
                self._raw_body = self._io.read_bytes(self.size)
                _io__raw_body = KaitaiStream(BytesIO(self._raw_body))
                self.body = GameMakerData.VariBody(_io__raw_body, self, self._root)
            else:
                pass
                self.body = self._io.read_bytes(self.size)


        def _fetch_instances(self):
            pass
            _on = self.magic
            if _on == u"AUDO":
                pass
                self.body._fetch_instances()
            elif _on == u"BGND":
                pass
                self.body._fetch_instances()
            elif _on == u"CODE":
                pass
                self.body._fetch_instances()
            elif _on == u"FONT":
                pass
                self.body._fetch_instances()
            elif _on == u"FUNC":
                pass
                self.body._fetch_instances()
            elif _on == u"GEN8":
                pass
                self.body._fetch_instances()
            elif _on == u"GLOB":
                pass
                self.body._fetch_instances()
            elif _on == u"LANG":
                pass
                self.body._fetch_instances()
            elif _on == u"OBJT":
                pass
                self.body._fetch_instances()
            elif _on == u"OPTN":
                pass
                self.body._fetch_instances()
            elif _on == u"ROOM":
                pass
                self.body._fetch_instances()
            elif _on == u"SCPT":
                pass
                self.body._fetch_instances()
            elif _on == u"SEQN":
                pass
                self.body._fetch_instances()
            elif _on == u"SHDR":
                pass
                self.body._fetch_instances()
            elif _on == u"SOND":
                pass
                self.body._fetch_instances()
            elif _on == u"SPRT":
                pass
                self.body._fetch_instances()
            elif _on == u"STRG":
                pass
                self.body._fetch_instances()
            elif _on == u"TPAG":
                pass
                self.body._fetch_instances()
            elif _on == u"TXTR":
                pass
                self.body._fetch_instances()
            elif _on == u"VARI":
                pass
                self.body._fetch_instances()
            else:
                pass


    class CodeBody(KaitaiStruct):
        """GML bytecode for all scripts and object events.
        An empty chunk (size == 0) means the game was compiled with YYC
        (GameMaker's native code compiler); no bytecode is available.
        
        Entry format depends on bytecode_version (from GEN8):
          BC <= 14: code_entry_v14 (simple: name + length, bytecode follows)
          BC >= 15: code_entry_v15 (extended: name + blob_length + locals +
                    args + bc_rel_addr + offset_in_blob)
        
        GMS2.3+ SHARED BYTECODE BLOBS (ide_version_major >= 2):
          Child functions (lambdas, struct constructors) share a single bytecode
          blob with their parent function. Each entry's blob_length is the TOTAL
          blob size (same value for parent and all its children).
        
          The actual per-entry bytecode length is NOT stored directly and must
          be computed by post-processing:
            1. Group entries by absolute blob address:
               blob_addr = (file_offset_of_bc_rel_addr_field) + bc_rel_addr
            2. Sort each group by offset_in_blob ascending.
            3. Each entry's length = next_entry.offset_in_blob - this.offset_in_blob
               (or blob_length - offset_in_blob for the last entry in the group).
        
          This gap-based computation is intentional; there is no stored length
          field that gives individual child function sizes.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.CodeBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.entries = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.entries._fetch_instances()


    class CodeEntryV14(KaitaiStruct):
        """BC <= 14 code entry (12 bytes header; bytecode follows immediately).
        Bytecode begins at: (absolute file offset of this entry) + 8.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.CodeEntryV14, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.length = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class CodeEntryV15(KaitaiStruct):
        """BC >= 15 code entry (24 bytes header).
        Bytecode blob address = (file offset of bc_rel_addr field) + bc_rel_addr.
        bc_rel_addr field is at: (entry pointer) + 12
          (after name:4 + blob_length:4 + locals_count:2 + args_count:2).
        Actual bytecode for this entry starts at blob_addr + offset_in_blob.
        See code_body doc for shared blob length computation.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.CodeEntryV15, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.blob_length = self._io.read_u4le()
            self.locals_count = self._io.read_u2le()
            self.args_count = self._io.read_u2le()
            self.bc_rel_addr = self._io.read_s4le()
            self.offset_in_blob = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class CodeLocalsEntry(KaitaiStruct):
        """Local variable declarations for a single code entry (BC >= 15 only).
        Follows the function list in the FUNC chunk.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.CodeLocalsEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.var_count = self._io.read_u4le()
            self.name = self._io.read_u4le()
            self.vars = []
            for i in range(self.var_count):
                self.vars.append(GameMakerData.LocalVar(self._io, self, self._root))



        def _fetch_instances(self):
            pass
            for i in range(len(self.vars)):
                pass
                self.vars[i]._fetch_instances()



    class EventEntry(KaitaiStruct):
        """A specific event handler within an event type
        (e.g. Create_0, Alarm_3, Collision_with_obj_Wall).
        Accessed via an absolute pointer from event_sublist.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.EventEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.subtype = self._io.read_u4le()
            self.actions = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.actions._fetch_instances()


    class EventSublist(KaitaiStruct):
        """Pointer list of event_entry structs for one event type category.
        Accessed via an absolute pointer from object_entry.event_list_ptrs.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.EventSublist, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.entries = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.entries._fetch_instances()


    class FontBody(KaitaiStruct):
        """Font asset definitions with per-glyph texture atlas coordinates."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.FontBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.fonts = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.fonts._fetch_instances()


    class FontEntry(KaitaiStruct):
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.FontEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.display_name = self._io.read_u4le()
            self.size = self._io.read_u4le()
            self.bold = self._io.read_u4le()
            self.italic = self._io.read_u4le()
            self.range_start = self._io.read_u2le()
            self.charset = self._io.read_u1()
            self.antialias = self._io.read_u1()
            self.range_end = self._io.read_u4le()
            self.tpag_ptr = self._io.read_u4le()
            self.scale_x = self._io.read_f4le()
            self.scale_y = self._io.read_f4le()
            self.glyphs = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.glyphs._fetch_instances()


    class FuncBody(KaitaiStruct):
        """Function call site chains and per-entry local variable declarations.
        An empty chunk (size == 0) means YYC-compiled game.
        
        Layout depends on bytecode_version:
          BC <= 14: flat list of function_entry structs (12 bytes each), no count
                    prefix. Read until end of chunk.
          BC >= 15: [func_count: u32][func_count × function_entry]
                    [locals_count: u32][locals_count × code_locals_entry]
        
        first_address field semantics differ by version:
          BC <= 16: absolute offset of the Call INSTRUCTION WORD (8-byte instruction).
          BC >= 17: absolute offset of the Call OPERAND WORD (4 bytes into the
                    instruction). Subtract 4 to obtain the instruction address.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.FuncBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.data = self._io.read_bytes_full()


        def _fetch_instances(self):
            pass


    class FunctionEntry(KaitaiStruct):
        """A single GML function definition with its call site chain head.
        Appears in both BC<=14 (flat list) and BC>=15 (count-prefixed list) formats.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.FunctionEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.occurrences = self._io.read_u4le()
            self.first_address = self._io.read_s4le()


        def _fetch_instances(self):
            pass


    class Gen8Body(KaitaiStruct):
        """Game metadata: version info, window dimensions, room order, etc.
        Always the first chunk. The bytecode_version field here governs the
        layout of CODE, FUNC, and VARI chunks.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.Gen8Body, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.is_debug_disabled = self._io.read_u1()
            self.bytecode_version = self._io.read_u1()
            self.padding = self._io.read_u2le()
            self.filename = self._io.read_u4le()
            self.config = self._io.read_u4le()
            self.last_obj = self._io.read_u4le()
            self.last_tile = self._io.read_u4le()
            self.game_id = self._io.read_u4le()
            self.guid = self._io.read_bytes(16)
            self.name = self._io.read_u4le()
            self.ide_version_major = self._io.read_u4le()
            self.ide_version_minor = self._io.read_u4le()
            self.ide_version_release = self._io.read_u4le()
            self.ide_version_build = self._io.read_u4le()
            self.default_window_width = self._io.read_u4le()
            self.default_window_height = self._io.read_u4le()
            self.info_flags = self._io.read_u4le()
            self.license_crc32 = self._io.read_u4le()
            self.license_md5 = self._io.read_bytes(16)
            self.timestamp = self._io.read_u8le()
            self.display_name = self._io.read_u4le()
            self.active_targets = self._io.read_u8le()
            self.function_classifications = self._io.read_u8le()
            self.steam_app_id = self._io.read_s4le()
            if self.bytecode_version >= 14:
                pass
                self.debugger_port = self._io.read_u4le()

            self.room_count = self._io.read_u4le()
            self.room_order = []
            for i in range(self.room_count):
                self.room_order.append(self._io.read_u4le())

            if self.ide_version_major >= 2:
                pass
                self.gms2_extra = self._io.read_bytes_full()



        def _fetch_instances(self):
            pass
            if self.bytecode_version >= 14:
                pass

            for i in range(len(self.room_order)):
                pass

            if self.ide_version_major >= 2:
                pass



    class GlobBody(KaitaiStruct):
        """List of CODE chunk indices for global init scripts — scripts that execute
        at game startup before the first room loads.
        Present only when bytecode_version >= 16.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.GlobBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.count = self._io.read_u4le()
            self.script_ids = []
            for i in range(self.count):
                self.script_ids.append(self._io.read_u4le())



        def _fetch_instances(self):
            pass
            for i in range(len(self.script_ids)):
                pass



    class Glyph(KaitaiStruct):
        """Per-character rendering data. Located via absolute pointer from font_entry.glyphs."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.Glyph, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.character = self._io.read_u2le()
            self.x = self._io.read_u2le()
            self.y = self._io.read_u2le()
            self.width = self._io.read_u2le()
            self.height = self._io.read_u2le()
            self.shift = self._io.read_s2le()
            self.advance = self._io.read_s2le()


        def _fetch_instances(self):
            pass


    class GmString(KaitaiStruct):
        """A GameMaker null-terminated string with a u32 length prefix.
        Format: [length: u32][chars: u8 × length][null: u8]
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.GmString, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.length = self._io.read_u4le()
            self.value = (self._io.read_bytes(self.length)).decode(u"UTF-8")
            self.terminator = self._io.read_bytes(1)
            if not self.terminator == b"\x00":
                raise kaitaistruct.ValidationNotEqualError(b"\x00", self.terminator, self._io, u"/types/gm_string/seq/2")


        def _fetch_instances(self):
            pass


    class LangBody(KaitaiStruct):
        """Language and localization configuration.
        Present only when bytecode_version >= 16.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.LangBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.entry_count = self._io.read_u4le()
            self.actual_count = self._io.read_u4le()
            self.entries = []
            for i in range(self.actual_count):
                self.entries.append(GameMakerData.LangEntry(self._io, self, self._root))



        def _fetch_instances(self):
            pass
            for i in range(len(self.entries)):
                pass
                self.entries[i]._fetch_instances()



    class LangEntry(KaitaiStruct):
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.LangEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.region = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class LocalVar(KaitaiStruct):
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.LocalVar, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.index = self._io.read_u4le()
            self.name = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class ObjectEntryGms1(KaitaiStruct):
        """Object definition for bytecode_version <= 16 (GMS1).
        Use when GEN8.bytecode_version < 17. Followed by object_entry_tail.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.ObjectEntryGms1, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.sprite_index = self._io.read_s4le()
            self.visible = self._io.read_u4le()
            self.solid = self._io.read_u4le()
            self.tail = GameMakerData.ObjectEntryTail(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.tail._fetch_instances()


    class ObjectEntryGms2(KaitaiStruct):
        """Object definition for bytecode_version >= 17 (GMS2).
        Adds a `managed` field between `visible` and `solid` compared to GMS1.
        Use when GEN8.bytecode_version >= 17. Followed by object_entry_tail.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.ObjectEntryGms2, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.sprite_index = self._io.read_s4le()
            self.visible = self._io.read_u4le()
            self.managed = self._io.read_u4le()
            self.solid = self._io.read_u4le()
            self.tail = GameMakerData.ObjectEntryTail(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.tail._fetch_instances()


    class ObjectEntryTail(KaitaiStruct):
        """The version-invariant tail of an object entry, shared by both GMS1 and GMS2.
        Immediately follows `solid` in both object_entry_gms1 and object_entry_gms2.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.ObjectEntryTail, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.depth = self._io.read_s4le()
            self.persistent = self._io.read_u4le()
            self.parent_index = self._io.read_s4le()
            self.mask_index = self._io.read_s4le()
            self.physics_enabled = self._io.read_u4le()
            self.physics_sensor = self._io.read_u4le()
            self.physics_shape = KaitaiStream.resolve_enum(GameMakerData.PhysicsShapeKind, self._io.read_u4le())
            self.physics_density = self._io.read_f4le()
            self.physics_restitution = self._io.read_f4le()
            self.physics_group = self._io.read_u4le()
            self.physics_linear_damping = self._io.read_f4le()
            self.physics_angular_damping = self._io.read_f4le()
            self.physics_vertex_count = self._io.read_u4le()
            self.physics_friction = self._io.read_f4le()
            self.physics_awake = self._io.read_u4le()
            self.physics_kinematic = self._io.read_u4le()
            self.physics_vertices = []
            for i in range(self.physics_vertex_count):
                self.physics_vertices.append(GameMakerData.PhysicsVertex(self._io, self, self._root))

            self.event_type_count = self._io.read_u4le()
            self.event_list_ptrs = []
            for i in range(self.event_type_count):
                self.event_list_ptrs.append(self._io.read_u4le())



        def _fetch_instances(self):
            pass
            for i in range(len(self.physics_vertices)):
                pass
                self.physics_vertices[i]._fetch_instances()

            for i in range(len(self.event_list_ptrs)):
                pass



    class ObjtBody(KaitaiStruct):
        """Object definitions — the game's "classes", each with physics properties
        and event handlers (Create, Step, Draw, Collision, etc.).
        
        Pointer targets are object_entry_gms1 (BC <= 16) or object_entry_gms2 (BC >= 17).
        Check GEN8.bytecode_version to select the correct entry type.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.ObjtBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.objects = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.objects._fetch_instances()


    class OptionConstant(KaitaiStruct):
        """A named compile-time project constant (equivalent to a."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.OptionConstant, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.value = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class OptnBody(KaitaiStruct):
        """Game options flags and named compile-time constants.
        The constant list starts at a fixed offset: 60 bytes from the start
        of the chunk body (after flags + 56 bytes of reserved option data).
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.OptnBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.flags = self._io.read_u4le()
            self.reserved = self._io.read_bytes(56)
            self.constant_count = self._io.read_u4le()
            self.constants = []
            for i in range(self.constant_count):
                self.constants.append(GameMakerData.OptionConstant(self._io, self, self._root))



        def _fetch_instances(self):
            pass
            for i in range(len(self.constants)):
                pass
                self.constants[i]._fetch_instances()



    class PhysicsVertex(KaitaiStruct):
        """A single polygon vertex for custom physics shapes."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.PhysicsVertex, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.x = self._io.read_f4le()
            self.y = self._io.read_f4le()


        def _fetch_instances(self):
            pass


    class PointerList(KaitaiStruct):
        """A count-prefixed list of absolute file offsets (u32 each).
        The ubiquitous indirection pattern throughout the format:
          [count: u32][offset_0: u32][offset_1: u32]...[offset_{count-1}: u32]
        Each offset is an absolute byte position in the file where a typed
        struct begins. Follow each offset to parse the actual entry.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.PointerList, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.count = self._io.read_u4le()
            self.offsets = []
            for i in range(self.count):
                self.offsets.append(self._io.read_u4le())



        def _fetch_instances(self):
            pass
            for i in range(len(self.offsets)):
                pass



    class RoomBody(KaitaiStruct):
        """Room definitions (level layouts with object placements, views, backgrounds)."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.RoomBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.rooms = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.rooms._fetch_instances()


    class RoomEntry(KaitaiStruct):
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.RoomEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.caption = self._io.read_u4le()
            self.width = self._io.read_u4le()
            self.height = self._io.read_u4le()
            self.speed = self._io.read_u4le()
            self.persistent = self._io.read_u4le()
            self.background_color = self._io.read_u4le()
            self.draw_background_color = self._io.read_u4le()
            self.creation_code_id = self._io.read_s4le()
            self.flags = self._io.read_u4le()
            self.background_list_ptr = self._io.read_u4le()
            self.views_list_ptr = self._io.read_u4le()
            self.objects_list_ptr = self._io.read_u4le()
            self.tiles_list_ptr = self._io.read_u4le()
            self.physics_world = self._io.read_u4le()
            self.physics_top = self._io.read_u4le()
            self.physics_left = self._io.read_u4le()
            self.physics_right = self._io.read_u4le()
            self.physics_bottom = self._io.read_u4le()
            self.physics_gravity_x = self._io.read_f4le()
            self.physics_gravity_y = self._io.read_f4le()
            self.physics_pixels_to_meters = self._io.read_f4le()


        def _fetch_instances(self):
            pass


    class RoomObjectEntry(KaitaiStruct):
        """An object instance pre-placed in a room.
        Accessed via absolute pointer from room_entry.objects_list_ptr → pointer_list.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.RoomObjectEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.x = self._io.read_s4le()
            self.y = self._io.read_s4le()
            self.object_id = self._io.read_s4le()
            self.instance_id = self._io.read_u4le()
            self.creation_code_id = self._io.read_s4le()
            self.scale_x = self._io.read_f4le()
            self.scale_y = self._io.read_f4le()
            self.color = self._io.read_u4le()
            self.rotation = self._io.read_f4le()


        def _fetch_instances(self):
            pass


    class ScptBody(KaitaiStruct):
        """Script asset name-to-code mappings.
        In GMS2.3+, constructor functions and nested scripts have code_id values
        with the high bit set (>= 0x80000000). These are not direct CODE indices;
        look up the code entry by canonical name "gml_Script_<name>" instead.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.ScptBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.scripts = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.scripts._fetch_instances()


    class ScriptEntry(KaitaiStruct):
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.ScriptEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.code_id = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class SeqnBody(KaitaiStruct):
        """Animation sequence asset definitions. Present only in GMS2.3+ games
        (ide_version_major >= 2 from GEN8).
        
        CRITICAL: Unlike every other chunk, SEQN has a 4-byte version field
        BEFORE the standard count+pointer list. This is unique to SEQN.
        
        Full sequence data (keyframes, tracks, playback settings, embedded curves)
        follows each name entry but is not parsed here.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.SeqnBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.version = self._io.read_u4le()
            self.sequences = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.sequences._fetch_instances()


    class SeqnEntry(KaitaiStruct):
        """Sequence entry. Accessed via absolute pointer.
        Entry size is not stored; compute from pointer spacing if needed.
        After name: full sequence definition (playback mode, length, origin, tracks,
        keyframes, embedded animation curves, etc.). Format varies by SEQN version.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.SeqnEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class ShdrBody(KaitaiStruct):
        """Shader asset definitions. Each entry stores a name followed by
        GLSL vertex and fragment shader source strings (and HLSL equivalents
        for Windows targets). The source string layout varies by GM version
        and is not fully parsed here.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.ShdrBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.shaders = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.shaders._fetch_instances()


    class ShdrEntry(KaitaiStruct):
        """Shader entry. Accessed via absolute pointer.
        Entry size is not stored; compute from pointer spacing if needed.
        After name: GLSL vertex + fragment source strings and HLSL equivalents,
        stored as inline gm_strings (length-prefixed). Layout varies by GM version.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.ShdrEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class SondBody(KaitaiStruct):
        """Sound asset metadata (file references, volume, audio group assignment)."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.SondBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.sounds = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.sounds._fetch_instances()


    class SoundEntry(KaitaiStruct):
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.SoundEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.flags = self._io.read_u4le()
            self.type_name = self._io.read_u4le()
            self.file_name = self._io.read_u4le()
            self.effects = self._io.read_u4le()
            self.volume = self._io.read_f4le()
            self.pitch = self._io.read_f4le()
            self.group_id = self._io.read_s4le()
            self.audio_id = self._io.read_s4le()


        def _fetch_instances(self):
            pass


    class SpriteEntry(KaitaiStruct):
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.SpriteEntry, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.width = self._io.read_u4le()
            self.height = self._io.read_u4le()
            self.bbox_left = self._io.read_s4le()
            self.bbox_right = self._io.read_s4le()
            self.bbox_bottom = self._io.read_s4le()
            self.bbox_top = self._io.read_s4le()
            self.transparent = self._io.read_u4le()
            self.smooth = self._io.read_u4le()
            self.preload = self._io.read_u4le()
            self.bbox_mode = KaitaiStream.resolve_enum(GameMakerData.BboxModeKind, self._io.read_u4le())
            self.sep_masks = KaitaiStream.resolve_enum(GameMakerData.SepMasksKind, self._io.read_u4le())
            self.origin_x = self._io.read_s4le()
            self.origin_y = self._io.read_s4le()
            self.tpag_count = self._io.read_s4le()
            if self.tpag_count >= 0:
                pass
                self.tpag_ptrs = []
                for i in range(self.tpag_count):
                    self.tpag_ptrs.append(self._io.read_u4le())




        def _fetch_instances(self):
            pass
            if self.tpag_count >= 0:
                pass
                for i in range(len(self.tpag_ptrs)):
                    pass




    class SprtBody(KaitaiStruct):
        """Sprite asset metadata (dimensions, bounding boxes, per-frame texture refs)."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.SprtBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.sprites = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.sprites._fetch_instances()


    class StrgBody(KaitaiStruct):
        """String table: a pointer_list of absolute file offsets, one per string.
        Each offset points to the START of a gm_string (the length prefix u32).
        
        StringRef values used elsewhere in the file point to the CHARACTER DATA
        of a string, which is 4 bytes PAST the length prefix. To resolve a
        StringRef value V: seek to (V - 4) and read a gm_string.
        
        String indices in the STRG table are rarely used directly. Most references
        throughout the file are StringRef absolute offsets, not STRG indices.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.StrgBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.strings = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.strings._fetch_instances()


    class TexturePageItem(KaitaiStruct):
        """A single 22-byte rectangular region on a texture atlas.
        Accessed via absolute pointer from tpag_body.items.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.TexturePageItem, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.source_x = self._io.read_u2le()
            self.source_y = self._io.read_u2le()
            self.source_width = self._io.read_u2le()
            self.source_height = self._io.read_u2le()
            self.target_x = self._io.read_u2le()
            self.target_y = self._io.read_u2le()
            self.target_width = self._io.read_u2le()
            self.target_height = self._io.read_u2le()
            self.render_width = self._io.read_u2le()
            self.render_height = self._io.read_u2le()
            self.texture_page_id = self._io.read_u2le()


        def _fetch_instances(self):
            pass


    class TpagBody(KaitaiStruct):
        """Texture page items: rectangular sub-regions on texture atlas pages.
        Each sprite frame, font glyph, background tile, etc. maps to one entry
        describing its source location on a TXTR atlas page and how to blit it.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.TpagBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.items = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.items._fetch_instances()


    class TxtrBody(KaitaiStruct):
        """Texture atlas pages. Each entry points to raw image data (typically PNG,
        or QOI in GMS2023.4+) embedded in data.win.
        
        GMS2+ games may use external textures: data_offset points into an external
        .png file rather than into data.win. In that case data_offset is 0 or
        points past the file end; treat as absent and load externally.
        
        Entry layout differs by version — detect by pointer spacing:
          GMS1: (ptr[1] - ptr[0]) <= 12 → txtr_entry_gms1 (8 bytes)
          GMS2: (ptr[1] - ptr[0]) > 12  → txtr_entry_gms2 (28 bytes)
        For single-entry files, default to the simpler GMS1 layout unless
        bytecode_version >= 17.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.TxtrBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.textures = GameMakerData.PointerList(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.textures._fetch_instances()


    class TxtrEntryGms1(KaitaiStruct):
        """GMS1 texture entry (8 bytes at pointer location)."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.TxtrEntryGms1, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.unknown = self._io.read_u4le()
            self.data_offset = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class TxtrEntryGms2(KaitaiStruct):
        """GMS2 texture entry (28 bytes at pointer location)."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.TxtrEntryGms2, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.unknown0 = self._io.read_u4le()
            self.unknown1 = self._io.read_u4le()
            self.scaled = self._io.read_u4le()
            self.generated = self._io.read_u4le()
            self.unknown2 = self._io.read_u4le()
            self.width_or_zero = self._io.read_u4le()
            self.data_offset = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class VariBody(KaitaiStruct):
        """Variable reference chains (instance, global, and local variables).
        An empty chunk (size == 0) means YYC-compiled game.
        
        Layout depends on bytecode_version:
          BC <= 14: flat list of vari_entry_v14 (12 bytes each), no header.
          BC >= 15: [instance_var_count: u32][instance_var_count_max: u32]
                    [max_local_var_count: u32] then flat list of vari_entry_v15
                    (20 bytes each).
        
        Entry count for BC<=14: chunk_size / 12
        Entry count for BC>=15: (chunk_size - 12) / 20
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.VariBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.data = self._io.read_bytes_full()


        def _fetch_instances(self):
            pass


    class VariEntryV14(KaitaiStruct):
        """Variable entry for bytecode_version <= 14 (12 bytes)."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.VariEntryV14, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.occurrences = self._io.read_u4le()
            self.first_address = self._io.read_s4le()


        def _fetch_instances(self):
            pass


    class VariEntryV15(KaitaiStruct):
        """Variable entry for bytecode_version >= 15 (20 bytes)."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.VariEntryV15, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.name = self._io.read_u4le()
            self.instance_type = self._io.read_s4le()
            self.var_id = self._io.read_s4le()
            self.occurrences = self._io.read_u4le()
            self.first_address = self._io.read_s4le()


        def _fetch_instances(self):
            pass


    class VariHeaderV15(KaitaiStruct):
        """The 3-field header at the start of VARI when bytecode_version >= 15."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GameMakerData.VariHeaderV15, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.instance_var_count = self._io.read_u4le()
            self.instance_var_count_max = self._io.read_u4le()
            self.max_local_var_count = self._io.read_u4le()


        def _fetch_instances(self):
            pass



