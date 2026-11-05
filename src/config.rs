//! Persistent data: library, playlists, per-display assignments and settings.
//!
//! Everything lives in one JSON file, `$XDG_CONFIG_HOME/frontrise/config.json`.
//! The GNOME Shell extension reads the `settings` object from the same file.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::scene;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Kind {
    #[default]
    Video,
    Scene,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Item {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub kind: Kind,
    /// File on disk (videos and imported scenes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<PathBuf>,
    /// Key of a scene shipped inside Frontrise.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub builtin: Option<String>,
    #[serde(default)]
    pub favorite: bool,
}

impl Item {
    pub fn is_builtin(&self) -> bool {
        self.builtin.is_some()
    }

    pub fn is_available(&self) -> bool {
        self.builtin.is_some() || self.path.as_deref().is_some_and(Path::exists)
    }

    /// GLSL source of a scene item.
    pub fn scene_source(&self) -> Result<String, String> {
        if let Some(key) = &self.builtin {
            return scene::builtin_source(key)
                .map(str::to_owned)
                .ok_or_else(|| format!("unknown built-in scene {key}"));
        }
        let path = self.path.as_ref().ok_or("scene has no file")?;
        std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))
    }

    pub fn kind_label(&self) -> &'static str {
        match self.kind {
            Kind::Video => "Video",
            Kind::Scene => "Scene",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Playlist {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub items: Vec<String>,
    #[serde(default = "default_interval")]
    pub interval_minutes: u32,
    #[serde(default)]
    pub shuffle: bool,
}

fn default_interval() -> u32 {
    10
}

/// What a display shows.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", content = "id", rename_all = "snake_case")]
pub enum Source {
    /// Frontrise is off on this display: GNOME shows its own wallpaper.
    #[default]
    Off,
    Item(String),
    Playlist(String),
    /// Mirror another display (by connector name), in sync.
    SameAs(String),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Fit {
    /// Fill the screen, cropping the edges if needed.
    #[default]
    Cover,
    /// Show the whole video, with black bars if needed.
    Contain,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DisplayConfig {
    #[serde(default)]
    pub source: Source,
    #[serde(default)]
    pub fit: Fit,
    /// Last known human-readable name, for disconnected displays.
    #[serde(default)]
    pub label: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub pause_on_battery: bool,
    pub pause_on_power_saver: bool,
    pub pause_when_fullscreen: bool,
    pub react_to_sound: bool,
    pub autostart: bool,
    pub lock_screen: bool,
    pub panel_icon: bool,
    /// Use the GPU video decoder (NVDEC). Read at startup, before GStreamer init.
    pub hardware_decoding: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            pause_on_battery: true,
            pause_on_power_saver: true,
            pause_when_fullscreen: true,
            react_to_sound: true,
            autostart: false,
            lock_screen: true,
            panel_icon: true,
            hardware_decoding: false,
        }
    }
}

/// The user's answer to "may Frontrise repair its GNOME extension by
/// itself?". It only holds for the installation it was given for: updating
/// or reinstalling Frontrise asks again.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RepairConsent {
    /// `install_stamp()` at the time of the answer.
    pub install: String,
    pub allow: bool,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Store {
    pub library: Vec<Item>,
    pub playlists: Vec<Playlist>,
    pub displays: BTreeMap<String, DisplayConfig>,
    pub settings: Settings,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extension_repair: Option<RepairConsent>,
}

/// Identifies this installation of Frontrise: its version and when its
/// binary was written (install.sh rewrites it on every install).
pub fn install_stamp() -> &'static str {
    static STAMP: OnceLock<String> = OnceLock::new();
    STAMP.get_or_init(|| {
        let modified = std::env::current_exe()
            .and_then(std::fs::metadata)
            .and_then(|m| m.modified())
            .ok();
        stamp(env!("CARGO_PKG_VERSION"), modified)
    })
}

fn stamp(version: &str, modified: Option<SystemTime>) -> String {
    match modified.and_then(|t| t.duration_since(UNIX_EPOCH).ok()) {
        Some(d) if d.as_secs() > 0 => format!("{version}@{}", d.as_secs()),
        // Read-only images can zero the mtime: fall back to the version.
        _ => version.to_owned(),
    }
}

pub fn config_dir() -> PathBuf {
    glib::user_config_dir().join("frontrise")
}

pub fn cache_dir() -> PathBuf {
    glib::user_cache_dir().join("frontrise")
}

fn config_file() -> PathBuf {
    config_dir().join("config.json")
}

pub fn new_id() -> String {
    (0..12).map(|_| fastrand::alphanumeric().to_ascii_lowercase()).collect()
}

impl Store {
    pub fn load() -> Self {
        let mut store: Store = std::fs::read_to_string(config_file())
            .ok()
            .and_then(|text| match serde_json::from_str(&text) {
                Ok(store) => Some(store),
                Err(e) => {
                    eprintln!("frontrise: ignoring unreadable config: {e}");
                    None
                }
            })
            .unwrap_or_default();
        store.ensure_builtins();
        store
    }

    /// Built-in scenes always exist in the library (their favorite flag persists).
    fn ensure_builtins(&mut self) {
        for (key, title) in scene::BUILTINS.iter().map(|b| (b.key, b.title)).rev() {
            let id = format!("builtin-{key}");
            if !self.library.iter().any(|i| i.id == id) {
                self.library.insert(
                    0,
                    Item {
                        id,
                        title: title.to_owned(),
                        kind: Kind::Scene,
                        path: None,
                        builtin: Some(key.to_owned()),
                        favorite: false,
                    },
                );
            }
        }
    }

