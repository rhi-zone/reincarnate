use crate::cursor::Cursor;
use crate::error::Result;

/// An audio entry in the AUDO chunk.
#[derive(Debug)]
pub struct AudioEntry {
    /// Absolute file offset of the audio data.
    pub offset: usize,
    /// Length of the audio data in bytes.
    pub length: u32,
}

/// Parsed AUDO chunk.
#[derive(Debug)]
pub struct Audo {
    /// Audio entries.
    pub entries: Vec<AudioEntry>,
}

impl Audo {
    /// Parse the AUDO chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    /// `chunk_data_offset` is the absolute file offset where `chunk_data` begins.
    pub fn parse(chunk_data: &[u8], chunk_data_offset: usize) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let pointers = c.read_pointer_list()?;

        let mut entries = Vec::with_capacity(pointers.len());
        for ptr in pointers {
            // Each audio entry starts with a u32 length, followed by the audio bytes
            let entry_offset = ptr as usize;
            let relative = entry_offset - chunk_data_offset;
            let mut ec = Cursor::new(chunk_data);
            ec.seek(relative);
            let length = ec.read_u32()?;
            let data_start = entry_offset + 4;

            entries.push(AudioEntry {
                offset: data_start,
                length,
            });
        }

        Ok(Self { entries })
    }

    /// Extract the raw audio data for an entry.
    pub fn audio_data<'a>(&self, index: usize, data: &'a [u8]) -> Option<&'a [u8]> {
        let entry = self.entries.get(index)?;
        let end = entry.offset + entry.length as usize;
        if end > data.len() {
            return None;
        }
        Some(&data[entry.offset..end])
    }
}
