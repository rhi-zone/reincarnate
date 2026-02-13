use crate::cursor::Cursor;
use crate::error::Result;
use crate::string_table::StringRef;

/// A glyph entry within a font.
#[derive(Debug, Clone)]
pub struct Glyph {
    /// Unicode character code.
    pub character: u16,
    /// Source X on the texture page.
    pub x: u16,
    /// Source Y on the texture page.
    pub y: u16,
    /// Width of the glyph on the texture.
    pub width: u16,
    /// Height of the glyph on the texture.
    pub height: u16,
    /// Horizontal shift when rendering.
    pub shift: i16,
    /// Horizontal advance after rendering.
    pub offset: i16,
}

/// A font entry in the FONT chunk.
#[derive(Debug)]
pub struct FontEntry {
    /// Reference to the font name string (code name).
    pub name: StringRef,
    /// Reference to the display name string.
    pub display_name: StringRef,
    /// Font size in points.
    pub size: u32,
    /// Whether the font is bold.
    pub bold: bool,
    /// Whether the font is italic.
    pub italic: bool,
    /// First character code in the range.
    pub range_start: u16,
    /// Character set/codepage.
    pub charset: u8,
    /// Anti-alias level.
    pub antialias: u8,
    /// Last character code in the range.
    pub range_end: u32,
    /// Texture page item index for this font's texture.
    pub tpag_index: u32,
    /// Scale factors (x, y). Both typically 1.0.
    pub scale_x: f32,
    pub scale_y: f32,
    /// Glyph definitions.
    pub glyphs: Vec<Glyph>,
}

/// Parsed FONT chunk.
#[derive(Debug)]
pub struct Font {
    /// Font entries.
    pub fonts: Vec<FontEntry>,
}

impl Font {
    /// Parse the FONT chunk.
    ///
    /// `chunk_data` is the raw chunk content (after the 8-byte header).
    /// `data` is the full file data (for following absolute pointers).
    pub fn parse(chunk_data: &[u8], data: &[u8]) -> Result<Self> {
        let mut c = Cursor::new(chunk_data);
        let pointers = c.read_pointer_list()?;

        let mut fonts = Vec::with_capacity(pointers.len());
        for ptr in pointers {
            let font = Self::parse_font(data, ptr as usize)?;
            fonts.push(font);
        }

        Ok(Self { fonts })
    }

    fn parse_font(data: &[u8], offset: usize) -> Result<FontEntry> {
        let mut c = Cursor::new(data);
        c.seek(offset);

        let name = StringRef(c.read_u32()?);
        let display_name = StringRef(c.read_u32()?);
        let size = c.read_u32()?;
        let bold = c.read_u32()? != 0;
        let italic = c.read_u32()? != 0;
        let range_start = c.read_u16()?;
        let charset = c.read_u8()?;
        let antialias = c.read_u8()?;
        let range_end = c.read_u32()?;
        let tpag_index = c.read_u32()?;
        let scale_x = c.read_f32()?;
        let scale_y = c.read_f32()?;

        // Glyph pointer list
        let glyph_ptrs = c.read_pointer_list()?;
        let mut glyphs = Vec::with_capacity(glyph_ptrs.len());
        for gp in glyph_ptrs {
            let mut gc = Cursor::new(data);
            gc.seek(gp as usize);
            let character = gc.read_u16()?;
            let x = gc.read_u16()?;
            let y = gc.read_u16()?;
            let width = gc.read_u16()?;
            let height = gc.read_u16()?;
            let shift = gc.read_i16()?;
            let offset = gc.read_i16()?;
            glyphs.push(Glyph {
                character,
                x,
                y,
                width,
                height,
                shift,
                offset,
            });
        }

        Ok(FontEntry {
            name,
            display_name,
            size,
            bold,
            italic,
            range_start,
            charset,
            antialias,
            range_end,
            tpag_index,
            scale_x,
            scale_y,
            glyphs,
        })
    }
}
