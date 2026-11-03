//! "Start at login" through an XDG autostart entry.

use std::path::PathBuf;

use crate::APP_ID;

fn entry_path() -> PathBuf {
    glib::user_config_dir().join("autostart").join(format!("{APP_ID}.desktop"))
}

pub fn set(enabled: bool) {
    let path = entry_path();
    if !enabled {
        if path.exists() {
            if let Err(e) = std::fs::remove_file(&path) {
                eprintln!("frontrise: could not remove {}: {e}", path.display());
            }
        }
        return;
    }
    let exe = std::env::current_exe()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|_| "frontrise".into());
    let content = format!(
        "[Desktop Entry]\nType=Application\nName=Frontrise\n\
         Comment=Animated wallpapers\nExec=\"{exe}\" --background\nIcon={APP_ID}\n\
         NoDisplay=true\nX-GNOME-Autostart-enabled=true\n"
    );
    let result = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|_| std::fs::write(&path, content));
    if let Err(e) = result {
        eprintln!("frontrise: could not write {}: {e}", path.display());
    }
}
