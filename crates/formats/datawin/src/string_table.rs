use crate::cursor::Cursor;
use crate::error::{Error, Result};

/// A reference to a string by its absolute file offset.
///
/// String references appear throughout the data.win format. The offset points
/// to the u32 length prefix of the string in the STRG chunk.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StringRef(pub u32);

impl StringRef {
    /// Resolve this reference against the full file data.
    ///
    /// Inline string references (in GEN8, CODE, etc.) point to the character
    /// data, with the u32 length prefix at offset - 4. This reads the length
    /// from offset - 4, then the character bytes and null terminator.
    pub fn resolve(&self, data: &[u8]) -> Result<String> {
        let offset = self.0 as usize;
        // Length prefix is 4 bytes before the character data
        if offset < 4 {
            return Err(Error::InvalidStringOffset { offset });
        }
        let len_offset = offset - 4;
        if len_offset + 4 > data.len() {
            return Err(Error::InvalidStringOffset { offset });
        }
        let mut cursor = Cursor::new(data);
        cursor.seek(len_offset);
        cursor.read_gm_string()
    }
}

/// Parsed string table from the STRG chunk.
///
/// Stores the array of absolute file offsets pointing to each string.
/// Strings are resolved on demand from the file data.
pub struct StringTable {
    /// Absolute file offsets for each string (index → offset).
    offsets: Vec<u32>,
}

impl StringTable {
    /// Parse the STRG chunk.
    ///
    /// `chunk_data` is the raw STRG chunk content (after the 8-byte header).
    /// `chunk_data_offset` is the absolute file offset where `chunk_data` begins.
    pub fn parse(chunk_data: &[u8], chunk_data_offset: usize) -> Result<Self> {
        let _ = chunk_data_offset; // offsets in pointer list are already absolute
        let mut cursor = Cursor::new(chunk_data);
        let offsets = cursor.read_pointer_list()?;
        Ok(Self { offsets })
    }

    /// Number of strings in the table.
    pub fn len(&self) -> usize {
        self.offsets.len()
    }

    /// Whether the table is empty.
    pub fn is_empty(&self) -> bool {
        self.offsets.is_empty()
    }

    /// Get the absolute file offset for string at `index`.
    pub fn offset(&self, index: usize) -> Option<u32> {
        self.offsets.get(index).copied()
    }

    /// Resolve string at `index` from the full file data.
    ///
    /// STRG offsets point to the length prefix (u32 len + chars + null).
    pub fn get(&self, index: usize, data: &[u8]) -> Result<String> {
        let offset = self.offsets.get(index).ok_or(Error::Parse {
            context: "STRG",
            message: format!("string index {} out of range (count={})", index, self.offsets.len()),
        })?;
        let offset = *offset as usize;
        if offset + 4 > data.len() {
            return Err(Error::InvalidStringOffset { offset });
        }
        let mut cursor = Cursor::new(data);
        cursor.seek(offset);
        cursor.read_gm_string()
    }

    /// Resolve a `StringRef` to an index in this table (linear scan).
    pub fn index_of(&self, string_ref: StringRef) -> Option<usize> {
        self.offsets.iter().position(|&off| off == string_ref.0)
    }

    /// Iterate over all string offsets.
    pub fn offsets(&self) -> &[u32] {
        &self.offsets
    }
}
