//! Video thumbnails, cached as PNG in `$XDG_CACHE_HOME/frontrise/thumbs`.

use std::path::{Path, PathBuf};

use gst::prelude::*;

use crate::config::cache_dir;

pub fn path_for(id: &str) -> PathBuf {
    cache_dir().join("thumbs").join(format!("{id}.png"))
}

/// Decodes one frame (about a quarter in, at most 3 s) and writes a PNG.
/// Blocking: call from a worker thread.
pub fn generate(video: &Path, out: &Path) -> Result<(), String> {
    generate_sized(video, out, 384)
}

/// Same as [`generate`], scaled to `width` pixels wide.
pub fn generate_sized(video: &Path, out: &Path, width: i32) -> Result<(), String> {
    let pipeline = gst::parse::launch(&format!(
        "uridecodebin name=src ! videoconvert ! videoscale ! \
         video/x-raw,format=RGBA,width={width},pixel-aspect-ratio=1/1 ! appsink name=sink"
    ))
    .map_err(|e| e.to_string())?
    .downcast::<gst::Pipeline>()
    .map_err(|_| "not a pipeline")?;
    let src = pipeline.by_name("src").ok_or("no source")?;
    let uri = gio::prelude::FileExt::uri(&gio::File::for_path(video));
    src.set_property("uri", uri.as_str());
    let sink = pipeline
        .by_name("sink")
        .and_then(|e| e.downcast::<gst_app::AppSink>().ok())
        .ok_or("no sink")?;

    let result = (|| {
        pipeline.set_state(gst::State::Paused).map_err(|e| e.to_string())?;
        let (res, _, _) = pipeline.state(gst::ClockTime::from_seconds(10));
        res.map_err(|_| "could not open video".to_owned())?;
        if let Some(duration) = pipeline.query_duration::<gst::ClockTime>() {
            if duration > gst::ClockTime::from_seconds(2) {
                let at = (duration / 4).min(gst::ClockTime::from_seconds(3));
                if pipeline
                    .seek_simple(gst::SeekFlags::FLUSH | gst::SeekFlags::KEY_UNIT, at)
                    .is_ok()
                {
                    let _ = pipeline.state(gst::ClockTime::from_seconds(10));
                }
            }
        }
        let sample = sink
            .try_pull_preroll(gst::ClockTime::from_seconds(5))
            .ok_or("no frame decoded")?;
        let caps = sample.caps().ok_or("no caps")?;
        let s = caps.structure(0).ok_or("no caps structure")?;
        let width = s.get::<i32>("width").map_err(|e| e.to_string())? as usize;
        let height = s.get::<i32>("height").map_err(|e| e.to_string())? as usize;
        let buffer = sample.buffer().ok_or("no buffer")?;
        let map = buffer.map_readable().map_err(|e| e.to_string())?;
        let data = map.as_slice();
        let stride = data.len() / height.max(1);
        if stride < width * 4 {
            return Err("unexpected frame layout".into());
        }

        let mut pixels = Vec::with_capacity(width * height * 4);
        for row in 0..height {
            let start = row * stride;
            pixels.extend_from_slice(&data[start..start + width * 4]);
        }
        write_png(out, width as u32, height as u32, pixels)
    })();

    let _ = pipeline.set_state(gst::State::Null);
    result
}

/// Writes opaque RGBA pixels (`width * height * 4`) as a PNG, atomically.
fn write_png(out: &Path, width: u32, height: u32, mut pixels: Vec<u8>) -> Result<(), String> {
    if let Some(dir) = out.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = out.with_extension("png.tmp");
    let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().map_err(|e| e.to_string())?;
    // Thumbnails are opaque.
    for px in pixels.chunks_exact_mut(4) {
        px[3] = 255;
    }
    writer.write_image_data(&pixels).map_err(|e| e.to_string())?;
    writer.finish().map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())
}

/// Renders a frame of a scene (`seconds` into the animation) to a PNG.
/// Main thread only: it needs a GL context.
pub fn generate_scene(source: &str, seconds: f64, out: &Path) -> Result<(), String> {
    generate_scene_sized(source, seconds, out, 384, 216)
}

/// Same as [`generate_scene`], at `width` x `height` pixels.
pub fn generate_scene_sized(source: &str, seconds: f64, out: &Path, width: i32, height: i32) -> Result<(), String> {
    let pixels = crate::scene::render_still(source, seconds, width, height)?;
    write_png(out, width as u32, height as u32, pixels)
}

/// True if the cached scene preview `out` is newer than the scene file (when
/// it has one) and than this executable (built-in scenes ship inside it).
pub fn scene_is_fresh(out: &Path, scene_file: Option<&Path>) -> bool {
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    let Some(cached) = mtime(out) else { return false };
    let newest = [scene_file.and_then(mtime), std::env::current_exe().ok().and_then(|e| mtime(&e))]
        .into_iter()
        .flatten()
        .max();
    newest.is_none_or(|n| cached >= n)
}

/// True if the cached video thumbnail `out` is not older than the video.
pub fn video_is_fresh(out: &Path, video: Option<&Path>) -> bool {
    let mtime = |p: &Path| std::fs::metadata(p).and_then(|m| m.modified()).ok();
    match (mtime(out), video.and_then(mtime)) {
        (Some(cached), Some(source)) => cached >= source,
        (Some(_), None) => true, // video missing: keep what we have
        (None, _) => false,
    }
}
