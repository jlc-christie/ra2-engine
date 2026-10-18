//! House colours: turning a rules.ini `[Colors]` entry into the sixteen remap
//! shades that replace palette indices 16 to 31 on anything a house owns.

use std::collections::HashMap;

use ra2_formats::ini::Ini;
use ra2_formats::map::Map;
use ra2_formats::pal::Palette;

/// First palette index of the remap range.
const REMAP_START: usize = 16;
const REMAP_COUNT: usize = 16;

/// Entries in the engine's sine table, one full turn.
const TABLE_SIZE: i32 = 8192;

/// The engine's table-driven sine.
// The angle is scaled to 16384 steps per turn and truncated, halved rounding odd
// values up, and looked up in an 8192-entry table of single-precision sines.
fn table_sin(angle: f64, quarter_turns: i32) -> f64 {
    let steps = (angle * 2607.594_482_421_875) as i32;
    let mut index = (steps / 2).rem_euclid(TABLE_SIZE) + quarter_turns * (TABLE_SIZE / 4);
    if steps & 1 != 0 {
        index += 1;
    }
    (index as f32 * (std::f32::consts::TAU / TABLE_SIZE as f32)).sin() as f64
}

/// HSV to RGB, each 0 to 255, in the engine's integer arithmetic.
pub(crate) fn hsv_to_rgb(hsv: [u8; 3]) -> [u8; 3] {
    let [h, s, v] = hsv.map(u32::from);
    let fraction = (h * 6) % 255;
    let p = (255 - s) * v / 255;
    let q = (255 - fraction * s / 255) * v / 255;
    let t = (255 - (255 - fraction) * s / 255) * v / 255;
    let (r, g, b) = match (h * 6) / 255 {
        0 | 6 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    [r as u8, g as u8, b as u8]
}

/// The sixteen remap shades of a colour, brightest first.
// Shade `i` keeps the hue, scales saturation by the
// sine of an angle rising from 50 to 90 degrees and value by the cosine of an angle
// rising from 20 to 90 degrees (11.25 degrees for the first shade), truncating both.
pub fn remap_shades(hsv: [u8; 3]) -> [[u8; 3]; REMAP_COUNT] {
    std::array::from_fn(|i| {
        let step = i as f64;
        let value_angle = if i == 0 {
            0.196_349_540_849_362_07
        } else {
            step * 0.081_448_698_426_402_04 + 0.349_065_850_398_865_9
        };
        let saturation_angle = step * 0.046_542_113_386_515_455 + 0.872_664_625_997_164_8;
        let saturation = (table_sin(saturation_angle, 0) * hsv[1] as f64) as u8;
        let value = (table_sin(value_angle, 1) * hsv[2] as f64) as u8;
        hsv_to_rgb([hsv[0], saturation, value])
    })
}

/// Remapped copies of a base palette, one per colour in use.
pub struct HouseColors {
    colors: HashMap<String, [u8; 3]>,
    palettes: Vec<Palette>,
    by_color: HashMap<String, usize>,
}

impl HouseColors {
    pub fn new(rules: Option<&Ini>) -> Self {
        // DATA: rules.ini `[Colors]`, each `name=hue,saturation,value`.
        let colors = rules
            .map(|rules| rules.section("Colors"))
            .unwrap_or(&[])
            .iter()
            .filter_map(|(name, value)| {
                let parts: Vec<u8> = value.split(',').filter_map(|n| n.trim().parse().ok()).collect();
                Some((name.to_uppercase(), [*parts.first()?, *parts.get(1)?, *parts.get(2)?]))
            })
            .collect();
        Self {
            colors,
            palettes: Vec::new(),
            by_color: HashMap::new(),
        }
    }

    /// The index of the palette for things owned by `owner`, if its colour is known.
    // DATA: an owner's colour is the `Color` key of its section in the map, or
    // failing that in rules.ini. The engine's scheme set-up is handed both that
    // and the plain theatre palette.
    pub fn palette_for(&mut self, owner: &str, map: &Map, rules: Option<&Ini>, base: &Palette) -> Option<usize> {
        let name = map
            .ini
            .get(owner, "Color")
            .or_else(|| rules?.get(owner, "Color"))?
            .to_uppercase();
        self.palette_for_color(&name, base)
    }

    /// The index of the palette for a `[Colors]` entry, by name.
    pub fn palette_for_color(&mut self, color: &str, base: &Palette) -> Option<usize> {
        let name = color.to_uppercase();
        if let Some(&index) = self.by_color.get(&name) {
            return Some(index);
        }
        let hsv = *self.colors.get(&name)?;
        let mut colors = base.0;
        colors[REMAP_START..REMAP_START + REMAP_COUNT].copy_from_slice(&remap_shades(hsv));
        self.palettes.push(Palette(colors));
        self.by_color.insert(name, self.palettes.len() - 1);
        Some(self.palettes.len() - 1)
    }

    /// Copies of every palette made so far, indexed as `palette_for` returned them.
    pub fn palettes(&self) -> Vec<Palette> {
        self.palettes.iter().map(|palette| Palette(palette.0)).collect()
    }

    pub fn palette(&self, index: usize) -> &Palette {
        &self.palettes[index]
    }
}
