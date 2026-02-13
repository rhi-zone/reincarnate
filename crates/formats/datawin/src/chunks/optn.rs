use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;

/// A named constant from OPTN.
#[derive(Debug)]
pub struct OptionConstant {
    /// Reference to the constant name string.
    pub name: StringRef,
    /// Reference to the constant value string.
    pub value: StringRef,
}

/// Parsed OPTN chunk — game options and flags.
#[derive(Debug)]
pub struct Optn {
    /// Option flags bitmask.
    pub flags: u32,
    /// Named constants defined in the project.
    pub constants: Vec<OptionConstant>,
}

/// Fixed header size before the constant count field.
const CONSTANTS_OFFSET: usize = 60;

impl Optn {
    /// Parse the OPTN chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    pub fn parse(chunk_data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);

        let flags = c.read_u32()?;

        // The constant list starts at a fixed offset (60 bytes into the chunk).
        // Before it: flags, color fields, and other option values.
        let mut constants = Vec::new();
        if chunk_data.len() >= CONSTANTS_OFFSET + 4 {
            c.seek(CONSTANTS_OFFSET);
            let count = c.read_u32()? as usize;
            constants.reserve(count);
            for _ in 0..count {
                let name = StringRef(c.read_u32()?);
                let value = StringRef(c.read_u32()?);
                constants.push(OptionConstant { name, value });
            }
        }

        Ok(Self { flags, constants })
    }
}
