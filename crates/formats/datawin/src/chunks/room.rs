use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;

/// A room entry in the ROOM chunk.
///
/// Rooms are complex structures with many sub-lists (backgrounds, views,
/// objects, tiles, layers). We parse the core fields and leave sub-structures
/// as raw pointer lists for now.
#[derive(Debug)]
pub struct RoomEntry {
    /// Reference to the room name string.
    pub name: StringRef,
    /// Reference to the room caption string.
    pub caption: StringRef,
    /// Room width in pixels.
    pub width: u32,
    /// Room height in pixels.
    pub height: u32,
    /// Room speed (frames per second or microseconds per frame).
    pub speed: u32,
    /// Whether the room is persistent.
    pub persistent: bool,
    /// Background color.
    pub background_color: u32,
    /// Whether to draw the background color.
    pub draw_background_color: bool,
    /// Creation code entry index into the CODE chunk, or -1.
    pub creation_code_id: i32,
    /// Room flags.
    pub flags: u32,
    /// Physics world properties.
    pub physics_world: bool,
    pub physics_gravity_x: f32,
    pub physics_gravity_y: f32,
    pub physics_pixels_to_meters: f32,
}

/// Parsed ROOM chunk.
#[derive(Debug)]
pub struct Room {
    /// Room entries.
    pub rooms: Vec<RoomEntry>,
}

impl Room {
    /// Parse the ROOM chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    /// `data` is the full file data (for following absolute pointers).
    pub fn parse(chunk_data: &[u8], data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let pointers = c.read_pointer_list()?;

        let mut rooms = Vec::with_capacity(pointers.len());
        for ptr in pointers {
            let room = Self::parse_room(data, ptr as usize)?;
            rooms.push(room);
        }

        Ok(Self { rooms })
    }

    fn parse_room(data: &[u8], offset: usize) -> Result<RoomEntry> {
        let mut c = Cursor::new(data);
        c.seek(offset);

        let name = StringRef(c.read_u32()?);
        let caption = StringRef(c.read_u32()?);
        let width = c.read_u32()?;
        let height = c.read_u32()?;
        let speed = c.read_u32()?;
        let persistent = c.read_u32()? != 0;
        let background_color = c.read_u32()?;
        let draw_background_color = c.read_u32()? != 0;
        let creation_code_id = c.read_i32()?;
        let flags = c.read_u32()?;

        // Backgrounds pointer, Views pointer, Objects pointer, Tiles pointer
        // (we skip these sub-lists for now)
        let _bg_ptr = c.read_u32()?;
        let _views_ptr = c.read_u32()?;
        let _objs_ptr = c.read_u32()?;
        let _tiles_ptr = c.read_u32()?;

        let physics_world = c.read_u32()? != 0;
        // Physics top/left/right/bottom or gravity_x/gravity_y
        let _physics_top = c.read_u32()?;
        let _physics_left = c.read_u32()?;
        let _physics_right = c.read_u32()?;
        let _physics_bottom = c.read_u32()?;
        let physics_gravity_x = c.read_f32()?;
        let physics_gravity_y = c.read_f32()?;
        let physics_pixels_to_meters = c.read_f32()?;

        Ok(RoomEntry {
            name,
            caption,
            width,
            height,
            speed,
            persistent,
            background_color,
            draw_background_color,
            creation_code_id,
            flags,
            physics_world,
            physics_gravity_x,
            physics_gravity_y,
            physics_pixels_to_meters,
        })
    }
}
