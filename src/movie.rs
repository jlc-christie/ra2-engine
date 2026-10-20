//! The original menu's Bink movie. FFmpeg decodes into a bounded frame queue;
//! no decoded movie or proprietary asset is included in the executable.

use std::io::Read;
use std::process::{Child, Command, Stdio};
use std::sync::{Mutex, mpsc};

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

#[derive(Resource)]
pub(crate) struct MenuMovie {
    pub image: Handle<Image>,
    frames: Mutex<mpsc::Receiver<Vec<u8>>>,
    decoder: Mutex<Child>,
    frame_seconds: f64,
    next_frame: f64,
}

impl Drop for MenuMovie {
    fn drop(&mut self) {
        if let Ok(decoder) = self.decoder.get_mut() {
            let _ = decoder.kill();
            let _ = decoder.wait();
        }
    }
}

pub(crate) fn image(
    movie: Option<&MenuMovie>,
    files: &crate::files::GameFiles,
    images: &mut Assets<Image>,
    commands: &mut Commands,
) -> Option<Handle<Image>> {
    if let Some(movie) = movie { return Some(movie.image.clone()) }
    let mut load = || -> Result<MenuMovie, String> {
        let bytes = files.get("ra2ts_l.bik").ok_or("ra2ts_l.bik is missing")?;
        let (width, height, fps) = dimensions(bytes).ok_or("invalid menu movie header")?;
        let digest = bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3));
        let dir = crate::home::data_dir().join("cache");
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        let path = dir.join(format!("ra2ts_l-{digest:016x}.bik"));
        if !path.exists() { std::fs::write(&path, bytes).map_err(|e| e.to_string())? }
        let mut decoder = Command::new("ffmpeg")
            .args(["-v", "error", "-threads", "1", "-stream_loop", "-1", "-i"])
            .arg(path)
            .args(["-an", "-f", "rawvideo", "-pix_fmt", "rgba", "-threads", "1", "pipe:1"])
            .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null())
            .spawn().map_err(|e| format!("FFmpeg is needed for the menu animation: {e}"))?;
        let mut output = decoder.stdout.take().ok_or("decoder has no output")?;
        let (send, receive) = mpsc::sync_channel(2);
        let size = width as usize * height as usize * 4;
        std::thread::spawn(move || loop {
            let mut frame = vec![0; size];
            if output.read_exact(&mut frame).is_err() || send.send(frame).is_err() { break }
        });
        let image = images.add(Image::new_fill(
            Extent3d { width, height, depth_or_array_layers: 1 },
            TextureDimension::D2, &[0, 0, 0, 0], TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD,
        ));
        Ok(MenuMovie { image, frames: Mutex::new(receive), decoder: Mutex::new(decoder), frame_seconds: 1.0 / fps, next_frame: 0.0 })
    };
    match load() {
        Ok(movie) => {
            let image = movie.image.clone();
            commands.insert_resource(movie);
            Some(image)
        }
        Err(reason) => { warn!("{reason}"); None }
    }
}

fn dimensions(bytes: &[u8]) -> Option<(u32, u32, f64)> {
    if bytes.get(..3)? != b"BIK" { return None }
    let word = |at| Some(u32::from_le_bytes(bytes.get(at..at + 4)?.try_into().ok()?));
    let (width, height, numerator, denominator) = (word(20)?, word(24)?, word(28)?, word(32)?);
    (width > 0 && width <= 4096 && height > 0 && height <= 4096 && numerator > 0 && denominator > 0)
        .then_some((width, height, f64::from(numerator) / f64::from(denominator)))
}

pub struct MoviePlugin;

impl Plugin for MoviePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, advance.run_if(|screen: Res<State<crate::shell::Screen>>| {
            matches!(screen.get(), crate::shell::Screen::MainMenu | crate::shell::Screen::SinglePlayer)
        }));
    }
}

fn advance(movie: Option<ResMut<MenuMovie>>, time: Res<Time<Real>>, mut images: ResMut<Assets<Image>>) {
    let Some(mut movie) = movie else { return };
    let now = time.elapsed_secs_f64();
    if now < movie.next_frame { return }
    let frame = movie.frames.lock().ok().and_then(|frames| frames.try_recv().ok());
    if let Some(frame) = frame {
        if let Some(mut image) = images.get_mut(&movie.image) { image.data = Some(frame) }
        movie.next_frame = now + movie.frame_seconds;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bink_header_checks_dimensions_and_frame_rate() {
        let mut header = [0u8; 36];
        header[..4].copy_from_slice(b"BIKi");
        for (offset, value) in [(20, 632u32), (24, 570), (28, 30), (32, 2)] {
            header[offset..offset + 4].copy_from_slice(&value.to_le_bytes());
        }
        assert_eq!(dimensions(&header), Some((632, 570, 15.0)));
        assert_eq!(dimensions(&header[..35]), None);
        header[32..36].fill(0);
        assert_eq!(dimensions(&header), None);
    }
}
