//! A loading screen stays up until the map's textures reach the renderer.

use std::sync::{Arc, atomic::{AtomicBool, Ordering}};
use bevy::prelude::*;
use bevy::render::{
    Render, RenderApp, RenderSystems,
    extract_resource::{ExtractResource, ExtractResourcePlugin},
    render_asset::RenderAssets,
    texture::GpuImage,
};

#[derive(Resource, Clone, ExtractResource)]
#[extract_app(RenderApp)]
pub(crate) struct Textures {
    ids: Vec<AssetId<Image>>,
    ready: Arc<AtomicBool>,
}

impl Textures {
    pub fn watch(images: &Assets<Image>) -> Self {
        Self { ids: images.ids().collect(), ready: Arc::new(AtomicBool::new(false)) }
    }

    pub fn ready(&self) -> bool { self.ready.load(Ordering::Acquire) }
}

pub struct LoadingPlugin;

impl Plugin for LoadingPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(ExtractResourcePlugin::<Textures>::default());
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(Render, ready.in_set(RenderSystems::PrepareBindGroups));
        }
    }
}

fn ready(textures: Option<Res<Textures>>, images: Res<RenderAssets<GpuImage>>) {
    let Some(textures) = textures else { return };
    if !textures.ready() && textures.ids.iter().all(|id| images.get(*id).is_some()) {
        textures.ready.store(true, Ordering::Release);
    }
}
