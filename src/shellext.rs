//! Tracks whether the Frontrise GNOME Shell extension is running, and
//! repairs it when it can.
//! Wallpaper windows are only created while it is active: without it they
//! would show up as ordinary windows.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gio::prelude::*;

use crate::engine::Engine;
use crate::EXTENSION_UUID;

const IFACE: &str = "org.gnome.Shell.Extensions";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShellStatus {
    /// Not known yet.
    Unknown,
    /// No GNOME Shell on the session bus.
    Unavailable,
    /// GNOME Shell does not know the extension (not installed, or installed
    /// during this session: Wayland needs a new login to load it).
    NotLoaded,
    Inactive,
    Active,
    Failed,
    /// The extension does not declare the running GNOME Shell version.
    OutOfDate,
}

thread_local! {
    // One automatic attempt per problem, so a repair that does not take
    // never turns into a loop. Reset once the extension runs again.
    static ENABLE_TRIED: Cell<bool> = const { Cell::new(false) };
    static PATCH_TRIED: Cell<bool> = const { Cell::new(false) };
}

fn lookup<T: glib::variant::FromVariant>(info: &glib::Variant, key: &str) -> Option<T> {
    glib::VariantDict::new(Some(info)).lookup_value(key, None).and_then(|v| v.get::<T>())
}

fn status_from_info(info: &glib::Variant) -> ShellStatus {
    let Some(state) = lookup::<f64>(info, "state") else {
        return ShellStatus::NotLoaded;
    };
    match state as i32 {
        1 => ShellStatus::Active,
        3 => ShellStatus::Failed,
        4 => ShellStatus::OutOfDate,
        99 => ShellStatus::NotLoaded, // uninstalled
        _ => ShellStatus::Inactive,
    }
}

/// Records the new state and starts the matching repair, once.
fn on_info(proxy: &gio::DBusProxy, engine: &Rc<Engine>, info: &glib::Variant) {
    let status = status_from_info(info);
    engine.set_shell_status(status);
    match status {
        ShellStatus::Active => {
            ENABLE_TRIED.set(false);
            PATCH_TRIED.set(false);
            engine.set_shell_note(None);
        }
        ShellStatus::Inactive if !ENABLE_TRIED.replace(true) => {
            // Already in the enabled list but still off: the global
            // "Extensions" switch is off.
            if lookup::<bool>(info, "enabled") == Some(true) {
                enable_user_extensions(proxy, engine);
            }
            enable(proxy, engine);
        }
        ShellStatus::OutOfDate if !PATCH_TRIED.replace(true) => {
            if let Some(path) = lookup::<String>(info, "path") {
                update_metadata(proxy, engine, path);
            }
        }
        ShellStatus::Failed => {
            let error = lookup::<String>(info, "error").filter(|e| !e.is_empty());
            engine.set_shell_note(error.map(|e| format!("The Frontrise GNOME extension failed to start: {e}")));
        }
        _ => {}
    }
}

fn query(proxy: &gio::DBusProxy, engine: &Rc<Engine>) {
    if proxy.name_owner().is_none() {
        engine.set_shell_status(ShellStatus::Unavailable);
        return;
    }
    let weak = Rc::downgrade(engine);
    let proxy2 = proxy.clone();
    proxy.call(
        "GetExtensionInfo",
        Some(&(EXTENSION_UUID,).to_variant()),
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        move |result| {
            let Some(engine) = weak.upgrade() else { return };
            match result {
                Ok(v) => on_info(&proxy2, &engine, &v.child_value(0)),
                Err(_) => engine.set_shell_status(ShellStatus::Unavailable),
            }
        },
    );
}

pub fn watch(engine: &Rc<Engine>) {
    let weak = Rc::downgrade(engine);
    gio::DBusProxy::for_bus(
        gio::BusType::Session,
        gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES,
        None,
        IFACE,
        "/org/gnome/Shell/Extensions",
        IFACE,
        gio::Cancellable::NONE,
        move |result| {
            let Some(engine) = weak.upgrade() else { return };
            let Ok(proxy) = result else {
                engine.set_shell_status(ShellStatus::Unavailable);
                return;
            };
            query(&proxy, &engine);

            let weak = Rc::downgrade(&engine);
            proxy.connect_local("g-signal", false, move |args| {
                let engine = weak.upgrade()?;
                let proxy: gio::DBusProxy = args[0].get().ok()?;
                let signal: String = args[2].get().ok()?;
                let params: glib::Variant = args[3].get().ok()?;
                if signal == "ExtensionStateChanged"
                    && params.child_value(0).get::<String>().as_deref() == Some(EXTENSION_UUID)
                {
                    on_info(&proxy, &engine, &params.child_value(1));
                }
                None
            });
            // GNOME Shell restarted or went away.
            let weak = Rc::downgrade(&engine);
            proxy.connect_notify_local(Some("g-name-owner"), move |proxy, _| {
                if let Some(engine) = weak.upgrade() {
                    query(proxy, &engine);
                }
            });
            engine.set_shell_proxy(proxy);
        },
    );
}

