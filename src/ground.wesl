import bevy_sprite_render::{
    mesh2d::vertex_output::VertexOutput,
    mesh2d::view_bindings::view,
};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;
@if(SRGB_OUTPUT)
import bevy_render::color_operations::linear_to_srgb;
@if(OKLAB_OUTPUT)
import bevy_render::color_operations::linear_rgb_to_oklab;

@group(constants::MATERIAL_BIND_GROUP) @binding(0) var colours: texture_2d<f32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(1) var heights: texture_2d<u32>;

struct Output {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

@fragment
fn fragment(mesh: VertexOutput) -> Output {
    let at = vec2<i32>(floor(mesh.uv * vec2<f32>(textureDimensions(colours))));
    var color = textureLoad(colours, at, 0);
    if color.a <= 0.0 {
        discard;
    }
    // A tile pixel's depth is its tile's base plus its own depth byte.
    let d = i32(round(mesh.color.r)) + i32(textureLoad(heights, at, 0).r);

@if(TONEMAP_IN_SHADER)
    color = tonemapping::tone_mapping(color, view.color_grading);
@if(SRGB_OUTPUT)
    color = vec4(linear_to_srgb(color.rgb), color.a);
@if(OKLAB_OUTPUT)
    color = vec4(linear_rgb_to_oklab(color.rgb), color.a);

    // As in depth.wesl.
    let limit = view.clip_from_world * vec4<f32>(0.0, 0.0, -1.0, 1.0);
    var out: Output;
    out.color = color;
    out.depth = limit.z / limit.w * clamp(f32(8192 - d) / 262144.0, 0.0, 1.0);
    return out;
}
