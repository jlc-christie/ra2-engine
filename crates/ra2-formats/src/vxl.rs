//! VXL voxel models, their HVA limb transforms and the VPL shading table.

/// A 3x4 transform, row by row: rotation and scale in the first three columns,
/// translation in the fourth.
pub type Matrix = [[f32; 4]; 3];

pub struct Voxel {
    pub x: u8,
    pub y: u8,
    pub z: u8,
    /// Palette index.
    pub color: u8,
    /// Index into the limb's normals table.
    pub normal: u8,
}

pub struct Limb {
    pub name: String,
    /// Scale applied to HVA translations.
    pub scale: f32,
    pub min: [f32; 3],
    pub max: [f32; 3],
    pub size: [u8; 3],
    /// Which normals table the voxels' normal indices refer to.
    pub normals_type: u8,
    pub voxels: Vec<Voxel>,
}

pub struct Vxl {
    pub limbs: Vec<Limb>,
}

const HEADER_LEN: usize = 802;
const LIMB_HEADER_LEN: usize = 28;
const LIMB_TAILER_LEN: usize = 92;

fn u32_at(data: &[u8], at: usize) -> Option<usize> {
    Some(u32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?) as usize)
}

fn f32_at(data: &[u8], at: usize) -> Option<f32> {
    Some(f32::from_le_bytes(data.get(at..at + 4)?.try_into().ok()?))
}

fn name_at(data: &[u8], at: usize) -> Option<String> {
    let raw = data.get(at..at + 16)?;
    let end = raw.iter().position(|&b| b == 0).unwrap_or(16);
    Some(String::from_utf8_lossy(&raw[..end]).into_owned())
}

impl Vxl {
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.get(..16)? != b"Voxel Animation\0" {
            return None;
        }
        let limb_count = u32_at(data, 20)?;
        let body_len = u32_at(data, 28)?;
        let body = HEADER_LEN + limb_count * LIMB_HEADER_LEN;
        let tailers = body + body_len;

        let mut limbs = Vec::with_capacity(limb_count);
        for index in 0..limb_count {
            let tailer = data.get(tailers + index * LIMB_TAILER_LEN..)?.get(..LIMB_TAILER_LEN)?;
            let starts = body + u32_at(tailer, 0)?;
            let spans = body + u32_at(tailer, 8)?;
            let vector = |at: usize| -> Option<[f32; 3]> {
                Some([f32_at(tailer, at)?, f32_at(tailer, at + 4)?, f32_at(tailer, at + 8)?])
            };
            let size = [tailer[88], tailer[89], tailer[90]];

            // One column of voxels per (x, y): runs of voxels separated by gaps.
            let mut voxels = Vec::new();
            for column in 0..size[0] as usize * size[1] as usize {
                let start = u32_at(data, starts + column * 4)? as u32;
                if start == u32::MAX {
                    continue;
                }
                let mut at = spans + start as usize;
                let mut z = 0usize;
                while z < size[2] as usize {
                    z += *data.get(at)? as usize;
                    let count = *data.get(at + 1)? as usize;
                    at += 2;
                    for _ in 0..count {
                        voxels.push(Voxel {
                            x: (column % size[0] as usize) as u8,
                            y: (column / size[0] as usize) as u8,
                            z: z as u8,
                            color: *data.get(at)?,
                            normal: *data.get(at + 1)?,
                        });
                        at += 2;
                        z += 1;
                    }
                    // Each run ends with its count repeated.
                    at += 1;
                }
            }

            limbs.push(Limb {
                name: name_at(data, HEADER_LEN + index * LIMB_HEADER_LEN)?,
                scale: f32_at(tailer, 12)?,
                min: vector(64)?,
                max: vector(76)?,
                size,
                normals_type: tailer[91],
                voxels,
            });
        }
        Some(Self { limbs })
    }
}

/// Per-frame transforms for each limb of a model.
pub struct Hva {
    pub limb_names: Vec<String>,
    /// `frames[frame][limb]`.
    pub frames: Vec<Vec<Matrix>>,
}

impl Hva {
    pub fn parse(data: &[u8]) -> Option<Self> {
        let frame_count = u32_at(data, 16)?;
        let limb_count = u32_at(data, 20)?;
        let limb_names = (0..limb_count)
            .map(|i| name_at(data, 24 + i * 16))
            .collect::<Option<Vec<_>>>()?;
        let mut at = 24 + limb_count * 16;
        let mut frames = Vec::with_capacity(frame_count);
        for _ in 0..frame_count {
            let mut limbs = Vec::with_capacity(limb_count);
            for _ in 0..limb_count {
                let mut matrix = [[0.0f32; 4]; 3];
                for row in &mut matrix {
                    for cell in row.iter_mut() {
                        *cell = f32_at(data, at)?;
                        at += 4;
                    }
                }
                limbs.push(matrix);
            }
            frames.push(limbs);
        }
        Some(Self { limb_names, frames })
    }
}

/// VOXELS.VPL: for each of a number of brightness sections, the palette index a
/// voxel's colour becomes.
pub struct Vpl {
    pub sections: Vec<[u8; 256]>,
}

impl Vpl {
    pub fn parse(data: &[u8]) -> Option<Self> {
        let count = u32_at(data, 8)?;
        let tables = data.get(16 + 768..)?;
        let sections: Vec<[u8; 256]> = tables.as_chunks::<256>().0.iter().take(count).copied().collect();
        (sections.len() == count && count > 0).then_some(Self { sections })
    }
}
