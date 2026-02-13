use crate::cursor::Cursor;
use crate::error::Result;

/// A texture page item in the TPAG chunk.
///
/// Describes a rectangular region on a texture atlas page.
#[derive(Debug, Clone)]
pub struct TexturePageItem {
    /// Source X position on the texture page.
    pub source_x: u16,
    /// Source Y position on the texture page.
    pub source_y: u16,
    /// Source width on the texture page.
    pub source_width: u16,
    /// Source height on the texture page.
    pub source_height: u16,
    /// Target X offset when rendering.
    pub target_x: u16,
    /// Target Y offset when rendering.
    pub target_y: u16,
    /// Target (bounding) width.
    pub target_width: u16,
    /// Target (bounding) height.
    pub target_height: u16,
    /// Render width (original sprite width).
    pub render_width: u16,
    /// Render height (original sprite height).
    pub render_height: u16,
    /// Index into the TXTR chunk (which texture atlas page).
    pub texture_page_id: u16,
}

/// Parsed TPAG chunk.
#[derive(Debug)]
pub struct Tpag {
    /// Texture page items.
    pub items: Vec<TexturePageItem>,
}

impl Tpag {
    /// Size of a single TPAG entry in bytes.
    const ENTRY_SIZE: usize = 22;

    /// Parse the TPAG chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    /// `data` is the full file data (for following absolute pointers).
    pub fn parse(chunk_data: &[u8], data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let pointers = c.read_pointer_list()?;

        let mut items = Vec::with_capacity(pointers.len());
        for ptr in pointers {
            let mut ec = Cursor::new(data);
            ec.seek(ptr as usize);

            let source_x = ec.read_u16()?;
            let source_y = ec.read_u16()?;
            let source_width = ec.read_u16()?;
            let source_height = ec.read_u16()?;
            let target_x = ec.read_u16()?;
            let target_y = ec.read_u16()?;
            let target_width = ec.read_u16()?;
            let target_height = ec.read_u16()?;
            let render_width = ec.read_u16()?;
            let render_height = ec.read_u16()?;
            let texture_page_id = ec.read_u16()?;

            items.push(TexturePageItem {
                source_x,
                source_y,
                source_width,
                source_height,
                target_x,
                target_y,
                target_width,
                target_height,
                render_width,
                render_height,
                texture_page_id,
            });
        }

        Ok(Self { items })
    }

    /// Entry size in bytes (useful for serialization).
    pub fn entry_size() -> usize {
        Self::ENTRY_SIZE
    }
}
