//! Frontrise: animated video and shader wallpapers for GNOME.

mod app;
mod audio;
mod autostart;
mod config;
mod engine;
mod power;
mod scene;
mod shellext;
mod state;
mod thumbs;
mod ui;

pub const APP_ID: &str = "fr.ansicode.frontrise";
pub const EXTENSION_UUID: &str = "frontrise@ansicode.fr";
/// Title prefix of renderer windows; must match the GNOME Shell extension.
pub const WINDOW_TITLE_PREFIX: &str = "FrontriseWallpaper:";

fn main() -> glib::ExitCode {
    // By default wallpapers are decoded in software: NVDEC/CUDA costs ~400 MB of
    // RSS and ~1 core of CPU for a looping 1080p clip, versus ~265 MB and <10% in
    // libav. The "Hardware video decoding" setting opts back into the GPU decoders.
    // Must be set before gst::init(), so the saved setting is read here; a
    // user-provided GST_PLUGIN_FEATURE_RANK always wins.
    if std::env::var_os("GST_PLUGIN_FEATURE_RANK").is_none()
        && !config::Store::load().settings.hardware_decoding
    {
        std::env::set_var(
            "GST_PLUGIN_FEATURE_RANK",
            "nvh264dec:0,nvh265dec:0,nvvp9dec:0,nvav1dec:0,nvh264sldec:0,nvh265sldec:0,\
             nvautogpuh264dec:0,nvautogpuh265dec:0,nvautogpuvp9dec:0,nvautogpuav1dec:0",
        );
    }
    if let Err(e) = gst::init() {
        eprintln!("frontrise: GStreamer unavailable: {e}");
        return glib::ExitCode::FAILURE;
    }
    scene::load_gl();
    app::run()
}
