//! The picture behind the disk image's window: the game's name, an arrow from
//! where the app sits to where the Applications folder sits, and a line saying
//! to drag one onto the other. Drawn here (the lettering in the Mac's own
//! font, read at build time), so nothing of the game's is used.

use ab_glyph::{Font, FontVec, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

/// The window's inside, in points.
pub const WIDTH: u32 = 640;
pub const HEIGHT: u32 = 420;
/// Where the icons' middles go, in points from the window's top left.
pub const APP_AT: (u32, u32) = (165, 165);
pub const APPLICATIONS_AT: (u32, u32) = (475, 165);
pub const READ_ME_AT: (u32, u32) = (320, 345);
/// The icons' size, in points.
pub const ICON_SIZE: u32 = 96;

/// The picture at `scale` times the window's size (1, and 2 for Retina).
pub fn draw(scale: u32, name: &str) -> RgbaImage {
    let (w, h) = (WIDTH * scale, HEIGHT * scale);
    let s = scale as f32;
    let mut image = RgbaImage::from_fn(w, h, |x, y| {
        // Dark, warming toward red at the bottom, with a soft light behind the
        // icons' row.
        let v = y as f32 / h as f32;
        let (dx, dy) = (x as f32 / w as f32 - 0.5, v - APP_AT.1 as f32 / HEIGHT as f32);
        let glow = (1.0 - (dx * dx * 1.2 + dy * dy * 6.0).sqrt() * 1.6).max(0.0) * 0.10;
        let r = 0.075 + 0.17 * v * v + glow;
        let g = 0.07 + 0.02 * v + glow * 0.8;
        let b = 0.08 + 0.01 * v + glow * 0.8;
        Rgba([to_byte(r), to_byte(g), to_byte(b), 255])
    });
    // A red rule under the title.
    fill(&mut image, (40.0 * s) as u32, (66.0 * s) as u32, (560.0 * s) as u32, (2.0 * s).max(1.0) as u32, [200, 24, 18]);
    // The arrow, from beside the app to beside the folder.
    arrow(&mut image, s, (APP_AT.0 + 75) as f32, (APPLICATIONS_AT.0 - 75) as f32, APP_AT.1 as f32);
    if let Some(font) = font() {
        text(&mut image, &font, s, name.to_uppercase().as_str(), 28.0, 40.0, 50.0, [255, 255, 255], false);
        text(&mut image, &font, s, "Drag the game onto the Applications folder to install it", 15.0, 320.0, 262.0, [235, 225, 220], true);
        text(&mut image, &font, s, "You'll need your own copy of Red Alert 2: see Read Me First, below", 12.0, 320.0, 284.0, [160, 150, 150], true);
    }
    image
}

fn to_byte(value: f32) -> u8 {
    (value.clamp(0.0, 1.0) * 255.0).round() as u8
}

fn fill(image: &mut RgbaImage, x: u32, y: u32, w: u32, h: u32, rgb: [u8; 3]) {
    for py in y..(y + h).min(image.height()) {
        for px in x..(x + w).min(image.width()) {
            image.put_pixel(px, py, Rgba([rgb[0], rgb[1], rgb[2], 255]));
        }
    }
}

/// A gold arrow pointing right, from `x0` to `x1` at height `y` (points).
fn arrow(image: &mut RgbaImage, s: f32, x0: f32, x1: f32, y: f32) {
    let (shaft, head_len, head_half) = (7.0, 30.0, 22.0);
    let gold = [255.0, 196.0, 58.0];
    // Each pixel sampled 4x4 for smooth edges.
    let (left, right) = ((x0 * s) as u32, (x1 * s) as u32 + 1);
    let (top, bottom) = (((y - head_half - 2.0) * s) as u32, ((y + head_half + 2.0) * s) as u32);
    for py in top..bottom.min(image.height()) {
        for px in left..right.min(image.width()) {
            let mut covered = 0.0;
            for sy in 0..4 {
                for sx in 0..4 {
                    let (x, yy) = ((px as f32 + (sx as f32 + 0.5) / 4.0) / s, (py as f32 + (sy as f32 + 0.5) / 4.0) / s);
                    let d = (yy - y).abs();
                    let head_start = x1 - head_len;
                    let inside = if x < x0 {
                        false
                    } else if x < head_start {
                        d <= shaft / 2.0
                    } else {
                        x <= x1 && d <= head_half * (x1 - x) / head_len
                    };
                    if inside {
                        covered += 1.0 / 16.0;
                    }
                }
            }
            if covered > 0.0 {
                let under = image.get_pixel(px, py).0;
                let mix = |i: usize| (under[i] as f32 * (1.0 - covered) + gold[i] * covered).round() as u8;
                image.put_pixel(px, py, Rgba([mix(0), mix(1), mix(2), 255]));
            }
        }
    }
}

/// The Mac's system font, if it can be read.
fn font() -> Option<FontVec> {
    ["/System/Library/Fonts/SFNS.ttf", "/System/Library/Fonts/Supplemental/Arial.ttf"]
        .iter()
        .find_map(|path| FontVec::try_from_vec(std::fs::read(path).ok()?).ok())
}

/// Writes a line of text, its left edge (or its middle, when `centred`) at
/// `x` and its baseline at `y`, in points.
fn text(image: &mut RgbaImage, font: &FontVec, s: f32, words: &str, size: f32, x: f32, y: f32, rgb: [u8; 3], centred: bool) {
    let scaled = font.as_scaled(PxScale::from(size * s));
    let width: f32 = words.chars().map(|c| scaled.h_advance(scaled.glyph_id(c))).sum();
    let mut pen = if centred { x * s - width / 2.0 } else { x * s };
    for c in words.chars() {
        let id = scaled.glyph_id(c);
        let glyph = id.with_scale_and_position(PxScale::from(size * s), ab_glyph::point(pen, y * s));
        pen += scaled.h_advance(id);
        let Some(outline) = font.outline_glyph(glyph) else {
            continue;
        };
        let bounds = outline.px_bounds();
        outline.draw(|gx, gy, coverage| {
            let (px, py) = (bounds.min.x as i32 + gx as i32, bounds.min.y as i32 + gy as i32);
            if px < 0 || py < 0 || px as u32 >= image.width() || py as u32 >= image.height() {
                return;
            }
            let under = image.get_pixel(px as u32, py as u32).0;
            let mix = |i: usize| (under[i] as f32 * (1.0 - coverage) + rgb[i] as f32 * coverage).round() as u8;
            image.put_pixel(px as u32, py as u32, Rgba([mix(0), mix(1), mix(2), 255]));
        });
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn draws_at_both_sizes() {
        let one = super::draw(1, "RA2 Engine");
        let two = super::draw(2, "RA2 Engine");
        assert_eq!((one.width(), one.height()), (super::WIDTH, super::HEIGHT));
        assert_eq!((two.width(), two.height()), (super::WIDTH * 2, super::HEIGHT * 2));
        if let Ok(path) = std::env::var("BACKGROUND_OUT") {
            two.save(path).unwrap();
        }
    }
}
