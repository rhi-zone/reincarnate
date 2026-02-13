use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;

/// A language entry.
#[derive(Debug)]
pub struct LangEntry {
    /// Reference to the language name string.
    pub name: StringRef,
    /// Reference to the region string.
    pub region: StringRef,
}

/// Parsed LANG chunk — language/localization settings.
#[derive(Debug)]
pub struct Lang {
    /// Number of language entries.
    pub entry_count: u32,
    /// Language entries.
    pub entries: Vec<LangEntry>,
}

impl Lang {
    /// Parse the LANG chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    pub fn parse(chunk_data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let entry_count = c.read_u32()?;

        let mut entries = Vec::new();
        if c.remaining() >= 4 {
            let count2 = c.read_u32()? as usize;
            entries.reserve(count2);
            for _ in 0..count2 {
                let name = StringRef(c.read_u32()?);
                let region = StringRef(c.read_u32()?);
                entries.push(LangEntry { name, region });
            }
        }

        Ok(Self {
            entry_count,
            entries,
        })
    }
}
