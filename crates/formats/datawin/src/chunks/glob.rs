use crate::cursor::Cursor;
use crate::error::Result;

/// Parsed GLOB chunk — global script IDs.
#[derive(Debug)]
pub struct Glob {
    /// Global script indices (into the CODE chunk).
    pub script_ids: Vec<u32>,
}

impl Glob {
    /// Parse the GLOB chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    pub fn parse(chunk_data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let count = c.read_u32()? as usize;
        let mut script_ids = Vec::with_capacity(count);
        for _ in 0..count {
            script_ids.push(c.read_u32()?);
        }
        Ok(Self { script_ids })
    }
}
