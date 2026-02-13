use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;

/// A sound entry in the SOND chunk.
#[derive(Debug)]
pub struct SoundEntry {
    /// Reference to the sound name string.
    pub name: StringRef,
    /// Sound flags (e.g., 100 = normal).
    pub flags: u32,
    /// Reference to the sound type string (e.g., ".ogg", ".wav").
    pub type_name: StringRef,
    /// Reference to the sound file name string.
    pub file_name: StringRef,
    /// Effects bitmask.
    pub effects: u32,
    /// Volume (0.0 to 1.0).
    pub volume: f32,
    /// Pitch adjustment.
    pub pitch: f32,
    /// Audio group ID.
    pub group_id: i32,
    /// Index into the AUDO chunk, or -1 if external.
    pub audio_id: i32,
}

/// Parsed SOND chunk.
#[derive(Debug)]
pub struct Sond {
    /// Sound entries.
    pub sounds: Vec<SoundEntry>,
}

impl Sond {
    /// Parse the SOND chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    /// `data` is the full file data (for following absolute pointers).
    pub fn parse(chunk_data: &[u8], data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let pointers = c.read_pointer_list()?;

        let mut sounds = Vec::with_capacity(pointers.len());
        for ptr in pointers {
            let mut ec = Cursor::new(data);
            ec.seek(ptr as usize);

            let name = StringRef(ec.read_u32()?);
            let flags = ec.read_u32()?;
            let type_name = StringRef(ec.read_u32()?);
            let file_name = StringRef(ec.read_u32()?);
            let effects = ec.read_u32()?;
            let volume = ec.read_f32()?;
            let pitch = ec.read_f32()?;
            let group_id = ec.read_i32()?;
            let audio_id = ec.read_i32()?;

            sounds.push(SoundEntry {
                name,
                flags,
                type_name,
                file_name,
                effects,
                volume,
                pitch,
                group_id,
                audio_id,
            });
        }

        Ok(Self { sounds })
    }
}