/// Asks GNOME Shell to turn the extension on.
pub fn enable(proxy: &gio::DBusProxy, engine: &Rc<Engine>) {
    let weak = Rc::downgrade(engine);
    proxy.call(
        "EnableExtension",
        Some(&(EXTENSION_UUID,).to_variant()),
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        move |result| {
            let error = match result {
                Ok(v) if v.child_value(0).get::<bool>() != Some(false) => return,
                Ok(_) => "GNOME Shell refused the request".to_string(),
                Err(e) => e.to_string(),
            };
            eprintln!("frontrise: could not enable the extension: {error}");
            if let Some(engine) = weak.upgrade() {
                engine.set_shell_note(Some(format!(
                    "Frontrise could not turn its GNOME extension on ({error}). \
                     Try “gnome-extensions enable {EXTENSION_UUID}”."
                )));
            }
        },
    );
}

/// Turns the global "Extensions" switch back on.
fn enable_user_extensions(proxy: &gio::DBusProxy, engine: &Rc<Engine>) {
    let weak = Rc::downgrade(engine);
    proxy.call(
        "org.freedesktop.DBus.Properties.Set",
        Some(&(IFACE, "UserExtensionsEnabled", true.to_variant()).to_variant()),
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        move |result| {
            if let Err(e) = result {
                eprintln!("frontrise: could not turn user extensions on: {e}");
                if let Some(engine) = weak.upgrade() {
                    engine.set_shell_note(Some(format!(
                        "GNOME extensions are turned off and Frontrise could not turn them back on ({e})."
                    )));
                }
            }
        },
    );
}

/// Returns `metadata` with `major` added to its "shell-version" list, or
/// `None` when it is already there (or the file is not one we understand).
fn add_shell_version(metadata: &str, major: &str) -> Option<String> {
    let mut json: serde_json::Value = serde_json::from_str(metadata).ok()?;
    let versions = json.get_mut("shell-version")?.as_array_mut()?;
    if versions.iter().any(|v| v.as_str() == Some(major)) {
        return None;
    }
    versions.push(major.into());
    let mut out = serde_json::to_string_pretty(&json).ok()?;
    out.push('\n');
    Some(out)
}

/// GNOME Shell was upgraded past the versions the installed extension
/// declares: declare the running one too. GNOME only reads extension
/// metadata at login, so this takes effect on the next session.
fn update_metadata(proxy: &gio::DBusProxy, engine: &Rc<Engine>, dir: String) {
    let weak = Rc::downgrade(engine);
    proxy.call(
        "org.freedesktop.DBus.Properties.Get",
        Some(&(IFACE, "ShellVersion").to_variant()),
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        move |result| {
            let Some(engine) = weak.upgrade() else { return };
            let version = result
                .ok()
                .and_then(|v| v.child_value(0).as_variant())
                .and_then(|v| v.get::<String>());
            let Some(major) = version.as_deref().and_then(|v| v.split('.').next()).map(str::to_owned) else {
                engine.set_shell_note(Some(
                    "The Frontrise GNOME extension does not support this GNOME version.".into(),
                ));
                return;
            };
            let file = Path::new(&dir).join("metadata.json");
            let result = std::fs::read_to_string(&file).and_then(|text| match add_shell_version(&text, &major) {
                Some(updated) => std::fs::write(&file, updated).map(|_| true),
                None => Ok(false),
            });
            let note = match result {
                Ok(_) => format!(
                    "Frontrise updated its GNOME extension for GNOME {major}. \
                     Log out and back in once to start it."
                ),
                Err(e) => {
                    eprintln!("frontrise: could not update {}: {e}", file.display());
                    format!(
                        "The Frontrise GNOME extension does not support GNOME {major} \
                         and could not be updated ({e}). Reinstall Frontrise."
                    )
                }
            };
            engine.set_shell_note(Some(note));
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(state: f64) -> glib::Variant {
        let d = glib::VariantDict::new(None);
        d.insert("state", state);
        d.end()
    }

    #[test]
    fn maps_shell_states() {
        assert_eq!(status_from_info(&info(1.0)), ShellStatus::Active);
        assert_eq!(status_from_info(&info(2.0)), ShellStatus::Inactive);
        assert_eq!(status_from_info(&info(6.0)), ShellStatus::Inactive);
        assert_eq!(status_from_info(&info(3.0)), ShellStatus::Failed);
        assert_eq!(status_from_info(&info(4.0)), ShellStatus::OutOfDate);
        assert_eq!(status_from_info(&glib::VariantDict::new(None).end()), ShellStatus::NotLoaded);
    }

    #[test]
    fn adds_missing_shell_version_once() {
        let meta = r#"{"uuid": "frontrise@ansicode.fr", "shell-version": ["50"], "version": 2}"#;
        let updated = add_shell_version(meta, "51").unwrap();
        let json: serde_json::Value = serde_json::from_str(&updated).unwrap();
        assert_eq!(json["shell-version"], serde_json::json!(["50", "51"]));
        assert_eq!(json["uuid"], "frontrise@ansicode.fr");
        assert!(add_shell_version(&updated, "51").is_none());
        assert!(add_shell_version("not json", "51").is_none());
    }
}
