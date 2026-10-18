// The dolphin's sonic wave: the picture already drawn inside the wave's band
// is bent and tinted. Each pixel takes the colour of one up to three pixels
// further along the wave's direction, by the sine of its distance from the
// firer, and its green and blue are raised.

struct Wave {
    // The band's four corners, in screen pixels, in order round it.
    ab: vec4<f32>,
    cd: vec4<f32>,
    // The firer's point and the one-pixel step along the wave.
    centre_step: vec4<f32>,
    // x: the phase (frames of life left), y: 1 when the wave is there.
    misc: vec4<f32>,
}

struct Sonic {
    waves: array<Wave, 4>,
    // The map area in screen pixels (min x, min y, max x, max y); z of scale:
    // screen pixels per game pixel.
    clip: vec4<f32>,
    scale: vec4<f32>,
}

@group(0) @binding(0) var screen_texture: texture_2d<f32>;
@group(0) @binding(1) var screen_sampler: sampler;
@group(0) @binding(2) var<uniform> sonic: Sonic;

fn side(a: vec2<f32>, b: vec2<f32>, p: vec2<f32>) -> f32 {
    let e = b - a;
    let q = p - a;
    return e.x * q.y - e.y * q.x;
}

fn inside(w: Wave, p: vec2<f32>) -> bool {
    let a = w.ab.xy;
    let b = w.ab.zw;
    let c = w.cd.xy;
    let d = w.cd.zw;
    let s0 = side(a, b, p);
    let s1 = side(b, c, p);
    let s2 = side(c, d, p);
    let s3 = side(d, a, p);
    return (s0 >= 0.0 && s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0) || (s0 <= 0.0 && s1 <= 0.0 && s2 <= 0.0 && s3 <= 0.0);
}

fn to_srgb(c: vec3<f32>) -> vec3<f32> {
    return select(1.055 * pow(c, vec3(1.0 / 2.4)) - 0.055, c * 12.92, c <= vec3(0.0031308));
}

fn to_linear(c: vec3<f32>) -> vec3<f32> {
    return select(pow((c + 0.055) / 1.055, vec3(2.4)), c / 12.92, c <= vec3(0.04045));
}

@fragment
fn fragment(@builtin(position) position: vec4<f32>) -> @location(0) vec4<f32> {
    let size = vec2<i32>(textureDimensions(screen_texture));
    let here = vec2<i32>(position.xy);
    let original = textureLoad(screen_texture, here, 0);
    let p = position.xy;
    if p.x < sonic.clip.x || p.y < sonic.clip.y || p.x >= sonic.clip.z || p.y >= sonic.clip.w {
        return original;
    }
    let scale = sonic.scale.x;
    for (var i = 0; i < 4; i++) {
        let w = sonic.waves[i];
        if w.misc.y < 0.5 || !inside(w, p) {
            continue;
        }
        // Distances and offsets are in the game's own pixels.
        let q = p / scale;
        let d = abs(q - w.centre_step.xy);
        let distance = floor(sqrt(d.x * d.x + d.y * d.y));
        let ripple = i32(sin((distance + w.misc.x) * 0.125) * 12.0 + 0.49);
        let level = abs(ripple);
        var shift = 0;
        if level >= 10 {
            shift = 3;
        } else if level >= 7 {
            shift = 2;
        } else if level >= 2 {
            shift = 1;
        }
        let source = clamp(here + vec2<i32>(round(w.centre_step.zw * f32(shift) * scale)), vec2(0), size - 1);
        let picked = to_srgb(textureLoad(screen_texture, source, 0).rgb);
        let raise = 1.0 + f32(110 + 8 * level) / 256.0;
        let shaded = vec3(picked.r, min(picked.g * raise, 1.0), min(picked.b * raise, 1.0));
        return vec4(to_linear(shaded), original.a);
    }
    return original;
}
