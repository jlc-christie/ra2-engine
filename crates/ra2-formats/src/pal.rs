//! 256-colour palettes, stored as 6-bit RGB triples.

pub struct Palette(pub [[u8; 3]; 256]);

impl Palette {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 768 {
            return None;
        }
        let mut colors = [[0u8; 3]; 256];
        for (color, raw) in colors.iter_mut().zip(data.as_chunks::<3>().0) {
            // The theatre palettes are expanded from 6 to 8 bits by shifting left
            // two, so full brightness is 252, not 255.
            *color = [raw[0], raw[1], raw[2]].map(|c| c << 2);
        }
        Some(Self(colors))
    }
}
