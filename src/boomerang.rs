//! Boomerang playback: a video that plays forward, then backward, forever.
//!
//! GTK's media stream cannot play backward, so the forward-then-reverse clip
//! is rendered once with `ffmpeg` and cached in
//! `$XDG_CACHE_HOME/frontrise/boomerang`; the cached clip then simply loops.
//! Until it is ready (or if it cannot be made) the original video loops.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use gtk::{gio, glib};

use crate::config::cache_dir;

/// ffmpeg's `reverse` filter holds every decoded frame in memory: refuse clips
/// that would need more than this much for them.
const MAX_FRAME_BYTES: f64 = 2.0 * 1024.0 * 1024.0 * 1024.0;

/// How long to leave a failed rendering alone before trying it again.
const RETRY_AFTER: Duration = Duration::from_secs(300);

/// H.264 encoders to try, best first; the last resort is always available.
const ENCODERS: &[&str] = &["libx264", "libopenh264", "libvpx-vp9", "mpeg4"];

thread_local! {
    static PENDING: RefCell<HashSet<String>> = RefCell::default();
    static FAILED: RefCell<HashMap<String, Instant>> = RefCell::default();
}

/// MP4, not Matroska: GTK loops a video by queueing the same file again when
/// it is about to finish, and with a Matroska file that hand-over froze the
/// picture for about 8 s on every turn. The very same stream in MP4 loops
/// smoothly.
pub fn path_for(id: &str) -> PathBuf {
    cache_dir().join("boomerang").join(format!("{id}.mp4"))
}

/// Where early 0.5.0 development builds cached clips, in Matroska.
fn legacy_path_for(id: &str) -> PathBuf {
    path_for(id).with_extension("mkv")
}

/// The rendered clip for `id`, if it exists and is not older than the video.
pub fn ready(id: &str, video: &Path) -> Option<PathBuf> {
    let out = path_for(id);
    crate::thumbs::video_is_fresh(&out, Some(video)).then_some(out).filter(|p| p.exists())
}

pub fn remove(id: &str) {
    let _ = std::fs::remove_file(path_for(id));
    let _ = std::fs::remove_file(legacy_path_for(id));
}

/// Starts rendering `id`'s boomerang clip in the background, unless it is
/// already running or failed recently. The engine is reconciled when done.
pub fn request(id: &str, video: &Path) {
    let cooling = FAILED.with(|f| f.borrow().get(id).is_some_and(|t| t.elapsed() < RETRY_AFTER));
    if cooling || !PENDING.with(|p| p.borrow_mut().insert(id.to_owned())) {
        return;
    }
    let id = id.to_owned();
    let video = video.to_owned();
    glib::spawn_future_local(async move {
        let out = path_for(&id);
        let _ = std::fs::remove_file(legacy_path_for(&id));
        let result = {
            let video = video.clone();
            gio::spawn_blocking(move || generate(&video, &out)).await
        };
        PENDING.with(|p| p.borrow_mut().remove(&id));
        match result {
            Ok(Ok(())) => {
                FAILED.with(|f| f.borrow_mut().remove(&id));
                crate::app::ctx().engine.queue_reconcile();
            }
            Ok(Err(e)) => {
                eprintln!("frontrise: no boomerang for {}: {e}", video.display());
                FAILED.with(|f| f.borrow_mut().insert(id, Instant::now()));
            }
            Err(_) => {
                eprintln!("frontrise: boomerang worker panicked for {}", video.display());
                FAILED.with(|f| f.borrow_mut().insert(id, Instant::now()));
            }
        }
    });
}

/// Renders `video` followed by itself reversed into `out`. Blocking.
fn generate(video: &Path, out: &Path) -> Result<(), String> {
    check_size(video)?;
    let encoder = pick_encoder()?;
    let dir = out.parent().ok_or("no cache directory")?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    let tmp = out.with_extension("part.mp4");

    let mut cmd = Command::new("ffmpeg");
    cmd.args(["-nostdin", "-hide_banner", "-loglevel", "error", "-y", "-i"])
        .arg(video)
        .args([
            "-filter_complex",
            // Even dimensions for the encoders, then forward + reversed.
            "[0:v:0]scale=trunc(iw/2)*2:trunc(ih/2)*2,format=yuv420p,split[a][b];\
             [b]reverse[r];[a][r]concat=n=2:v=1:a=0[v]",
            "-map",
            "[v]",
            "-an",
            "-c:v",
            encoder,
        ]);
    match encoder {
        "libx264" => {
            cmd.args(["-preset", "veryfast", "-crf", "18"]);
        }
        "libvpx-vp9" => {
            cmd.args(["-crf", "24", "-b:v", "0", "-deadline", "realtime", "-cpu-used", "8"]);
        }
        "mpeg4" => {
            cmd.args(["-q:v", "3"]);
        }
        _ => {
            cmd.args(["-b:v", "12M"]);
        }
    }
    let output = cmd
        .args(["-movflags", "+faststart"])
        .arg(&tmp)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .output()
        .map_err(|e| format!("could not run ffmpeg (is it installed?): {e}"))?;
    if !output.status.success() {
        let _ = std::fs::remove_file(&tmp);
        let msg = String::from_utf8_lossy(&output.stderr);
        return Err(format!("ffmpeg failed: {}", msg.lines().last().unwrap_or("unknown error")));
    }
    std::fs::rename(&tmp, out).map_err(|e| e.to_string())
}

fn pick_encoder() -> Result<&'static str, String> {
    let output = Command::new("ffmpeg")
        .args(["-hide_banner", "-encoders"])
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run ffmpeg (is it installed?): {e}"))?;
    let list = String::from_utf8_lossy(&output.stdout);
    let has = |name: &str| list.lines().any(|l| l.split_whitespace().nth(1) == Some(name));
    ENCODERS.iter().copied().find(|e| has(e)).ok_or_else(|| "ffmpeg has no usable video encoder".into())
}

/// Refuses clips whose decoded frames would not fit in memory.
fn check_size(video: &Path) -> Result<(), String> {
    let output = Command::new("ffprobe")
        .args([
            "-v",
            "error",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=width,height,r_frame_rate,duration:format=duration",
            "-of",
            "default=noprint_wrappers=1",
        ])
        .arg(video)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run ffprobe (is ffmpeg installed?): {e}"))?;
    let text = String::from_utf8_lossy(&output.stdout);
    let (mut width, mut height, mut fps, mut seconds) = (0.0, 0.0, 0.0, 0.0f64);
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else { continue };
        match key {
            "width" => width = value.parse().unwrap_or(0.0),
            "height" => height = value.parse().unwrap_or(0.0),
            "r_frame_rate" => {
                fps = match value.split_once('/') {
                    Some((n, d)) => n.parse::<f64>().unwrap_or(0.0) / d.parse::<f64>().unwrap_or(1.0).max(1.0),
                    None => value.parse().unwrap_or(0.0),
                }
            }
            // The stream's duration wins over the container's when both exist.
            "duration" => seconds = seconds.max(value.parse().unwrap_or(0.0)),
            _ => {}
        }
    }
    let bytes = width * height * 1.5 * fps * seconds;
    if bytes > MAX_FRAME_BYTES {
        return Err(format!("clip too long for a boomerang ({seconds:.0} s at {width}x{height})"));
    }
    Ok(())
}
