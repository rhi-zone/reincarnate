use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;
use crate::version::BytecodeVersion;

/// Parsed GEN8 chunk — game metadata.
#[derive(Debug)]
pub struct Gen8 {
    /// Whether the debugger is disabled.
    pub is_debug_disabled: bool,
    /// Bytecode format version (14, 15, 16, 17, ...).
    pub bytecode_version: BytecodeVersion,
    /// Reference to the filename string.
    pub filename: StringRef,
    /// Reference to the config string.
    pub config: StringRef,
    /// Last object ID + 1.
    pub last_obj: u32,
    /// Last tile ID + 1.
    pub last_tile: u32,
    /// Unique game ID.
    pub game_id: u32,
    /// DirectPlay GUID (16 bytes, usually zeroed).
    pub guid: [u8; 16],
    /// Reference to the game name string.
    pub name: StringRef,
    /// IDE version: major.
    pub major: u32,
    /// IDE version: minor.
    pub minor: u32,
    /// IDE version: release.
    pub release: u32,
    /// IDE version: build.
    pub build: u32,
    /// Default window width in pixels.
    pub default_window_width: u32,
    /// Default window height in pixels.
    pub default_window_height: u32,
    /// Game info flags.
    pub info: u32,
    /// License CRC32.
    pub license_crc32: u32,
    /// License MD5 hash (16 bytes).
    pub license_md5: [u8; 16],
    /// Compilation timestamp (Unix epoch).
    pub timestamp: u64,
    /// Reference to the display name string.
    pub display_name: StringRef,
    /// Active compilation targets (bitfield).
    pub active_targets: u64,
    /// Function classifications (bitfield).
    pub function_classifications: u64,
    /// Steam App ID.
    pub steam_app_id: i32,
    /// Debugger port (bc >= 14).
    pub debugger_port: u32,
    /// Room execution order (list of room indices).
    pub room_order: Vec<u32>,
    /// Raw bytes of GMS2 extension data (Major >= 2), if any.
    pub gms2_data: Vec<u8>,
}

impl Gen8 {
    /// Parse the GEN8 chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    pub fn parse(chunk_data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);

        // First u32: debug(u8) + bytecodeVersion(u8) + padding(u16)
        let is_debug_disabled = c.read_u8()? != 0;
        let bytecode_version = BytecodeVersion(c.read_u8()?);
        let _padding = c.read_u16()?;

        let filename = StringRef(c.read_u32()?);
        let config = StringRef(c.read_u32()?);
        let last_obj = c.read_u32()?;
        let last_tile = c.read_u32()?;
        let game_id = c.read_u32()?;

        let mut guid = [0u8; 16];
        guid.copy_from_slice(c.read_bytes(16)?);

        let name = StringRef(c.read_u32()?);
        let major = c.read_u32()?;
        let minor = c.read_u32()?;
        let release = c.read_u32()?;
        let build = c.read_u32()?;
        let default_window_width = c.read_u32()?;
        let default_window_height = c.read_u32()?;
        let info = c.read_u32()?;
        let license_crc32 = c.read_u32()?;

        let mut license_md5 = [0u8; 16];
        license_md5.copy_from_slice(c.read_bytes(16)?);

        let timestamp = c.read_u64()?;
        let display_name = StringRef(c.read_u32()?);
        let active_targets = c.read_u64()?;
        let function_classifications = c.read_u64()?;
        let steam_app_id = c.read_i32()?;

        let debugger_port = if bytecode_version.0 >= 14 {
            c.read_u32()?
        } else {
            0
        };

        // Room order: u32 count + count × u32
        let room_count = c.read_u32()? as usize;
        let mut room_order = Vec::with_capacity(room_count);
        for _ in 0..room_count {
            room_order.push(c.read_u32()?);
        }

        // GMS2 extension data (Major >= 2): store as raw bytes
        let gms2_data = if major >= 2 && c.remaining() > 0 {
            c.read_bytes(c.remaining())?.to_vec()
        } else {
            Vec::new()
        };

        Ok(Self {
            is_debug_disabled,
            bytecode_version,
            filename,
            config,
            last_obj,
            last_tile,
            game_id,
            guid,
            name,
            major,
            minor,
            release,
            build,
            default_window_width,
            default_window_height,
            info,
            license_crc32,
            license_md5,
            timestamp,
            display_name,
            active_targets,
            function_classifications,
            steam_app_id,
            debugger_port,
            room_order,
            gms2_data,
        })
    }
}
