//! The depth buffer: which pixels of the map's pictures hide which.
//!
//! Every picture drawn into the map has a depth for each of its pixels, worked
//! out from where the pixel is on screen; smaller is nearer. The ground, the
//! bodies of buildings, bridges, overlays and most terrain objects write their
//! depths; a pixel of anything is drawn only where it is nearer than what is
//! already there. So a unit behind a cliff or a building is cut off exactly
//! where the picture in front covers it.
//!
//! Depths are counted with the scene's row 0 as the origin, so that a flat
//! ground pixel on scene row r at level 0 has depth -r - 1. They go to the GPU
//! as each fragment's depth, all of them behind every plain sprite, which so
//! draws over them as before. The ground is drawn as meshes through
//! [`GroundMaterial`]; everything else with a [`Depth`] is an ordinary sprite
//! drawn through [`DepthMaterial`], in the usual order.
//!
//! `RA2_NODEPTH=1` leaves the sprites plain, for comparing.

use std::collections::HashMap;

use bevy::asset::{uuid_handle, RenderAssetUsages};
use bevy::mesh::{MeshTag, MeshVertexBufferLayoutRef};
use bevy::prelude::*;
use bevy::render::render_resource::{AsBindGroup, CompareFunction, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError, TextureDimension, TextureFormat};
use bevy::shader::ShaderRef;
use bevy::sprite_render::{AlphaMode2d, Material2d, Material2dKey, Material2dPipeline, Material2dPlugin, MaterialExtension2d, SpriteMaterial, SpriteMaterialPlugin};

const SHADER: Handle<Shader> = uuid_handle!("6b0e8f8e-3f5b-4b0e-9a55-1d6a5c2f0d17");
const GROUND_SHADER: Handle<Shader> = uuid_handle!("0d6f3c55-8a51-4f6e-b1d4-2c7e9a4b6f21");

/// Where in the drawing order the pictures go that are drawn before every
/// object: bridges, overlays, and the bodies, bibs and floors of buildings.
/// After the ground and the shadows, before everything else.
pub const FIRST: f32 = 1.9;

/// How the depth of a picture's pixels is worked out.
#[derive(Component, Clone, PartialEq, Debug)]
pub enum Depth {
    /// Standing up from its bottom row: that row has the depth of the ground
    /// under it, moved by `adjust` and rounded down to a multiple of three,
    /// and every three rows up are one nearer.
    Upright { adjust: i32, write: bool },
    /// Lying on the ground: each row has the depth of flat ground there, moved
    /// by `adjust`.
    Flat { adjust: i32, write: bool },
    /// A building's body: the depth of its bottom row, moved by `adjust`, less
    /// the value of the building depth shape under the pixel, the shape's point
    /// `at` pixels from the picture's top-left corner. Pixels off the shape are
    /// not drawn.
    Body { adjust: i32, at: IVec2 },
}

#[derive(Asset, AsBindGroup, Reflect, Clone)]
#[bind_group_data(DepthKey)]
pub struct DepthMaterial {
    /// Mode, the depth shape's offset, and nothing.
    #[uniform(20)]
    params: IVec4,
    #[texture(21, sample_type = "s_int")]
    shape: Handle<Image>,
    #[reflect(ignore)]
    key: DepthKey,
}

#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Default)]
pub struct DepthKey {
    write: bool,
}

impl From<&DepthMaterial> for DepthKey {
    fn from(material: &DepthMaterial) -> Self {
        material.key
    }
}

impl MaterialExtension2d for DepthMaterial {
    fn fragment_shader() -> Option<ShaderRef> {
        Some(ShaderRef::Handle(SHADER))
    }

    fn specialize(_: &Material2dPipeline, descriptor: &mut RenderPipelineDescriptor, _: &MeshVertexBufferLayoutRef, key: Material2dKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(key.bind_group_data.write);
            depth.depth_compare = Some(CompareFunction::Greater);
        }
        Ok(())
    }
}

/// The ground of one atlas page: its tiles' colours and depth bytes. The tiles
/// are drawn as one mesh whose vertex colour's red holds each tile's base depth.
#[derive(Asset, AsBindGroup, TypePath, Clone)]
pub struct GroundMaterial {
    #[texture(0)]
    pub colours: Handle<Image>,
    #[texture(1, sample_type = "u_int")]
    pub heights: Handle<Image>,
}

impl Material2d for GroundMaterial {
    fn fragment_shader() -> ShaderRef {
        ShaderRef::Handle(GROUND_SHADER)
    }

    fn alpha_mode(&self) -> AlphaMode2d {
        AlphaMode2d::Blend
    }

    fn specialize(_: &Material2dPipeline, descriptor: &mut RenderPipelineDescriptor, _: &MeshVertexBufferLayoutRef, _: Material2dKey<Self>) -> Result<(), SpecializedMeshPipelineError> {
        if let Some(depth) = descriptor.depth_stencil.as_mut() {
            depth.depth_write_enabled = Some(true);
            depth.depth_compare = Some(CompareFunction::GreaterEqual);
        }
        Ok(())
    }
}

/// One tile picture on the ground: its atlas page and place in it, where it is
/// drawn in the scene, and its base depth.
pub struct GroundTile {
    pub page: usize,
    pub source: URect,
    pub at: IVec2,
    pub base: i32,
}

