# This is a generated file! Please edit source .ksy file and use kaitai-struct-compiler to rebuild
# type: ignore

import kaitaistruct
from kaitaistruct import KaitaiStruct, KaitaiStream, BytesIO
from enum import IntEnum


if getattr(kaitaistruct, 'API_VERSION', (0, 9)) < (0, 11):
    raise Exception("Incompatible Kaitai Struct Python API: 0.11 or later is required, but you have %s" % (kaitaistruct.__version__))

class GmlBytecode(KaitaiStruct):
    """GML virtual machine bytecode instruction stream.
    
    Each instruction is at minimum 4 bytes (one "word"). Some instructions consume
    one or two additional 4-byte words for the operand value.
    
    Instruction word layout (32 bits, little-endian):
      bits 31-24  opcode (u8)
      bits 23-20  type2  (4-bit DataType, interpretation depends on opcode)
      bits 19-16  type1  (4-bit DataType, interpretation depends on opcode)
      bits 15-0   val16  (16-bit field, interpretation depends on opcode)
    
    Two instruction encodings exist, distinguished by `GEN8.bytecode_version`:
      v14 (BC <= 14): old opcode values (see enum opcode_v14).
      v15+ (BC >= 15): new opcode values used here (see enum opcode).
    
    v14 games also lack the Call operand word and use slightly different field
    layouts; see the v14 notes in each instruction type.
    
    This file describes the v15+ encoding. The `gml_instruction` sequence type
    expects the stream to start at the beginning of a code entry's bytecode blob
    and continue until the entry's byte length is exhausted.
    
    Bytecode location in data.win:
      - code_entry_v14: bytecode follows the 12-byte header immediately.
      - code_entry_v15+: bytecode is at (entry_header_addr + offset_in_blob) within
        the shared blob. See game_maker_data.ksy for pointer/offset arithmetic.
    """

    class ComparisonKind(IntEnum):
        less = 1
        less_equal = 2
        equal = 3
        not_equal = 4
        greater_equal = 5
        greater = 6

    class DataType(IntEnum):
        double = 0
        float = 1
        int32 = 2
        int64 = 3
        bool = 4
        variable = 5
        string = 6
        int16 = 15

    class InstanceType(IntEnum):
        arg = -16
        static = -15
        stacktop = -9
        local = -7
        builtin = -6
        global = -5
        noone = -4
        all = -3
        other = -2
        own = -1

    class Opcode(IntEnum):
        conv = 7
        mul = 8
        div = 9
        rem = 10
        mod = 11
        add = 12
        sub = 13
        and = 14
        or = 15
        xor = 16
        neg = 17
        not = 18
        shl = 19
        shr = 20
        cmp = 21
        pop = 69
        push_i = 132
        dup = 134
        call_v = 153
        ret = 156
        exit = 157
        popz = 158
        b = 182
        bt = 183
        bf = 184
        push_env = 186
        pop_env = 187
        push = 192
        push_loc = 193
        push_glb = 194
        push_bltn = 195
        call = 217
        brk = 255

    class OpcodeV14(IntEnum):
        conv = 3
        mul = 4
        div = 5
        rem = 6
        mod = 7
        add = 8
        sub = 9
        and = 10
        or = 11
        xor = 12
        neg = 13
        not = 14
        shl = 15
        shr = 16
        cmp_lt = 17
        cmp_le = 18
        cmp_eq = 19
        cmp_ne = 20
        cmp_ge = 21
        cmp_gt = 22
        pop = 65
        dup = 130
        ret = 157
        exit = 158
        popz = 159
        b = 183
        bt = 184
        bf = 185
        push_env = 187
        pop_env = 188
        push = 192
        call = 218
        brk = 255
    def __init__(self, bytecode_version, _io, _parent=None, _root=None):
        super(GmlBytecode, self).__init__(_io)
        self._parent = _parent
        self._root = _root or self
        self.bytecode_version = bytecode_version
        self._read()

    def _read(self):
        self.instructions = []
        i = 0
        while not self._io.is_eof():
            self.instructions.append(GmlBytecode.GmlInstruction(self._io, self, self._root))
            i += 1



    def _fetch_instances(self):
        pass
        for i in range(len(self.instructions)):
            pass
            self.instructions[i]._fetch_instances()


    class BreakBody(KaitaiStruct):
        """Break instruction — extended VM signals (GMS2.3+).
        
        The signal number is in val16 (treated as signed i16 = 0xFFFF..0x0000).
        When type1 == Int32 (0x2), one extra 4-byte word follows as the `extra` operand.
        
        Signal table (signal as signed i16):
          -1  (0xFFFF)  chkindex    — bounds-check array index; no stack effect.
          -2  (0xFFFE)  pushaf      — array get:  pops [index, array]; pushes value.
          -3  (0xFFFD)  popaf       — array set:  pops [value, index]; uses pushac ref.
          -4  (0xFFFC)  pushac      — capture array ref: pops array; stores for popaf.
          -5  (0xFFFB)  setowner    — pops instance ID (owner for next variable access).
          -6  (0xFFFA)  isstaticok  — pushes false (static init not yet done).
          -7  (0xFFF9)  setstatic   — enter static scope; nop for decompilation.
          -8  (0xFFF8)  savearef    — save array ref to temp; nop for decompilation.
          -9  (0xFFF7)  restorearef — restore array ref from temp; nop for decompilation.
          -10 (0xFFF6)  chknullish  — peek TOS; push bool (is TOS nullish?). Used for ?? / ?. .
          -11 (0xFFF5)  pushref     — push asset reference. extra = (type_tag << 24) | asset_index.
                                      type1 == Int32; one extra word follows.
        
        pushref asset type_tag values (bits 31-24 of extra):
          0  FUNC  — function index (zero-based into FUNC chunk).
          1  SPRT  — sprite index.
          2  SOND  — sound index.
          3  ROOM  — room index.
          4  PATH  — path index (deprecated in GMS2).
          5  SCPT  — script index.
          6  FONT  — font index.
          7  TMLN  — timeline index.
          8  SHDR  — shader index.
          9  SEQN  — sequence index (GMS2.3+).
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GmlBytecode.BreakBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            if self._parent.type1 == GmlBytecode.DataType.int32:
                pass
                self.extra = self._io.read_s4le()



        def _fetch_instances(self):
            pass
            if self._parent.type1 == GmlBytecode.DataType.int32:
                pass



    class CallBody(KaitaiStruct):
        """Extra operand for Call (direct function call).
        val16 = argument count.
        The following 4-byte word is the FUNC-table index of the callee.
        In BC <= 16, this word points to the instruction itself (absolute offset).
        In BC >= 17, this word points to the operand word (subtract 4 for instruction).
        Reincarnate's decoder normalises this to a zero-based function index.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GmlBytecode.CallBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.function_id = self._io.read_u4le()


        def _fetch_instances(self):
            pass


    class DupBody(KaitaiStruct):
        """Dup instruction — duplicates bytes on the value stack.
        
        Standard mode (dup_extra == 0):
          Copies (val8 + 1) * sizeof(type1) BYTES from the top of the stack.
          "Copies" means push N additional copies; originals remain.
        
        Swap mode (GMS2.3+, dup_extra != 0, dup_size > 0):
          Reorders the top portion of the stack (used before popaf to align
          a Variable-sized value for the fixed-size popaf window).
        
        No-op mode (GMS2.3+, dup_extra != 0, dup_size == 0):
          Struct swap marker; has no stack effect during decompilation.
        
        GML stack type sizes (bytes per item):
          Variable = 16  (4 u32 units)
          Double   = 8   (2 u32 units)
          Int64    = 8   (2 u32 units)
          Int32    = 4   (1 u32 unit)
          Int16    = 4   (1 u32 unit)
          Bool     = 4   (1 u32 unit)
          String   = 4   (1 u32 unit)
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GmlBytecode.DupBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.raw = self._io.read_bytes(0)


        def _fetch_instances(self):
            pass

        @property
        def dup_extra(self):
            """High byte of val16 (GMS2.3+).
            0 = standard dup.
            Non-zero = swap mode or no-op (combined with dup_size).
            """
            if hasattr(self, '_m_dup_extra'):
                return self._m_dup_extra

            self._m_dup_extra = self._parent.val16 >> 8 & 255
            return getattr(self, '_m_dup_extra', None)

        @property
        def val8(self):
            """Low byte of val16 — base duplication count/size parameter."""
            if hasattr(self, '_m_val8'):
                return self._m_val8

            self._m_val8 = self._parent.val16 & 255
            return getattr(self, '_m_val8', None)


    class EmptyBody(KaitaiStruct):
        """No extra operand words for this instruction."""
        def __init__(self, _io, _parent=None, _root=None):
            super(GmlBytecode.EmptyBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.raw = self._io.read_bytes(0)


        def _fetch_instances(self):
            pass


    class GmlInstruction(KaitaiStruct):
        """One GML VM instruction. Always starts on a 4-byte boundary.
        The instruction word is followed by 0, 1, or 2 extra words depending on
        the opcode and type fields.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GmlBytecode.GmlInstruction, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.word = self._io.read_u4le()
            _on = self.opcode
            if _on == GmlBytecode.Opcode.brk:
                pass
                self.body = GmlBytecode.BreakBody(self._io, self, self._root)
            elif _on == GmlBytecode.Opcode.call:
                pass
                self.body = GmlBytecode.CallBody(self._io, self, self._root)
            elif _on == GmlBytecode.Opcode.dup:
                pass
                self.body = GmlBytecode.DupBody(self._io, self, self._root)
            elif _on == GmlBytecode.Opcode.pop:
                pass
                self.body = GmlBytecode.PopBody(self._io, self, self._root)
            elif _on == GmlBytecode.Opcode.push:
                pass
                self.body = GmlBytecode.PushBody(self._io, self, self._root)
            elif _on == GmlBytecode.Opcode.push_bltn:
                pass
                self.body = GmlBytecode.PushBody(self._io, self, self._root)
            elif _on == GmlBytecode.Opcode.push_glb:
                pass
                self.body = GmlBytecode.PushBody(self._io, self, self._root)
            elif _on == GmlBytecode.Opcode.push_i:
                pass
                self.body = GmlBytecode.PushIBody(self._io, self, self._root)
            elif _on == GmlBytecode.Opcode.push_loc:
                pass
                self.body = GmlBytecode.PushBody(self._io, self, self._root)
            else:
                pass
                self.body = GmlBytecode.EmptyBody(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            _on = self.opcode
            if _on == GmlBytecode.Opcode.brk:
                pass
                self.body._fetch_instances()
            elif _on == GmlBytecode.Opcode.call:
                pass
                self.body._fetch_instances()
            elif _on == GmlBytecode.Opcode.dup:
                pass
                self.body._fetch_instances()
            elif _on == GmlBytecode.Opcode.pop:
                pass
                self.body._fetch_instances()
            elif _on == GmlBytecode.Opcode.push:
                pass
                self.body._fetch_instances()
            elif _on == GmlBytecode.Opcode.push_bltn:
                pass
                self.body._fetch_instances()
            elif _on == GmlBytecode.Opcode.push_glb:
                pass
                self.body._fetch_instances()
            elif _on == GmlBytecode.Opcode.push_i:
                pass
                self.body._fetch_instances()
            elif _on == GmlBytecode.Opcode.push_loc:
                pass
                self.body._fetch_instances()
            else:
                pass
                self.body._fetch_instances()

        @property
        def branch_offset_raw(self):
            """23-bit raw branch offset (bits 22-0).
            Only meaningful for B / Bt / Bf / PushEnv / PopEnv.
            Sign-extend from bit 22 to obtain a signed offset in 4-byte units.
            Multiply by 4 for byte offset from the start of this instruction.
            """
            if hasattr(self, '_m_branch_offset_raw'):
                return self._m_branch_offset_raw

            self._m_branch_offset_raw = self.word & 8388607
            return getattr(self, '_m_branch_offset_raw', None)

        @property
        def cmp_kind(self):
            """Comparison kind byte (bits 15-8).
            Only meaningful for Cmp; bits 15-8 encode the operator.
            """
            if hasattr(self, '_m_cmp_kind'):
                return self._m_cmp_kind

            self._m_cmp_kind = KaitaiStream.resolve_enum(GmlBytecode.ComparisonKind, self.word >> 8 & 255)
            return getattr(self, '_m_cmp_kind', None)

        @property
        def opcode(self):
            """High byte of the instruction word."""
            if hasattr(self, '_m_opcode'):
                return self._m_opcode

            self._m_opcode = KaitaiStream.resolve_enum(GmlBytecode.Opcode, self.word >> 24 & 255)
            return getattr(self, '_m_opcode', None)

        @property
        def type1(self):
            """Lower type nibble (bits 19-16).
            For most instructions: the type of the source/primary operand.
            For branch instructions: bits 19-16 are part of the 23-bit offset, not a type.
            """
            if hasattr(self, '_m_type1'):
                return self._m_type1

            self._m_type1 = KaitaiStream.resolve_enum(GmlBytecode.DataType, self.word >> 16 & 15)
            return getattr(self, '_m_type1', None)

        @property
        def type2(self):
            """Upper type nibble (bits 23-20).
            For two-operand arithmetic: destination type (Conv, arithmetic ops).
            For branch instructions: bits 23-20 are part of the 23-bit offset, not a type.
            Unused (0) for most other instructions.
            """
            if hasattr(self, '_m_type2'):
                return self._m_type2

            self._m_type2 = KaitaiStream.resolve_enum(GmlBytecode.DataType, self.word >> 20 & 15)
            return getattr(self, '_m_type2', None)

        @property
        def val16(self):
            """Low 16 bits of the instruction word. Interpretation depends on opcode."""
            if hasattr(self, '_m_val16'):
                return self._m_val16

            self._m_val16 = self.word & 65535
            return getattr(self, '_m_val16', None)


    class PopBody(KaitaiStruct):
        """Extra operand for Pop (variable write).
        val16 of the instruction word = instance (i16).
        The following 4-byte word is a variable_ref.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GmlBytecode.PopBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.var_ref = GmlBytecode.VariableRef(self._io, self, self._root)


        def _fetch_instances(self):
            pass
            self.var_ref._fetch_instances()


    class PushBody(KaitaiStruct):
        """Extra operand word(s) for Push / PushLoc / PushGlb / PushBltn.
        The number of words consumed and their interpretation is determined by
        `type1` from the enclosing instruction:
          Double (0x0): 8 bytes (two u32 words) — IEEE 754 f64, little-endian.
          Float  (0x1): 4 bytes (one u32 word)  — IEEE 754 f32, little-endian.
          Int32  (0x2): 4 bytes (one u32 word)  — signed 32-bit integer.
          Int64  (0x3): 8 bytes (two u32 words) — signed 64-bit integer, little-endian.
          Bool   (0x4): 4 bytes (one u32 word)  — boolean (0 = false, non-zero = true).
          String (0x6): 4 bytes (one u32 word)  — absolute file offset of a gm_string.
          Variable (0x5): 4 bytes (one u32 word) — variable_ref; val16 = instance (i16).
          Int16  (0xF): 0 bytes — value is inline in val16 (signed 16-bit).
        NOTE: This type cannot be expressed directly in Kaitai as a conditional-length
        sequence without using `if` expressions. Implementations should branch on
        type1 from the parent instruction word:
          - Double/Int64: read 8 bytes
          - Float/Int32/Bool/String: read 4 bytes
          - Variable: read 4 bytes (the variable_ref word; instance is in val16)
          - Int16: read 0 bytes
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GmlBytecode.PushBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.raw = self._io.read_bytes(0)


        def _fetch_instances(self):
            pass


    class PushIBody(KaitaiStruct):
        """Extra operand for PushI.
          Int16 (0xF): 0 bytes — value is inline in val16.
          Int32 (0x2): 4 bytes — one signed 32-bit word follows.
          Other: treated as Int16 (inline).
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GmlBytecode.PushIBody, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.raw = self._io.read_bytes(0)


        def _fetch_instances(self):
            pass


    class VariableRef(KaitaiStruct):
        """A packed 32-bit variable reference word (second word of Push/Pop Variable
        instructions). This encoding is used to build a linked list that the GM
        linker uses to patch variable indices. After linking, bits 23-0 are the
        final zero-based VARI table index.
        """
        def __init__(self, _io, _parent=None, _root=None):
            super(GmlBytecode.VariableRef, self).__init__(_io)
            self._parent = _parent
            self._root = _root
            self._read()

        def _read(self):
            self.raw = self._io.read_u4le()


        def _fetch_instances(self):
            pass

        @property
        def ref_type(self):
            """Reference type flags (high 5 bits of byte 3).
            Controls array-access mode and scope resolution:
              0x00  Normal variable access (self.field or local).
              0x80  Cross-instance access (target object index in instance field).
              0xA0  Singleton access (no index pops; instance field is object ID).
            Bit patterns observed in GMS1/GMS2 data.win files:
              0x00 + has_self:  self-field (object event handlers use this for own fields).
              0x00 + instance>=0: 2D array access (two indices on stack; dim1 on top).
              0x80 + instance>=0: cross-object field access via dynamic instance.
              0xA0 + instance>=0: singleton field (no stack pops for target resolution).
            """
            if hasattr(self, '_m_ref_type'):
                return self._m_ref_type

            self._m_ref_type = self.raw >> 24 & 248
            return getattr(self, '_m_ref_type', None)

        @property
        def variable_id(self):
            """Zero-based index into the VARI chunk entry list."""
            if hasattr(self, '_m_variable_id'):
                return self._m_variable_id

            self._m_variable_id = self.raw & 16777215
            return getattr(self, '_m_variable_id', None)



