//! Filter enlarged game pixels after the world and UI have both been drawn.
//! The user's cnc-ddraw installation uses Catmull-Rom, not nearest neighbour.

use bevy::asset::{embedded_asset, load_embedded_asset};
use bevy::core_pipeline::{FullscreenShader, schedule::Core2d, upscaling::upscaling};
use bevy::prelude::*;
use bevy::render::{
    RenderApp, RenderStartup,
    extract_component::{ComponentUniforms, DynamicUniformIndex, ExtractComponent, ExtractComponentPlugin, UniformComponentPlugin},
    render_resource::{binding_types::{texture_2d, uniform_buffer}, *},
    renderer::{RenderContext, RenderDevice, ViewQuery},
    view::ViewTarget,
};
use bevy::ui_render::ui_pass;
use bevy::window::PrimaryWindow;

#[derive(Component, Clone, Copy, ExtractComponent, ShaderType)]
#[extract_app(RenderApp)]
struct PixelScale { value: Vec4 }

pub struct ResamplePlugin;

impl Plugin for ResamplePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "resample.wesl");
        app.add_plugins((ExtractComponentPlugin::<PixelScale>::default(), UniformComponentPlugin::<PixelScale>::default()))
            .add_systems(PostUpdate, settings);
        if let Some(render) = app.get_sub_app_mut(RenderApp) {
            render.add_systems(RenderStartup, pipeline)
                .add_systems(Core2d, filter.after(ui_pass).before(upscaling));
        }
    }
}

fn settings(
    mut commands: Commands,
    window: Single<&Window, With<PrimaryWindow>>,
    scale: Res<UiScale>,
    screen: Res<State<crate::shell::Screen>>,
    cameras: Query<Entity, With<Camera2d>>,
) {
    let ui = if *screen.get() == crate::shell::Screen::InGame { 1.0 } else { scale.0 };
    let factor = if std::env::var_os("RA2_NEAREST").is_some() { 1.0 } else { window.scale_factor() * ui };
    for camera in &cameras {
        commands.entity(camera).insert(PixelScale { value: Vec4::splat(factor.max(1.0)) });
    }
}

#[derive(Resource)]
struct Pipeline { layout: BindGroupLayoutDescriptor, id: CachedRenderPipelineId }

fn pipeline(mut commands: Commands, server: Res<AssetServer>, fullscreen: Res<FullscreenShader>, cache: Res<PipelineCache>) {
    let layout = BindGroupLayoutDescriptor::new("game_pixel_filter", &BindGroupLayoutEntries::sequential(
        ShaderStages::FRAGMENT,
        (texture_2d(TextureSampleType::Float { filterable: false }), uniform_buffer::<PixelScale>(true)),
    ));
    let id = cache.queue_render_pipeline(RenderPipelineDescriptor {
        label: Some("game_pixel_filter".into()),
        layout: vec![layout.clone()],
        vertex: fullscreen.to_vertex_state(),
        fragment: Some(FragmentState {
            shader: load_embedded_asset!(server.as_ref(), "resample.wesl"),
            targets: vec![Some(ColorTargetState { format: TextureFormat::Rgba8UnormSrgb, blend: None, write_mask: ColorWrites::ALL })],
            ..default()
        }),
        ..default()
    });
    commands.insert_resource(Pipeline { layout, id });
}

fn filter(
    view: ViewQuery<(&ViewTarget, &PixelScale, &DynamicUniformIndex<PixelScale>)>,
    pipeline: Option<Res<Pipeline>>,
    cache: Res<PipelineCache>,
    uniforms: Res<ComponentUniforms<PixelScale>>,
    device: Res<RenderDevice>,
    mut context: RenderContext,
) {
    let (target, scale, index) = view.into_inner();
    if scale.value.x <= 1.001 { return }
    let Some(pipeline) = pipeline else { return };
    let Some(compiled) = cache.get_render_pipeline(pipeline.id) else { return };
    let Some(binding) = uniforms.uniforms().binding() else { return };
    let post = target.post_process_write();
    let group = device.create_bind_group("game_pixel_filter", &cache.get_bind_group_layout(&pipeline.layout),
        &BindGroupEntries::sequential((post.source, binding)));
    let mut pass = context.command_encoder().begin_render_pass(&RenderPassDescriptor {
        label: Some("game_pixel_filter"),
        color_attachments: &[Some(RenderPassColorAttachment {
            view: post.destination, depth_slice: None, resolve_target: None, ops: Operations::default(),
        })],
        depth_stencil_attachment: None, timestamp_writes: None, occlusion_query_set: None, multiview_mask: None,
    });
    pass.set_pipeline(compiled);
    pass.set_bind_group(0, &group, &[index.index()]);
    pass.draw(0..3, 0..1);
}
