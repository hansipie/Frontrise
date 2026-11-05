//! Frontrise: animated video and shader wallpapers for GNOME.

mod app;
mod audio;
mod autostart;
mod config;
mod decoding;
mod engine;
mod power;
mod scene;
mod shellext;
mod state;
mod still;
mod thumbs;
mod ui;

pub const APP_ID: &str = "fr.ansicode.frontrise";
pub const EXTENSION_UUID: &str = "frontrise@ansicode.fr";
/// Title prefix of renderer windows; must match the GNOME Shell extension.
pub const WINDOW_TITLE_PREFIX: &str = "FrontriseWallpaper:";

fn main() -> glib::ExitCode {
    // By default wallpapers are decoded in software: NVDEC/CUDA costs ~400 MB of
    // RSS and ~1 core of CPU for a looping 1080p clip, versus ~265 MB and <10% in
    // libav. The "Hardware video decoding" setting opts back into the GPU decoders,
    // NVDEC only when NVIDIA drives the display (see `decoding`).
    // Must be set before gst::init(), so the saved setting is read here; a
    // user-provided GST_PLUGIN_FEATURE_RANK always wins.
    if std::env::var_os("GST_PLUGIN_FEATURE_RANK").is_none() {
        let hardware = config::Store::load().settings.hardware_decoding;
        if !decoding::allow_nvidia_decoders(hardware, &decoding::gpus()) {
            std::env::set_var("GST_PLUGIN_FEATURE_RANK", decoding::NVIDIA_DECODERS_OFF);
        }
    }
    if let Err(e) = gst::init() {
        eprintln!("frontrise: GStreamer unavailable: {e}");
        return glib::ExitCode::FAILURE;
    }
    scene::load_gl();
    app::run()
}
