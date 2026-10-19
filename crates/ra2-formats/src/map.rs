//! Map files: an INI document whose terrain is stored in packed, encoded sections.

use base64::Engine;

use crate::ini::Ini;

/// One terrain cell from `[IsoMapPack5]`.
#[derive(Clone, Copy, Debug)]
pub struct IsoTile {
    pub x: i16,
    pub y: i16,
    /// Index into the theatre's tile list.
    pub tile: i32,
    /// Which cell of the tile's template this is.
    pub sub_tile: u8,
    /// Height level.
    pub level: u8,
}

/// An object from `[Terrain]`, such as a tree.
#[derive(Clone, Debug)]
pub struct TerrainObject {
    pub x: i32,
    pub y: i32,
    pub name: String,
}

/// A building from `[Structures]`.
#[derive(Clone, Debug)]
pub struct Structure {
    pub owner: String,
    pub kind: String,
    /// 0 to 256.
    pub health: i32,
    /// The top cell of its foundation.
    pub x: i32,
    pub y: i32,
}

/// A foot soldier, civilian or animal from `[Infantry]`.
#[derive(Clone, Debug)]
pub struct Infantry {
    pub owner: String,
    pub kind: String,
    pub health: i32,
    pub x: i32,
    pub y: i32,
    /// Which spot within the cell it stands on.
    pub sub_cell: u8,
    /// 0 to 255, clockwise from north.
    pub facing: u8,
}

/// A vehicle from `[Units]`.
#[derive(Clone, Debug)]
pub struct Unit {
    pub owner: String,
    pub kind: String,
    pub health: i32,
    pub x: i32,
    pub y: i32,
    /// 0 to 255, clockwise from north.
    pub facing: u8,
}

/// Overlays are stored for a fixed 512x512 grid of cells.
pub const OVERLAY_GRID: usize = 512;
/// The overlay type index meaning "no overlay".
pub const NO_OVERLAY: u8 = 0xFF;

pub struct Map {
    pub ini: Ini,
    /// Map width and height in cells, from `[Map] Size`.
    pub width: i32,
    pub height: i32,
    pub theater: String,
    pub tiles: Vec<IsoTile>,
    pub terrain_objects: Vec<TerrainObject>,
    pub structures: Vec<Structure>,
    pub infantry: Vec<Infantry>,
    pub units: Vec<Unit>,
    /// Overlay type index per cell of the overlay grid; empty if the map has none.
    overlay: Vec<u8>,
    /// Per-cell overlay state, such as the amount of ore.
    overlay_data: Vec<u8>,
}

const ISO_TILE_LEN: usize = 11;

/// Joins the numbered lines of a packed section and decodes them from base64.
fn packed_section(ini: &Ini, section: &str) -> Option<Vec<u8>> {
    let text: String = ini
        .section(section)
        .iter()
        .map(|(_, value)| value.as_str())
        .collect();
    base64::engine::general_purpose::STANDARD.decode(text).ok()
}

/// Decompresses a sequence of `(compressed length, plain length, data)` blocks.
fn blocks(
    mut data: &[u8],
    decompress: impl Fn(&[u8], usize) -> Option<Vec<u8>>,
) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    while data.len() >= 4 {
        let packed = u16::from_le_bytes([data[0], data[1]]) as usize;
        let plain = u16::from_le_bytes([data[2], data[3]]) as usize;
        out.extend(decompress(data.get(4..4 + packed)?, plain)?);
        data = &data[4 + packed..];
    }
    Some(out)
}

fn lzo_blocks(data: &[u8]) -> Option<Vec<u8>> {
    blocks(data, |block, plain| lzo::decompress(block, plain).ok())
}

/// The picture of a map stored in its file: width, height, and three bytes (red,
/// green, blue) a pixel from the top row down.
// `[Preview] Size=x,y,w,h`
// and `[PreviewPack]`, base64 of LZO blocks.
pub fn preview(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let ini = Ini::parse(data);
    let size: Vec<u32> = ini.get("Preview", "Size")?.split(',').filter_map(|n| n.trim().parse().ok()).collect();
    let (&w, &h) = (size.get(2)?, size.get(3)?);
    let pixels = lzo_blocks(&packed_section(&ini, "PreviewPack")?)?;
    (w > 0 && h > 0 && pixels.len() >= (w * h * 3) as usize).then_some((w, h, pixels))
}

/// A packed overlay section, or an empty grid if it is absent or malformed.
fn overlay_grid(ini: &Ini, section: &str) -> Vec<u8> {
    packed_section(ini, section)
        .and_then(|data| blocks(&data, crate::lcw::decompress))
        .filter(|grid| grid.len() == OVERLAY_GRID * OVERLAY_GRID)
        .unwrap_or_default()
}

