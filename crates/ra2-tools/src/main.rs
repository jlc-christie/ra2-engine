use std::collections::HashMap;
use std::path::Path;

use ra2_formats::mix::{mix_id, Mix};

fn usage() -> ! {
    eprintln!("usage: ra2-tools list <mix[:nested...]> [names.txt]\n       ra2-tools wav <mix[:nested...]> <sound or file.wav> <out.wav>");
    eprintln!("       ra2-tools extract <mix[:nested...]> <name> <out>");
    eprintln!("       ra2-tools shp <mix[:nested...]> <shp> <mix[:nested...]> <pal> <out.png>");
    eprintln!("       ra2-tools map <mix[:nested...]> <map> [overlay type to list]");
    eprintln!("       ra2-tools csf <mix[:nested...]> <csf> [filter]");
    std::process::exit(2);
}

/// Opens `path/to/a.mix:inner.mix:deeper.mix`.
fn open(spec: &str) -> Mix {
    let mut parts = spec.split(':');
    let mut mix = Mix::open(parts.next().unwrap()).expect("open mix");
    for inner in parts {
        mix = mix
            .nested(inner)
            .unwrap_or_else(|| panic!("{inner} not found"))
            .expect("parse nested mix");
    }
    mix
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        // Decodes a sound to a plain wave file: a name in the sound bank, or a
        // wave file in an archive.
        Some("wav") if args.len() == 4 => {
            let mix = open(&args[1]);
            let pcm = match mix.get(&args[2]) {
                Some(wave) => ra2_formats::audio::Pcm::from_wav(wave).expect("parse wave"),
                None => {
                    let index = ra2_formats::audio::AudioIndex::parse(mix.get("audio.idx").expect("audio.idx"))
                        .expect("parse audio.idx");
                    index.decode(&args[2], mix.get("audio.bag").expect("audio.bag")).expect("sound not found")
                }
            };
            println!("{} Hz, {} channel(s), {} samples", pcm.rate, pcm.channels, pcm.samples.len());
            std::fs::write(&args[3], pcm.to_wav()).expect("write output");
        }
        Some("list") if args.len() >= 2 => {
            let mix = open(&args[1]);
            let names: HashMap<u32, String> = args
                .get(2)
                .map(|path| std::fs::read_to_string(path).expect("read names"))
                .unwrap_or_default()
                .lines()
                .map(|name| (mix_id(name), name.to_owned()))
                .collect();
            let mut rows: Vec<(String, usize)> = mix
                .ids()
                .map(|id| {
                    let name = names.get(&id).cloned().unwrap_or(format!("?{id:08x}"));
                    (name, mix.get_id(id).unwrap().len())
                })
                .collect();
            rows.sort();
            for (name, size) in rows {
                println!("{size:>10} {name}");
            }
        }
        Some("extract") if args.len() == 4 => {
            let mix = open(&args[1]);
            let data = mix.get(&args[2]).unwrap_or_else(|| panic!("{} not found", args[2]));
            std::fs::write(Path::new(&args[3]), data).expect("write output");
        }
        Some("shp") if args.len() == 6 => {
            let shp_mix = open(&args[1]);
            let shp = ra2_formats::shp::Shp::parse(shp_mix.get(&args[2]).expect("shp not found"))
                .expect("parse shp");
            let pal_mix = open(&args[3]);
            let pal = ra2_formats::pal::Palette::parse(pal_mix.get(&args[4]).expect("pal not found"))
                .expect("parse pal");
            let (w, h) = (shp.width as u32, shp.height as u32);
            let drawn: Vec<usize> = (0..shp.frames.len())
                .filter(|&i| shp.frames[i].pixels.iter().any(|&p| p != 0))
                .collect();
            println!("{}x{} frames={} non-empty={:?}", w, h, shp.frames.len(), drawn);
            // All frames in one sheet, up to 8 per row, on a magenta backdrop.
            let cols = shp.frames.len().clamp(1, 8) as u32;
            let rows = (shp.frames.len() as u32).div_ceil(cols).max(1);
            let mut sheet =
                image::RgbaImage::from_pixel(cols * w, rows * h, image::Rgba([255, 0, 255, 255]));
            for index in 0..shp.frames.len() {
                let frame =
                    image::RgbaImage::from_raw(w, h, shp.frame_rgba(index, &pal)).unwrap();
                let (x, y) = (index as u32 % cols * w, index as u32 / cols * h);
                image::imageops::overlay(&mut sheet, &frame, x as i64, y as i64);
            }
            sheet.save(&args[5]).expect("write png");
        }
        Some("csf") if args.len() >= 3 => {
            let mix = open(&args[1]);
            let csf = ra2_formats::csf::Csf::parse(mix.get(&args[2]).expect("csf not found"))
                .expect("parse csf");
            let filter = args.get(3).map(|f| f.to_uppercase()).unwrap_or_default();
            let mut rows: Vec<_> = csf.iter().filter(|(k, _)| k.contains(&filter)).collect();
            rows.sort();
            for (label, text) in rows {
                println!("{label} = {text:?}");
            }
        }
        // pcx <mix> <name,name,...> <out.png>: all images in a row, enlarged 4x.
        Some("pcx") if args.len() == 4 => {
            let mix = open(&args[1]);
            let images: Vec<_> = args[2]
                .split(',')
                .map(|name| {
                    let pcx = ra2_formats::pcx::Pcx::parse(mix.get(name).expect("pcx not found"))
                        .expect("parse pcx");
                    println!("{name}: {}x{}", pcx.width, pcx.height);
                    image::RgbImage::from_raw(pcx.width, pcx.height, pcx.rgb).unwrap()
                })
                .collect();
            let width: u32 = images.iter().map(|i| i.width() + 2).sum();
            let height = images.iter().map(|i| i.height()).max().unwrap();
            let mut sheet = image::RgbImage::from_pixel(width, height, image::Rgb([0, 90, 0]));
            let mut x = 0;
            for img in &images {
                image::imageops::overlay(&mut sheet, img, x as i64, 0);
                x += img.width() + 2;
            }
            image::imageops::resize(&sheet, width * 4, height * 4, image::imageops::Nearest)
                .save(&args[3])
                .expect("write png");
        }
        // map <mix[:nested...]> <map>: summary of a map's contents.
        Some("map") if args.len() >= 3 => {
            let mix = open(&args[1]);
            let map = ra2_formats::map::Map::parse(mix.get(&args[2]).expect("map not found"))
                .expect("parse map");
            println!("{} {}x{} cells={} terrain objects={}", map.theater, map.width, map.height,
                map.tiles.len(), map.terrain_objects.len());
            // Per overlay type: cell count and one example cell.
            let mut overlays: std::collections::BTreeMap<u8, (usize, (i16, i16))> =
                Default::default();
            for tile in &map.tiles {
                if let Some((kind, _)) = map.overlay_at(tile.x as i32, tile.y as i32) {
                    let entry = overlays.entry(kind).or_insert((0, (tile.x, tile.y)));
                    entry.0 += 1;
                }
            }
            // `x0,y0,x1,y1` prints the raw overlay bytes for a block of cells.
            let block: Vec<i32> = args
                .get(3)
                .map(|a| a.split(',').filter_map(|n| n.parse().ok()).collect())
                .unwrap_or_default();
            if let [x0, y0, x1, y1] = block[..] {
                for y in y0..=y1 {
                    let row: Vec<String> = (x0..=x1)
                        .map(|x| match map.overlay_bytes(x, y) {
                            Some((kind, state)) => format!("{kind:3}/{state}"),
                            None => "  -  ".into(),
                        })
                        .collect();
                    println!("  y={y}: {}", row.join(" "));
                }
            }
            if let Some(want) = args.get(3).and_then(|n| n.parse::<u8>().ok()) {
                for tile in &map.tiles {
                    if let Some((kind, state)) = map.overlay_at(tile.x as i32, tile.y as i32)
                        && kind == want {
                            println!("  {},{} level {} state {state}", tile.x, tile.y, tile.level);
                        }
                }
            }
            for (kind, (count, (x, y))) in overlays {
                println!("overlay {kind}: {count} cells, for example at {x},{y}");
            }
        }
        _ => usage(),
    }
}