    pub fn save(&self) {
        let dir = config_dir();
        let result = std::fs::create_dir_all(&dir).and_then(|_| {
            let tmp = dir.join("config.json.tmp");
            let text = serde_json::to_string_pretty(self).expect("config serializes");
            std::fs::write(&tmp, text)?;
            std::fs::rename(&tmp, config_file())
        });
        if let Err(e) = result {
            eprintln!("frontrise: could not save config: {e}");
        }
    }

    /// Whether the user allowed automatic repairs for installation
    /// `install`; `None` when they have not answered for it yet.
    pub fn repair_consent(&self, install: &str) -> Option<bool> {
        self.extension_repair.as_ref().filter(|c| c.install == install).map(|c| c.allow)
    }

    pub fn set_repair_consent(&mut self, install: &str, allow: bool) {
        self.extension_repair = Some(RepairConsent { install: install.to_owned(), allow });
    }

    pub fn item(&self, id: &str) -> Option<&Item> {
        self.library.iter().find(|i| i.id == id)
    }

    pub fn item_mut(&mut self, id: &str) -> Option<&mut Item> {
        self.library.iter_mut().find(|i| i.id == id)
    }

    pub fn playlist(&self, id: &str) -> Option<&Playlist> {
        self.playlists.iter().find(|p| p.id == id)
    }

    pub fn playlist_mut(&mut self, id: &str) -> Option<&mut Playlist> {
        self.playlists.iter_mut().find(|p| p.id == id)
    }

    pub fn display_mut(&mut self, connector: &str) -> &mut DisplayConfig {
        self.displays.entry(connector.to_owned()).or_default()
    }

    /// Adds a file to the library (or returns the existing entry for it).
    pub fn import(&mut self, path: &Path) -> Option<String> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_owned());
        if let Some(existing) = self.library.iter().find(|i| i.path.as_deref() == Some(&path)) {
            return Some(existing.id.clone());
        }
        let kind = kind_for_path(&path)?;
        let title = path
            .file_stem()
            .map(|s| s.to_string_lossy().replace(['_', '-'], " "))
            .unwrap_or_else(|| "Untitled".into());
        let id = new_id();
        self.library.push(Item {
            id: id.clone(),
            title,
            kind,
            path: Some(path),
            builtin: None,
            favorite: false,
        });
        Some(id)
    }

    /// Removes an item from the library and from every playlist and display.
    pub fn remove_item(&mut self, id: &str) {
        self.library.retain(|i| i.id != id || i.is_builtin());
        for p in &mut self.playlists {
            p.items.retain(|i| i != id);
        }
        for d in self.displays.values_mut() {
            if d.source == Source::Item(id.to_owned()) {
                d.source = Source::Off;
            }
        }
    }

    pub fn remove_playlist(&mut self, id: &str) {
        self.playlists.retain(|p| p.id != id);
        for d in self.displays.values_mut() {
            if d.source == Source::Playlist(id.to_owned()) {
                d.source = Source::Off;
            }
        }
    }

    /// Shows `source` on every given display, in sync: the first display owns
    /// the content and the others mirror it (one decoder for all screens).
    pub fn apply_everywhere(&mut self, connectors: &[String], source: Source) {
        let Some((first, rest)) = connectors.split_first() else {
            return;
        };
        self.display_mut(first).source = source;
        for c in rest {
            self.display_mut(c).source = Source::SameAs(first.clone());
        }
    }
}

pub fn kind_for_path(path: &Path) -> Option<Kind> {
    let ext = path.extension()?.to_string_lossy().to_ascii_lowercase();
    match ext.as_str() {
        "glsl" | "frag" | "fs" | "shader" => Some(Kind::Scene),
        "mp4" | "m4v" | "mov" | "webm" | "mkv" | "avi" | "ogv" | "gif" => Some(Kind::Video),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn repair_consent_holds_for_one_installation() {
        let mut store = Store::default();
        assert_eq!(store.repair_consent("0.3.2@100"), None);
        store.set_repair_consent("0.3.2@100", false);
        assert_eq!(store.repair_consent("0.3.2@100"), Some(false));
        // An update or a reinstall forgets the answer.
        assert_eq!(store.repair_consent("0.3.3@200"), None);
        assert_eq!(store.repair_consent("0.3.2@300"), None);
        store.set_repair_consent("0.3.3@200", true);
        assert_eq!(store.repair_consent("0.3.3@200"), Some(true));
    }

    #[test]
    fn repair_consent_survives_a_save() {
        let mut store = Store::default();
        let text = serde_json::to_string(&store).unwrap();
        assert!(!text.contains("extension_repair"));
        store.set_repair_consent("1.0@5", true);
        let text = serde_json::to_string(&store).unwrap();
        let back: Store = serde_json::from_str(&text).unwrap();
        assert_eq!(back.repair_consent("1.0@5"), Some(true));
        // Older config files have no answer.
        let old: Store = serde_json::from_str(r#"{"settings": {}}"#).unwrap();
        assert_eq!(old.extension_repair, None);
    }

    #[test]
    fn install_stamp_combines_version_and_binary_time() {
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(stamp("0.3.2", Some(t)), "0.3.2@1700000000");
        assert_eq!(stamp("0.3.2", Some(UNIX_EPOCH)), "0.3.2");
        assert_eq!(stamp("0.3.2", None), "0.3.2");
        assert!(install_stamp().starts_with(env!("CARGO_PKG_VERSION")));
    }
}