/// Builds the meshes that draw the ground, one for each atlas page, in the
/// order the tiles are given.
pub fn ground_meshes(tiles: &[GroundTile], pages: usize, page_size: u32) -> Vec<Mesh> {
    use bevy::mesh::{Indices, PrimitiveTopology};
    let mut meshes = Vec::new();
    for page in 0..pages {
        let (mut positions, mut uvs, mut colours, mut indices): (Vec<[f32; 3]>, Vec<[f32; 2]>, Vec<[f32; 4]>, Vec<u32>) = Default::default();
        for tile in tiles.iter().filter(|tile| tile.page == page) {
            let first = positions.len() as u32;
            let size = tile.source.size().as_ivec2();
            for (dx, dy) in [(0, 0), (1, 0), (1, 1), (0, 1)] {
                let (x, y) = (tile.at.x + dx * size.x, tile.at.y + dy * size.y);
                positions.push([x as f32, -y as f32, 0.0]);
                let (u, v) = (tile.source.min.x + dx as u32 * size.x as u32, tile.source.min.y + dy as u32 * size.y as u32);
                uvs.push([u as f32 / page_size as f32, v as f32 / page_size as f32]);
                colours.push([tile.base as f32, 0.0, 0.0, 1.0]);
            }
            indices.extend([first, first + 2, first + 1, first, first + 3, first + 2]);
        }
        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::RENDER_WORLD);
        mesh.insert_attribute(Mesh::ATTRIBUTE_POSITION, positions);
        mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
        mesh.insert_attribute(Mesh::ATTRIBUTE_COLOR, colours);
        mesh.insert_indices(Indices::U32(indices));
        meshes.push(mesh);
    }
    meshes
}

/// The materials made so far, one for each mode and set of textures, and the
/// building depth shape.
#[derive(Resource)]
pub struct DepthMaterials {
    made: HashMap<(i32, bool, IVec2), Handle<DepthMaterial>>,
    shape: Handle<Image>,
}

impl FromWorld for DepthMaterials {
    fn from_world(world: &mut World) -> Self {
        let (width, height, data) = world.get_resource::<crate::files::GameFiles>().and_then(building_shape).unwrap_or((1, 1, vec![0]));
        let image = Image::new(Extent3d { width, height, depth_or_array_layers: 1 }, TextureDimension::D2, data, TextureFormat::R8Sint, RenderAssetUsages::RENDER_WORLD);
        let shape = world.resource_mut::<Assets<Image>>().add(image);
        DepthMaterials { made: HashMap::new(), shape }
    }
}

/// The building depth shape: its size and its values as signed bytes.
// The file's single frame, with 65 taken from every value that is not 0 as it
// is loaded.
fn building_shape(files: &crate::files::GameFiles) -> Option<(u32, u32, Vec<u8>)> {
    let shp = ra2_formats::shp::Shp::parse(files.get("buildngz.shp")?)?;
    let frame = shp.frames.first()?;
    let (width, height) = (shp.width as usize, shp.height as usize);
    let mut data = vec![0u8; width * height];
    for row in 0..frame.height as usize {
        for col in 0..frame.width as usize {
            let (x, y) = (frame.x as usize + col, frame.y as usize + row);
            if x < width && y < height {
                let raw = frame.pixels[row * frame.width as usize + col];
                data[y * width + x] = if raw == 0 { 0 } else { raw.wrapping_sub(65) };
            }
        }
    }
    Some((width as u32, height as u32, data))
}

/// Gives each picture with a [`Depth`] the material and tag that draw it so.
fn apply_depth(
    mut commands: Commands,
    mut made: ResMut<DepthMaterials>,
    mut materials: ResMut<Assets<DepthMaterial>>,
    changed: Query<(Entity, &Depth, Option<&SpriteMaterial<DepthMaterial>>), Changed<Depth>>,
) {
    let made = &mut *made;
    if std::env::var_os("RA2_NODEPTH").is_some() {
        return;
    }
    for (entity, depth, current) in &changed {
        let (mode, value, write, at) = match depth {
            Depth::Upright { adjust, write } => (1, *adjust, *write, IVec2::ZERO),
            Depth::Flat { adjust, write } => (2, *adjust, *write, IVec2::ZERO),
            Depth::Body { adjust, at } => (3, *adjust, true, *at),
        };
        let shape = made.shape.clone();
        let handle = made
            .made
            .entry((mode, write, at))
            .or_insert_with(|| materials.add(DepthMaterial { params: IVec4::new(mode, at.x, at.y, 0), shape, key: DepthKey { write } }))
            .clone();
        let mut entity = commands.entity(entity);
        entity.insert(MeshTag::new(value as u32));
        if current.is_none_or(|current| current.0 != handle) {
            entity.insert(SpriteMaterial(handle));
        }
    }
}

pub struct DepthPlugin;

impl Plugin for DepthPlugin {
    fn build(&self, app: &mut App) {
        let mut shaders = app.world_mut().resource_mut::<Assets<Shader>>();
        shaders.insert(SHADER.id(), Shader::from_wesl(include_str!("depth.wesl"), "ra2/depth.wesl")).ok();
        shaders.insert(GROUND_SHADER.id(), Shader::from_wesl(include_str!("ground.wesl"), "ra2/ground.wesl")).ok();
        app.add_plugins((SpriteMaterialPlugin::<DepthMaterial>::default(), Material2dPlugin::<GroundMaterial>::default()))
            .init_resource::<DepthMaterials>()
            .add_systems(PostUpdate, apply_depth);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[cfg_attr(not(game_files), ignore = "needs the game's files")]
    fn the_building_depth_shape_is_nearest_up_its_middle() {
        let files = crate::files::GameFiles::open().expect("the game's files");
        let (width, height, data) = building_shape(&files).expect("buildngz.shp");
        assert_eq!((width, height), (396, 477));
        let at = |x: usize, y: usize| data[y * width as usize + x] as i8;
        assert_eq!(at(198, 446), 9);
        assert!(at(150, 446) < at(198, 446) && at(198, 446) < at(198, 400));
    }
}
