use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;
use crate::version::BytecodeVersion;

/// A single code entry in the CODE chunk.
#[derive(Debug)]
pub struct CodeEntry {
    /// Reference to the entry's name string (e.g., "gml_Script_foo").
    pub name: StringRef,
    /// Length of bytecode in bytes.
    pub length: u32,
    /// Number of local variables (BC >= 15).
    pub locals_count: u16,
    /// Number of arguments. Bit 15 is "weird local flag" in some tools.
    pub args_count: u16,
    /// Absolute file offset where this entry's bytecode begins.
    pub bytecode_offset: usize,
}

/// Parsed CODE chunk.
#[derive(Debug)]
pub struct Code {
    pub entries: Vec<CodeEntry>,
}

impl Code {
    /// Parse the CODE chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    /// `chunk_data_offset` is the absolute file offset where `chunk_data` begins.
    /// `version` is needed to select the correct entry format.
    pub fn parse(
        chunk_data: &[u8],
        chunk_data_offset: usize,
        version: BytecodeVersion,
    ) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let ptrs = c.read_pointer_list()?;

        let mut entries = Vec::with_capacity(ptrs.len());
        for &ptr in &ptrs {
            // ptr is an absolute file offset to the entry header.
            // Convert to relative offset within chunk_data.
            let rel = ptr as usize - chunk_data_offset;
            let mut ec = Cursor::new(chunk_data);
            ec.seek(rel);

            let name = StringRef(ec.read_u32()?);
            let length = ec.read_u32()?;

            if version.has_extended_code_entries() {
                // BC >= 15: extended format with locals, args, bytecode offset
                let locals_count = ec.read_u16()?;
                let args_count = ec.read_u16()?;
                let bc_rel_addr = ec.read_i32()?;
                let _offset_in_blob = ec.read_u32()?;

                // bc_rel_addr is relative to the field that contains it.
                // The field is at rel + 12 (after name:4 + length:4 + locals:2 + args:2).
                let bc_abs = (ptr as i64 + 12 + bc_rel_addr as i64) as usize;

                entries.push(CodeEntry {
                    name,
                    length,
                    locals_count,
                    args_count,
                    bytecode_offset: bc_abs,
                });
            } else {
                // BC < 15: bytecode immediately follows the entry header
                let bc_abs = ptr as usize + 8; // after name(4) + length(4)
                entries.push(CodeEntry {
                    name,
                    length,
                    locals_count: 0,
                    args_count: 0,
                    bytecode_offset: bc_abs,
                });
            }
        }

        Ok(Self { entries })
    }

    /// Extract bytecode bytes for a specific entry from the full file data.
    pub fn entry_bytecode<'a>(&self, index: usize, data: &'a [u8]) -> Option<&'a [u8]> {
        let entry = self.entries.get(index)?;
        let start = entry.bytecode_offset;
        let end = start + entry.length as usize;
        if end <= data.len() {
            Some(&data[start..end])
        } else {
            None
        }
    }
}