impl Map {
    pub fn parse(data: &[u8]) -> Option<Self> {
        let ini = Ini::parse(data);
        let size: Vec<i32> = ini
            .get("Map", "Size")?
            .split(',')
            .filter_map(|n| n.trim().parse().ok())
            .collect();
        let (width, height) = (*size.get(2)?, *size.get(3)?);
        let theater = ini.get("Map", "Theater")?.to_uppercase();

        let raw = lzo_blocks(&packed_section(&ini, "IsoMapPack5")?)?;
        let tiles = raw
            .as_chunks::<ISO_TILE_LEN>().0.iter()
            .map(|r| IsoTile {
                x: i16::from_le_bytes([r[0], r[1]]),
                y: i16::from_le_bytes([r[2], r[3]]),
                tile: i32::from_le_bytes([r[4], r[5], r[6], r[7]]),
                sub_tile: r[8],
                level: r[9],
            })
            // The final block is padded with empty records.
            .filter(|tile| tile.x > 0 && tile.y > 0)
            .collect();

        // Trees land on road medians with this.
        let terrain_objects = ini
            .section("Terrain")
            .iter()
            .filter_map(|(key, name)| {
                let cell: i32 = key.parse().ok()?;
                Some(TerrainObject {
                    x: cell % 1000,
                    y: cell / 1000,
                    name: name.clone(),
                })
            })
            .collect();
        // Each entry is `owner,type,health,x,y,facing,tag,...`.
        let structures = ini
            .section("Structures")
            .iter()
            .filter_map(|(_, value)| {
                let fields: Vec<&str> = value.split(',').map(str::trim).collect();
                Some(Structure {
                    owner: fields.first()?.to_string(),
                    kind: fields.get(1)?.to_string(),
                    health: fields.get(2)?.parse().ok()?,
                    x: fields.get(3)?.parse().ok()?,
                    y: fields.get(4)?.parse().ok()?,
                })
            })
            .collect();
        // `owner,type,health,x,y,sub cell,mission,facing,tag,veterancy,group,...`.
        let infantry = ini
            .section("Infantry")
            .iter()
            .filter_map(|(_, value)| {
                let fields: Vec<&str> = value.split(',').map(str::trim).collect();
                Some(Infantry {
                    owner: fields.first()?.to_string(),
                    kind: fields.get(1)?.to_string(),
                    health: fields.get(2)?.parse().ok()?,
                    x: fields.get(3)?.parse().ok()?,
                    y: fields.get(4)?.parse().ok()?,
                    sub_cell: fields.get(5)?.parse().ok()?,
                    facing: fields.get(7)?.parse().ok()?,
                })
            })
            .collect();
        // `owner,type,health,x,y,facing,mission,tag,veterancy,group,...`.
        let units = ini
            .section("Units")
            .iter()
            .filter_map(|(_, value)| {
                let fields: Vec<&str> = value.split(',').map(str::trim).collect();
                Some(Unit {
                    owner: fields.first()?.to_string(),
                    kind: fields.get(1)?.to_string(),
                    health: fields.get(2)?.parse().ok()?,
                    x: fields.get(3)?.parse().ok()?,
                    y: fields.get(4)?.parse().ok()?,
                    facing: fields.get(5)?.parse().ok()?,
                })
            })
            .collect();
        let overlay = overlay_grid(&ini, "OverlayPack");
        let overlay_data = overlay_grid(&ini, "OverlayDataPack");

        Some(Self {
            structures,
            infantry,
            units,
            terrain_objects,
            overlay,
            overlay_data,
            ini,
            width,
            height,
            theater,
            tiles,
        })
    }
}

impl Map {
    /// The raw overlay type and state bytes stored for a cell, whether or not it has
    /// an overlay.
    pub fn overlay_bytes(&self, x: i32, y: i32) -> Option<(u8, u8)> {
        let index = usize::try_from(y).ok()? * OVERLAY_GRID + usize::try_from(x).ok()?;
        Some((*self.overlay.get(index)?, *self.overlay_data.get(index)?))
    }

    /// The overlay type index and state at a cell, if it has an overlay.
    // Both packs are read row by row, 512 cells per
    // row, so a cell's index is `y * 512 + x`; the state is stored as read.
    pub fn overlay_at(&self, x: i32, y: i32) -> Option<(u8, u8)> {
        let grid = OVERLAY_GRID as i32;
        if x < 0 || y < 0 || x >= grid || y >= grid {
            return None;
        }
        let index = (y * grid + x) as usize;
        let kind = *self.overlay.get(index)?;
        (kind != NO_OVERLAY).then(|| (kind, self.overlay_data.get(index).copied().unwrap_or(0)))
    }
}
