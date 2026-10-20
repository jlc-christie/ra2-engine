import bevy_core_pipeline::fullscreen_vertex_shader::FullscreenVertexOutput;
import bevy_render::color_operations::{linear_to_srgb, srgb_to_linear};

@group(0) @binding(0) var screen: texture_2d<f32>;
@group(0) @binding(1) var<uniform> pixel_scale: vec4<f32>;

// Recover the logical pixel centres from the nearest-neighbour enlarged image.
// Interpolate in sRGB, as cnc-ddraw's RGBA8 framebuffer shader does.
fn texel(p: vec2<f32>) -> vec3<f32> {
    let dimensions = vec2<i32>(textureDimensions(screen));
    let at = clamp(vec2<i32>(floor((p + 0.5) * pixel_scale.xy)), vec2<i32>(0), dimensions - 1);
    return linear_to_srgb(textureLoad(screen, at, 0).rgb);
}

fn bilinear(p: vec2<f32>) -> vec3<f32> {
    let base = floor(p);
    let f = fract(p);
    return mix(mix(texel(base), texel(base + vec2<f32>(1.0, 0.0)), f.x),
               mix(texel(base + vec2<f32>(0.0, 1.0)), texel(base + 1.0), f.x), f.y);
}

@fragment
fn fragment(in: FullscreenVertexOutput) -> @location(0) vec4<f32> {
    let p = in.position.xy / pixel_scale.xy - 0.5;
    let base = floor(p);
    let f = fract(p);
    let w0 = f * (-0.5 + f * (1.0 - 0.5 * f));
    let w1 = 1.0 + f * f * (-2.5 + 1.5 * f);
    let w2 = f * (0.5 + f * (2.0 - 1.5 * f));
    let w3 = f * f * (-0.5 + 0.5 * f);
    let w12 = w1 + w2;
    let middle = base + w2 / w12;
    // The same five-tap Catmull-Rom cross as catmull-rom-bilinear.glsl.
    let weights = vec4<f32>(w12.x * w0.y, w0.x * w12.y, w3.x * w12.y, w12.x * w3.y);
    let centre = w12.x * w12.y;
    let result = bilinear(vec2<f32>(middle.x, base.y - 1.0)) * weights.x
        + bilinear(vec2<f32>(base.x - 1.0, middle.y)) * weights.y
        + bilinear(middle) * centre
        + bilinear(vec2<f32>(base.x + 2.0, middle.y)) * weights.z
        + bilinear(vec2<f32>(middle.x, base.y + 2.0)) * weights.w;
    let total = dot(weights, vec4<f32>(1.0)) + centre;
    return vec4<f32>(srgb_to_linear(clamp(result / total, vec3<f32>(0.0), vec3<f32>(1.0))), 1.0);
}
