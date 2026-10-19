//! GAME.FNT: a fixed-height 1-bit bitmap font covering the Unicode BMP.

const HEADER_LEN: usize = 28;
const TABLE_LEN: usize = 65536;

pub struct Font {
    pub height: usize,
    stride: usize,
    glyph_len: usize,
    /// Glyph number plus one for each UTF-16 code unit; zero means no glyph.
    table: Vec<u16>,
    glyphs: Vec<u8>,
}

impl Font {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.get(..4)? != b"fonT" {
            return None;
        }
        let field = |at: usize| -> Option<usize> {
            Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize)
        };
        let stride = field(8)?;
        let height = field(12)?;
        let count = field(20)?;
        let glyph_len = field(24)?;
        if glyph_len != 1 + stride * height {
            return None;
        }
        let table_end = HEADER_LEN + TABLE_LEN * 2;
        let table = data
            .get(HEADER_LEN..table_end)?
            .as_chunks::<2>().0.iter()
            .map(|c| u16::from_le_bytes([c[0], c[1]]))
            .collect();
        let glyphs = data.get(table_end..table_end + count * glyph_len)?.to_vec();
        Some(Self {
            height,
            stride,
            glyph_len,
            table,
            glyphs,
        })
    }

    fn glyph(&self, unit: u16) -> Option<&[u8]> {
        let index = (self.table[unit as usize] as usize).checked_sub(1)?;
        self.glyphs.get(index * self.glyph_len..(index + 1) * self.glyph_len)
    }

    pub fn width(&self, text: &str) -> usize {
        text.encode_utf16()
            .filter_map(|unit| self.glyph(unit))
            .map(|glyph| glyph[0] as usize)
            .sum()
    }

    /// Renders one line of text as a coverage mask: `width * height` bytes of 0 or 255.
    pub fn rasterize(&self, text: &str) -> (usize, Vec<u8>) {
        let width = self.width(text).max(1);
        let mut mask = vec![0u8; width * self.height];
        let mut pen = 0;
        for glyph in text.encode_utf16().filter_map(|unit| self.glyph(unit)) {
            let advance = glyph[0] as usize;
            for row in 0..self.height {
                let bits = &glyph[1 + row * self.stride..][..self.stride];
                for col in 0..advance.min(self.stride * 8) {
                    if bits[col / 8] << (col % 8) & 0x80 != 0 {
                        mask[row * width + pen + col] = 255;
                    }
                }
            }
            pen += advance;
        }
        (width, mask)
    }
}
