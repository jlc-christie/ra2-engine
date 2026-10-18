import bevy_sprite_render::{
    mesh2d::vertex_output::VertexOutput,
    mesh2d::view_bindings::view,
    mesh2d::functions as mesh_functions,
    sprite_mesh::bindings::{material, texture},
    sprite_mesh::functions as sprite_functions,
};

@if(TONEMAP_IN_SHADER)
import bevy_core_pipeline::tonemapping;
@if(SRGB_OUTPUT)
import bevy_render::color_operations::linear_to_srgb;
@if(OKLAB_OUTPUT)
import bevy_render::color_operations::linear_rgb_to_oklab;

// Mode, depth shape offset x and y, unused.
@group(constants::MATERIAL_BIND_GROUP) @binding(20) var<uniform> params: vec4<i32>;
@group(constants::MATERIAL_BIND_GROUP) @binding(21) var shape: texture_2d<i32>;

struct Output {
    @location(0) color: vec4<f32>,
    @builtin(frag_depth) depth: f32,
}

fn floor_div(a: i32, b: i32) -> i32 {
    var q = a / b;
    if (a % b != 0) && ((a < 0) != (b < 0)) {
        q = q - 1;
    }
    return q;
}

fn texel(uv: vec2<f32>, size: vec2<u32>) -> vec2<i32> {
    return vec2<i32>(floor(uv * vec2<f32>(size)));
}

@fragment
fn fragment(mesh: VertexOutput) -> Output {
    var color = sprite_functions::sample_final_color(mesh.uv, mesh.instance_index);
    if color.a <= 0.0 {
        discard;
    }
    let row = i32(floor(-mesh.world_position.y));
    let adjust = bitcast<i32>(mesh_functions::get_tag(mesh.instance_index));
    let mode = params.x;
    // The bottom row of the picture's own rectangle.
    let world_from_local = mesh_functions::get_world_from_local(mesh.instance_index);
    let bottom = world_from_local * vec4<f32>(0.0, -0.5 * material.vertex_scale.y + material.vertex_offset.y, 0.0, 1.0);
    let yb = i32(round(-bottom.y)) - 1;
    var d: i32;
    if mode == 1 {
        d = 3 * floor_div(-yb + adjust, 3) - floor_div(yb - row, 3);
    } else if mode == 2 {
        d = -row + adjust;
    } else {
        let uv = sprite_functions::get_final_uv(mesh.uv, mesh.instance_index);
        let at = texel(uv, textureDimensions(texture)) + params.yz;
        let size = vec2<i32>(textureDimensions(shape));
        if at.x < 0 || at.y < 0 || at.x >= size.x || at.y >= size.y {
            discard;
        }
        d = -yb + adjust - textureLoad(shape, at, 0).r;
    }

@if(TONEMAP_IN_SHADER)
    color = tonemapping::tone_mapping(color, view.color_grading);
@if(SRGB_OUTPUT)
    color = vec4(linear_to_srgb(color.rgb), color.a);
@if(OKLAB_OUTPUT)
    color = vec4(linear_rgb_to_oklab(color.rgb), color.a);

    // Every depth lies behind world z = -1, so plain sprites draw over all of
    // them as before; nearer is greater.
    let limit = view.clip_from_world * vec4<f32>(0.0, 0.0, -1.0, 1.0);
    var out: Output;
    out.color = color;
    out.depth = limit.z / limit.w * clamp(f32(8192 - d) / 262144.0, 0.0, 1.0);
    return out;
}
