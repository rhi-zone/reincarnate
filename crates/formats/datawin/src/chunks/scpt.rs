use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;

/// A script entry in the SCPT chunk.
#[derive(Debug)]
pub struct ScriptEntry {
    /// Reference to the script name string.
    pub name: StringRef,
    /// Index into the CODE chunk's entry list.
    pub code_id: u32,
}

/// Parsed SCPT chunk.
#[derive(Debug)]
pub struct Scpt {
    /// Script entries.
    pub scripts: Vec<ScriptEntry>,
}

impl Scpt {
    /// Parse the SCPT chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    /// `chunk_data_offset` is the absolute file offset where `chunk_data` begins.
    pub fn parse(chunk_data: &[u8], chunk_data_offset: usize, data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let pointers = c.read_pointer_list()?;

        let mut scripts = Vec::with_capacity(pointers.len());
        for ptr in pointers {
            let _ = chunk_data_offset; // pointers are absolute file offsets
            let mut ec = Cursor::new(data);
            ec.seek(ptr as usize);
            let name = StringRef(ec.read_u32()?);
            let code_id = ec.read_u32()?;
            scripts.push(ScriptEntry { name, code_id });
        }

        Ok(Self { scripts })
    }
}
