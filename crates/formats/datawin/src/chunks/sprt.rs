use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;

/// A sprite entry in the SPRT chunk.
#[derive(Debug)]
pub struct SpriteEntry {
    /// Reference to the sprite name string.
    pub name: StringRef,
    /// Sprite width in pixels.
    pub width: u32,
    /// Sprite height in pixels.
    pub height: u32,
    /// Bounding box left.
    pub bbox_left: i32,
    /// Bounding box right.
    pub bbox_right: i32,
    /// Bounding box bottom.
    pub bbox_bottom: i32,
    /// Bounding box top.
    pub bbox_top: i32,
    /// Whether the sprite is transparent.
    pub transparent: bool,
    /// Whether to smooth edges.
    pub smooth: bool,
    /// Whether to preload the sprite.
    pub preload: bool,
    /// Bounding box mode.
    pub bbox_mode: u32,
    /// Collision checking mode (precise/rectangle).
    pub sep_masks: u32,
    /// Origin X.
    pub origin_x: i32,
    /// Origin Y.
    pub origin_y: i32,
    /// Texture page item indices (one per animation frame).
    pub tpag_indices: Vec<u32>,
}

/// Parsed SPRT chunk.
#[derive(Debug)]
pub struct Sprt {
    /// Sprite entries.
    pub sprites: Vec<SpriteEntry>,
}

impl Sprt {
    /// Parse the SPRT chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    /// `data` is the full file data (for following absolute pointers).
    pub fn parse(chunk_data: &[u8], data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let pointers = c.read_pointer_list()?;

        let mut sprites = Vec::with_capacity(pointers.len());
        for ptr in pointers {
            let sprite = Self::parse_sprite(data, ptr as usize)?;
            sprites.push(sprite);
        }

        Ok(Self { sprites })
    }

    fn parse_sprite(data: &[u8], offset: usize) -> Result<SpriteEntry> {
        let mut c = Cursor::new(data);
        c.seek(offset);

        let name = StringRef(c.read_u32()?);
        let width = c.read_u32()?;
        let height = c.read_u32()?;
        let bbox_left = c.read_i32()?;
        let bbox_right = c.read_i32()?;
        let bbox_bottom = c.read_i32()?;
        let bbox_top = c.read_i32()?;
        let transparent = c.read_u32()? != 0;
        let smooth = c.read_u32()? != 0;
        let preload = c.read_u32()? != 0;
        let bbox_mode = c.read_u32()?;
        let sep_masks = c.read_u32()?;
        let origin_x = c.read_i32()?;
        let origin_y = c.read_i32()?;

        // Texture page item count + TPAG pointers
        let tpag_count = c.read_i32()?;
        if tpag_count < 0 {
            // -1 means no TPAG entries (e.g., GMS2 spine sprites)
            return Ok(SpriteEntry {
                name,
                width,
                height,
                bbox_left,
                bbox_right,
                bbox_bottom,
                bbox_top,
                transparent,
                smooth,
                preload,
                bbox_mode,
                sep_masks,
                origin_x,
                origin_y,
                tpag_indices: Vec::new(),
            });
        }

        let mut tpag_indices = Vec::with_capacity(tpag_count as usize);
        for _ in 0..tpag_count {
            tpag_indices.push(c.read_u32()?);
        }

        Ok(SpriteEntry {
            name,
            width,
            height,
            bbox_left,
            bbox_right,
            bbox_bottom,
            bbox_top,
            transparent,
            smooth,
            preload,
            bbox_mode,
            sep_masks,
            origin_x,
            origin_y,
            tpag_indices,
        })
    }
}
