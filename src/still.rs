//! A still image of the animated wallpaper, set as GNOME's own wallpaper.
//!
//! The animated wallpaper is painted by the Frontrise window and its GNOME
//! extension. Without them (Frontrise not running, extension off, login
//! screen) GNOME falls back to its static wallpaper. Keeping that one in step
//! means the fallback looks like what was playing.

use std::path::PathBuf;

use gio::prelude::SettingsExt;
use gtk::glib;

use crate::config::OriginalWallpaper;

const SCHEMA: &str = "org.gnome.desktop.background";

fn dir() -> PathBuf {
    glib::user_data_dir().join("frontrise").join("wallpaper")
}

/// A new file name each time: GNOME ignores a `picture-uri` that does not change.
pub fn next_path() -> PathBuf {
    dir().join(format!("still-{}.png", crate::config::new_id()))
}

fn settings() -> Option<gio::Settings> {
    let source = gio::SettingsSchemaSource::default()?;
    source.lookup(SCHEMA, true)?;
    Some(gio::Settings::new(SCHEMA))
}

fn uri_of(path: &std::path::Path) -> String {
    gio::prelude::FileExt::uri(&gio::File::for_path(path)).to_string()
}

/// True if `uri` points at one of our stills.
fn is_ours(uri: &str) -> bool {
    uri.starts_with(&uri_of(&dir()))
}

/// The wallpaper to give back later, unless GNOME already shows one of ours.
pub fn current() -> Option<OriginalWallpaper> {
    let s = settings()?;
    let uri = s.string("picture-uri").to_string();
    if is_ours(&uri) {
        return None;
    }
    Some(OriginalWallpaper { uri, uri_dark: s.string("picture-uri-dark").to_string() })
}

/// Makes `path` GNOME's wallpaper and removes our older stills.
pub fn apply(path: &std::path::Path) -> Result<(), String> {
    let s = settings().ok_or("GNOME background settings not available")?;
    let uri = uri_of(path);
    s.set_string("picture-uri", &uri).map_err(|e| e.to_string())?;
    s.set_string("picture-uri-dark", &uri).map_err(|e| e.to_string())?;
    gio::Settings::sync();
    if let Ok(entries) = std::fs::read_dir(dir()) {
        for entry in entries.flatten() {
            if entry.path() != path {
                let _ = std::fs::remove_file(entry.path());
            }
        }
    }
    Ok(())
}

/// Gives the user's previous wallpaper back and deletes our stills. Does
/// nothing to a wallpaper the user changed since: only ours is replaced.
pub fn restore(original: &OriginalWallpaper) {
    if let Some(s) = settings() {
        if is_ours(s.string("picture-uri").as_str()) {
            let _ = s.set_string("picture-uri", &original.uri);
        }
        if is_ours(s.string("picture-uri-dark").as_str()) {
            let _ = s.set_string("picture-uri-dark", &original.uri_dark);
        }
        gio::Settings::sync();
    }
    let _ = std::fs::remove_dir_all(dir());
}
