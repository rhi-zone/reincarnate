use crate::cursor::Writer;
use crate::reader::ChunkIndex;

/// A chunk to be written, either from raw bytes or serialized data.
pub struct OutputChunk {
    /// 4-byte ASCII magic.
    pub magic: [u8; 4],
    /// Chunk content (without the 8-byte header).
    pub data: Vec<u8>,
}

/// Assemble a FORM file from a list of chunks.
///
/// Returns the complete file bytes including the FORM envelope.
pub fn assemble_form(chunks: &[OutputChunk]) -> Vec<u8> {
    // Calculate total FORM content size
    let content_size: usize = chunks.iter().map(|c| 8 + c.data.len()).sum();

    let mut w = Writer::with_capacity(8 + content_size);
    w.write_magic(b"FORM");
    w.write_u32(content_size as u32);

    for chunk in chunks {
        w.write_magic(&chunk.magic);
        w.write_u32(chunk.data.len() as u32);
        w.write_bytes(&chunk.data);
    }

    w.into_bytes()
}

/// Extract all chunks from a parsed file as `OutputChunk`s.
///
/// This enables a raw round-trip: read → extract → assemble → identical bytes.
pub fn extract_chunks(index: &ChunkIndex, data: &[u8]) -> Vec<OutputChunk> {
    index
        .chunks()
        .iter()
        .map(|entry| {
            let start = entry.data_offset();
            let end = start + entry.size;
            OutputChunk {
                magic: entry.magic,
                data: data[start..end].to_vec(),
            }
        })
        .collect()
}
