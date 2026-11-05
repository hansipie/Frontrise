//! Tracks whether the Frontrise GNOME Shell extension is running, and
//! repairs it when it can and the user allows it.
//! Wallpaper windows are only created while it is active: without it they
//! would show up as ordinary windows.

use std::cell::Cell;
use std::path::Path;
use std::rc::Rc;

use gio::prelude::*;

use crate::config::install_stamp;
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

/// A change Frontrise can make to fix the extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Repair {
    /// Turn the extension on; first turn the global "Extensions" switch on
    /// when `user_extensions` is set.
    Enable { user_extensions: bool },
    /// Declare the running Shell version in `dir`/metadata.json.
    Patch { dir: String },
}

/// What the UI is told about repairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellEvent {
    /// A repair is needed and the user has not said whether Frontrise may
    /// make it for this installation.
    AskRepair,
    /// A repair finished. `asked` is set when the user started it.
    Repaired { result: Result<(), String>, asked: bool },
}

/// Shown after every repair: GNOME only fully reloads an extension at login.
pub const LOGOUT_NOTE: &str =
    "Frontrise repaired its GNOME extension. Log out and back in for it to become active.";

thread_local! {
    // One automatic attempt (or question) per problem and launch, so a
    // repair that does not take never turns into a loop. Reset once the
    // extension runs again.
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

/// The repairs for the state in `info`. `asked` (the user pressed a Repair
/// button) also refreshes the metadata of a running extension, so the next
/// GNOME version change is covered too.
fn repairs_for(info: &glib::Variant, asked: bool) -> Vec<Repair> {
    let status = status_from_info(info);
    let path = lookup::<String>(info, "path").filter(|p| !p.is_empty());
    let mut repairs = Vec::new();
    if status == ShellStatus::Inactive {
        // Already in the enabled list but still off: the global
        // "Extensions" switch is off.
        let user_extensions = lookup::<bool>(info, "enabled") == Some(true);
        repairs.push(Repair::Enable { user_extensions });
    }
    if let Some(dir) = path {
        if status == ShellStatus::OutOfDate || (asked && status != ShellStatus::NotLoaded) {
            repairs.push(Repair::Patch { dir });
        }
    }
    repairs
}

/// Records the new state and, once per problem, repairs the extension when
/// the user allowed it for this installation, or asks them.
fn on_info(proxy: &gio::DBusProxy, engine: &Rc<Engine>, info: &glib::Variant) {
    let status = status_from_info(info);
    engine.set_shell_status(status);
    match status {
        ShellStatus::Active => {
            ENABLE_TRIED.set(false);
            PATCH_TRIED.set(false);
            engine.set_repair_wanted(false);
            engine.set_shell_note(None);
            return;
        }
        ShellStatus::Failed => {
            let error = lookup::<String>(info, "error").filter(|e| !e.is_empty());
            engine.set_shell_note(error.map(|e| format!("The Frontrise GNOME extension failed to start: {e}")));
            return;
        }
        _ => {}
    }
    let repairs = repairs_for(info, false);
    let guard = match repairs.first() {
        Some(Repair::Patch { .. }) => &PATCH_TRIED,
        Some(Repair::Enable { .. }) => &ENABLE_TRIED,
        None => return,
    };
    if guard.replace(true) {
        return;
    }
    let consent = engine.state().store.borrow().repair_consent(install_stamp());
    match consent {
        Some(true) => run(proxy, engine, repairs, false),
        // Declined for this installation: the banner and the preferences
        // still offer a manual repair.
        Some(false) => engine.set_repair_wanted(true),
        None => {
            engine.set_repair_wanted(true);
            engine.emit_shell_event(ShellEvent::AskRepair);
        }
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

/// Repairs the extension now, whatever the stored answer: the user asked
/// for it (Repair button in the question, the banner or the preferences).
pub fn repair_now(engine: &Rc<Engine>) {
    let Some(proxy) = engine.shell_proxy().filter(|p| p.name_owner().is_some()) else {
        engine.emit_shell_event(ShellEvent::Repaired {
            result: Err("GNOME Shell is not running, so the extension cannot be repaired.".into()),
            asked: true,
        });
        return;
    };
    let weak = Rc::downgrade(engine);
    glib::spawn_future_local(async move {
        let info = call(&proxy, "GetExtensionInfo", (EXTENSION_UUID,).to_variant()).await;
        let Some(engine) = weak.upgrade() else { return };
        let repairs = match info {
            Ok(v) => repairs_for(&v.child_value(0), true),
            Err(e) => {
                engine.emit_shell_event(ShellEvent::Repaired { result: Err(e), asked: true });
                return;
            }
        };
        if repairs.is_empty() {
            // Not installed, or installed during this session.
            engine.emit_shell_event(ShellEvent::Repaired {
                result: Err(format!(
                    "GNOME Shell has not loaded the Frontrise extension. If it is installed in \
                     ~/.local/share/gnome-shell/extensions/{EXTENSION_UUID}, log out and back in; \
                     otherwise reinstall Frontrise."
                )),
                asked: true,
            });
            return;
        }
        run(&proxy, &engine, repairs, true);
    });
}

/// Applies `repairs` in order, then reports the outcome.
fn run(proxy: &gio::DBusProxy, engine: &Rc<Engine>, repairs: Vec<Repair>, asked: bool) {
    let weak = Rc::downgrade(engine);
    let proxy = proxy.clone();
    glib::spawn_future_local(async move {
        let mut result = Ok(());
        for repair in &repairs {
            result = apply(&proxy, repair).await;
            if result.is_err() {
                break;
            }
        }
        let Some(engine) = weak.upgrade() else { return };
        match &result {
            Ok(()) => engine.set_shell_note(Some(LOGOUT_NOTE.into())),
            Err(e) => {
                eprintln!("frontrise: could not repair the extension: {e}");
                engine.set_shell_note(Some(e.clone()));
            }
        }
        engine.emit_shell_event(ShellEvent::Repaired { result, asked });
    });
}

async fn call(proxy: &gio::DBusProxy, method: &str, args: glib::Variant) -> Result<glib::Variant, String> {
    proxy
        .call_future(method, Some(&args), gio::DBusCallFlags::NONE, -1)
        .await
        .map_err(|e| e.to_string())
}

async fn apply(proxy: &gio::DBusProxy, repair: &Repair) -> Result<(), String> {
    match repair {
        Repair::Enable { user_extensions } => {
            if *user_extensions {
                enable_user_extensions(proxy).await?;
            }
            enable(proxy).await
        }
        Repair::Patch { dir } => update_metadata(proxy, dir).await,
    }
}

/// Asks GNOME Shell to turn the extension on.
async fn enable(proxy: &gio::DBusProxy) -> Result<(), String> {
    let error = match call(proxy, "EnableExtension", (EXTENSION_UUID,).to_variant()).await {
        Ok(v) if v.child_value(0).get::<bool>() != Some(false) => return Ok(()),
        Ok(_) => "GNOME Shell refused the request".to_string(),
        Err(e) => e,
    };
    Err(format!(
        "Frontrise could not turn its GNOME extension on ({error}). \
         Try “gnome-extensions enable {EXTENSION_UUID}”."
    ))
}

/// Turns the global "Extensions" switch back on.
async fn enable_user_extensions(proxy: &gio::DBusProxy) -> Result<(), String> {
    call(
        proxy,
        "org.freedesktop.DBus.Properties.Set",
        (IFACE, "UserExtensionsEnabled", true.to_variant()).to_variant(),
    )
    .await
    .map(|_| ())
    .map_err(|e| format!("GNOME extensions are turned off and Frontrise could not turn them back on ({e})."))
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

/// Makes the installed extension declare the running GNOME Shell version
/// (after a Shell upgrade past the versions it lists). GNOME only reads
/// extension metadata at login, so this takes effect on the next session.
async fn update_metadata(proxy: &gio::DBusProxy, dir: &str) -> Result<(), String> {
    let version = call(proxy, "org.freedesktop.DBus.Properties.Get", (IFACE, "ShellVersion").to_variant())
        .await
        .ok()
        .and_then(|v| v.child_value(0).as_variant())
        .and_then(|v| v.get::<String>());
    let Some(major) = version.as_deref().and_then(|v| v.split('.').next()).map(str::to_owned) else {
        return Err("The Frontrise GNOME extension does not support this GNOME version.".into());
    };
    let file = Path::new(dir).join("metadata.json");
    std::fs::read_to_string(&file)
        .and_then(|text| match add_shell_version(&text, &major) {
            Some(updated) => std::fs::write(&file, updated),
            None => Ok(()),
        })
        .map_err(|e| {
            eprintln!("frontrise: could not update {}: {e}", file.display());
            format!(
                "The Frontrise GNOME extension does not support GNOME {major} \
                 and could not be updated ({e}). Reinstall Frontrise."
            )
        })
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

    fn full_info(state: f64, enabled: bool, path: &str) -> glib::Variant {
        let d = glib::VariantDict::new(None);
        d.insert("state", state);
        d.insert("enabled", enabled);
        d.insert("path", path);
        d.end()
    }

    #[test]
    fn picks_repairs() {
        let dir = "/ext".to_string();
        // Automatic: only what is broken.
        assert_eq!(repairs_for(&full_info(1.0, true, "/ext"), false), vec![]);
        assert_eq!(
            repairs_for(&full_info(2.0, false, "/ext"), false),
            vec![Repair::Enable { user_extensions: false }]
        );
        assert_eq!(
            repairs_for(&full_info(2.0, true, "/ext"), false),
            vec![Repair::Enable { user_extensions: true }]
        );
        assert_eq!(repairs_for(&full_info(4.0, true, "/ext"), false), vec![Repair::Patch { dir: dir.clone() }]);
        assert_eq!(repairs_for(&full_info(3.0, true, "/ext"), false), vec![]);
        // Asked by the user: also refresh the metadata.
        assert_eq!(repairs_for(&full_info(1.0, true, "/ext"), true), vec![Repair::Patch { dir: dir.clone() }]);
        assert_eq!(
            repairs_for(&full_info(2.0, true, "/ext"), true),
            vec![Repair::Enable { user_extensions: true }, Repair::Patch { dir }]
        );
        // Not loaded: nothing Frontrise can do but a new login.
        assert_eq!(repairs_for(&glib::VariantDict::new(None).end(), true), vec![]);
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
