//! Tracks whether the Frontrise GNOME Shell extension is running, and
//! installs or repairs it when it can and the user allows it.
//! Wallpaper windows are only created while it is active: without it they
//! would show up as ordinary windows.
//!
//! The extension's files are embedded in the binary, so a package only has to
//! ship the binary: Frontrise installs (or refreshes) its own copy in
//! `$XDG_DATA_HOME/gnome-shell/extensions/<uuid>`.

use std::cell::Cell;
use std::io;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use gio::prelude::*;

use crate::config::install_stamp;
use crate::engine::Engine;
use crate::EXTENSION_UUID;

const IFACE: &str = "org.gnome.Shell.Extensions";

/// The files of `extension/`, as installed (path relative to the extension
/// directory, content). A test checks that no file of `extension/` is missing.
const EMBEDDED: &[(&str, &[u8])] = &[
    ("extension.js", include_bytes!("../extension/extension.js")),
    ("metadata.json", include_bytes!("../extension/metadata.json")),
    ("icons/frontrise-symbolic.svg", include_bytes!("../extension/icons/frontrise-symbolic.svg")),
];
const METADATA: &str = "metadata.json";

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

/// The copy of the extension in the user's extension directory, compared
/// with the one embedded in this binary.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Installed {
    Missing,
    /// A file differs from the embedded one (e.g. Frontrise was updated).
    Outdated,
    Current,
}

/// A change Frontrise can make to fix the extension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Repair {
    /// Write the embedded extension to the user's extension directory,
    /// declaring the running Shell version.
    Install,
    /// Turn the extension on; first turn the global "Extensions" switch on
    /// when `user_extensions` is set.
    Enable { user_extensions: bool },
}

/// What the UI is told about repairs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ShellEvent {
    /// A repair is needed and the user has not said whether Frontrise may
    /// make it for this installation.
    AskRepair,
    /// A repair finished: `Ok(true)` when something changed (a new login is
    /// needed), `Ok(false)` when there was nothing to change. `asked` is set
    /// when the user started it.
    Repaired { result: Result<bool, String>, asked: bool },
}

/// Shown after every install or repair: GNOME only fully reloads an
/// extension at login.
pub const LOGOUT_NOTE: &str =
    "Frontrise installed or repaired its GNOME extension. Log out and back in for it to become active.";

thread_local! {
    // One automatic attempt (or question) per problem and launch, so a
    // repair that does not take never turns into a loop. Reset once the
    // extension runs again (and, for installs, matches the embedded copy).
    static ENABLE_TRIED: Cell<bool> = const { Cell::new(false) };
    static INSTALL_TRIED: Cell<bool> = const { Cell::new(false) };
}

/// The user's copy of the extension (`XDG_DATA_HOME` aware).
pub fn extension_dir() -> PathBuf {
    glib::user_data_dir().join("gnome-shell").join("extensions").join(EXTENSION_UUID)
}

/// The state of the user's copy of the extension.
pub fn installed() -> Installed {
    installed_state(&extension_dir())
}

/// Whether `installed` (an installed metadata.json) is the embedded
/// `embedded` one, give or take Shell versions added at install time.
fn metadata_matches(installed: &str, embedded: &str) -> bool {
    let (Ok(mut a), Ok(mut b)) = (
        serde_json::from_str::<serde_json::Value>(installed),
        serde_json::from_str::<serde_json::Value>(embedded),
    ) else {
        return false;
    };
    let (Some(a_obj), Some(b_obj)) = (a.as_object_mut(), b.as_object_mut()) else {
        return false;
    };
    let a_versions = a_obj.remove("shell-version");
    let b_versions = b_obj.remove("shell-version");
    let covers = match (a_versions.as_ref().and_then(|v| v.as_array()), b_versions.as_ref().and_then(|v| v.as_array()))
    {
        (Some(a), Some(b)) => b.iter().all(|v| a.contains(v)),
        (None, None) => true,
        _ => false,
    };
    covers && a == b
}

fn installed_state(dir: &Path) -> Installed {
    if !dir.join(METADATA).is_file() {
        return Installed::Missing;
    }
    let current = EMBEDDED.iter().all(|(name, content)| match std::fs::read(dir.join(name)) {
        Ok(found) if *name == METADATA => match (std::str::from_utf8(&found), std::str::from_utf8(content)) {
            (Ok(found), Ok(embedded)) => metadata_matches(found, embedded),
            _ => false,
        },
        Ok(found) => found == *content,
        Err(_) => false,
    });
    if current {
        Installed::Current
    } else {
        Installed::Outdated
    }
}

/// Writes the embedded extension to `dir`, with `major` added to the Shell
/// versions of metadata.json when missing. Only files that differ are
/// rewritten (each atomically); other files in `dir` are left alone.
/// Returns whether anything changed.
fn install_into(dir: &Path, major: Option<&str>) -> io::Result<bool> {
    let mut changed = false;
    for (name, content) in EMBEDDED {
        let content: std::borrow::Cow<[u8]> = match (*name == METADATA, major) {
            (true, Some(major)) => {
                let text = std::str::from_utf8(content).map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e))?;
                match add_shell_version(text, major) {
                    Some(updated) => updated.into_bytes().into(),
                    None => (*content).into(),
                }
            }
            _ => (*content).into(),
        };
        let path = dir.join(name);
        if std::fs::read(&path).is_ok_and(|found| found == *content) {
            continue;
        }
        let parent = path.parent().unwrap_or(dir);
        std::fs::create_dir_all(parent)?;
        let tmp = parent.join(format!(
            ".{}.frontrise-tmp",
            path.file_name().and_then(|n| n.to_str()).unwrap_or("file")
        ));
        std::fs::write(&tmp, &content).and_then(|()| std::fs::rename(&tmp, &path)).inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })?;
        changed = true;
    }
    Ok(changed)
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

/// The repairs for `status` (with `enabled`, whether the extension is in
/// GNOME's enabled list) and the state of the user's copy. `asked` (the user
/// pressed a Repair button) also rewrites a current copy of a loaded
/// extension, so the running Shell version gets declared too.
fn repairs_for(status: ShellStatus, enabled: bool, installed: Installed, asked: bool) -> Vec<Repair> {
    let install = match installed {
        Installed::Outdated => true,
        // Missing from the user directory: install it when GNOME has none
        // (first run of a packaged Frontrise), or only an unusable one.
        Installed::Missing => matches!(status, ShellStatus::NotLoaded | ShellStatus::OutOfDate),
        Installed::Current => {
            status == ShellStatus::OutOfDate || (asked && status != ShellStatus::NotLoaded)
        }
    };
    let mut repairs = Vec::new();
    if install {
        repairs.push(Repair::Install);
    }
    if status == ShellStatus::Inactive {
        // Already in the enabled list but still off: the global
        // "Extensions" switch is off.
        repairs.push(Repair::Enable { user_extensions: enabled });
    }
    repairs
}

fn repairs_for_info(info: &glib::Variant, asked: bool) -> Vec<Repair> {
    let enabled = lookup::<bool>(info, "enabled") == Some(true);
    repairs_for(status_from_info(info), enabled, installed(), asked)
}

fn guard(repair: &Repair) -> &'static std::thread::LocalKey<Cell<bool>> {
    match repair {
        Repair::Install => &INSTALL_TRIED,
        Repair::Enable { .. } => &ENABLE_TRIED,
    }
}

/// The problem a repair would fix, as the start of a sentence for the UI.
pub fn problem_text(status: ShellStatus) -> &'static str {
    match (status, installed()) {
        (ShellStatus::OutOfDate, _) => {
            "The Frontrise GNOME extension does not declare support for this GNOME version, so wallpapers are stopped."
        }
        (_, Installed::Missing) => "The Frontrise GNOME extension is not installed, so wallpapers are stopped.",
        (ShellStatus::Active, Installed::Outdated) => {
            "The Frontrise GNOME extension is older than this version of Frontrise."
        }
        (_, Installed::Outdated) => {
            "The Frontrise GNOME extension is older than this version of Frontrise, so wallpapers are stopped."
        }
        _ => "The Frontrise GNOME extension is turned off, so wallpapers are stopped.",
    }
}

/// Records the new state and, once per problem, repairs the extension when
/// the user allowed it for this installation, or asks them.
fn on_info(proxy: &gio::DBusProxy, engine: &Rc<Engine>, info: &glib::Variant) {
    let status = status_from_info(info);
    engine.set_shell_status(status);
    match status {
        ShellStatus::Active => {
            ENABLE_TRIED.set(false);
            if installed() == Installed::Current {
                INSTALL_TRIED.set(false);
            }
            engine.set_shell_note(None);
        }
        ShellStatus::Failed => {
            let error = lookup::<String>(info, "error").filter(|e| !e.is_empty());
            engine.set_shell_note(error.map(|e| format!("The Frontrise GNOME extension failed to start: {e}")));
        }
        _ => {}
    }
    let mut repairs = repairs_for_info(info, false);
    if repairs.is_empty() {
        engine.set_repair_wanted(false);
        return;
    }
    repairs.retain(|r| !guard(r).replace(true));
    if repairs.is_empty() {
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
        let info = match info {
            Ok(v) => v.child_value(0),
            Err(e) => {
                engine.emit_shell_event(ShellEvent::Repaired { result: Err(e), asked: true });
                return;
            }
        };
        let repairs = repairs_for_info(&info, true);
        if repairs.is_empty() {
            let message = match status_from_info(&info) {
                // Installed during this session.
                ShellStatus::NotLoaded => "GNOME Shell has not loaded the Frontrise extension yet. \
                                           Log out and back in once."
                    .to_string(),
                // Only a copy outside the user's directory, which Frontrise
                // leaves alone.
                _ => format!(
                    "GNOME Shell uses a Frontrise extension installed outside {}; \
                     Frontrise cannot repair it.",
                    extension_dir().display()
                ),
            };
            engine.emit_shell_event(ShellEvent::Repaired { result: Err(message), asked: true });
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
        let mut result = Ok(false);
        for repair in &repairs {
            match apply(&proxy, repair).await {
                Ok(changed) => result = result.map(|c| c || changed),
                Err(e) => {
                    result = Err(e);
                    break;
                }
            }
        }
        let Some(engine) = weak.upgrade() else { return };
        match &result {
            Ok(true) => engine.set_shell_note(Some(LOGOUT_NOTE.into())),
            Ok(false) => {}
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

/// Applies one repair; returns whether it changed something.
async fn apply(proxy: &gio::DBusProxy, repair: &Repair) -> Result<bool, String> {
    match repair {
        Repair::Enable { user_extensions } => {
            if *user_extensions {
                enable_user_extensions(proxy).await?;
            }
            enable(proxy).await.map(|()| true)
        }
        Repair::Install => install(proxy).await,
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

/// The major version of the running GNOME Shell ("50" for "50.6").
async fn shell_major(proxy: &gio::DBusProxy) -> Option<String> {
    call(proxy, "org.freedesktop.DBus.Properties.Get", (IFACE, "ShellVersion").to_variant())
        .await
        .ok()
        .and_then(|v| v.child_value(0).as_variant())
        .and_then(|v| v.get::<String>())
        .and_then(|v| v.split('.').next().filter(|m| !m.is_empty()).map(str::to_owned))
}

/// Installs or refreshes the user's copy of the extension, declaring the
/// running GNOME Shell version. GNOME only reads extensions at login, so this
/// takes effect on the next session.
async fn install(proxy: &gio::DBusProxy) -> Result<bool, String> {
    let Some(major) = shell_major(proxy).await else {
        return Err("Frontrise could not read the GNOME Shell version to install its extension.".into());
    };
    let dir = extension_dir();
    install_into(&dir, Some(&major)).map_err(|e| {
        eprintln!("frontrise: could not install the extension in {}: {e}", dir.display());
        format!("Frontrise could not install its GNOME extension in {} ({e}).", dir.display())
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

    #[test]
    fn picks_repairs() {
        use Installed::*;
        use ShellStatus::*;
        let enable = |user_extensions| Repair::Enable { user_extensions };
        // Automatic: only what is broken.
        assert_eq!(repairs_for(Active, true, Current, false), vec![]);
        assert_eq!(repairs_for(Inactive, false, Current, false), vec![enable(false)]);
        assert_eq!(repairs_for(Inactive, true, Current, false), vec![enable(true)]);
        assert_eq!(repairs_for(OutOfDate, true, Current, false), vec![Repair::Install]);
        assert_eq!(repairs_for(Failed, true, Current, false), vec![]);
        // Missing (packaged app, first run) or older than the app.
        assert_eq!(repairs_for(NotLoaded, false, Missing, false), vec![Repair::Install]);
        assert_eq!(repairs_for(OutOfDate, true, Missing, false), vec![Repair::Install]);
        assert_eq!(repairs_for(Active, true, Outdated, false), vec![Repair::Install]);
        assert_eq!(repairs_for(Failed, true, Outdated, false), vec![Repair::Install]);
        assert_eq!(repairs_for(Inactive, true, Outdated, false), vec![Repair::Install, enable(true)]);
        // A copy outside the user directory is left alone.
        assert_eq!(repairs_for(Active, true, Missing, false), vec![]);
        // Installed during this session: only a new login helps.
        assert_eq!(repairs_for(NotLoaded, false, Current, false), vec![]);
        assert_eq!(repairs_for(NotLoaded, false, Current, true), vec![]);
        // Asked by the user: also refresh a current copy.
        assert_eq!(repairs_for(Active, true, Current, true), vec![Repair::Install]);
        assert_eq!(repairs_for(Inactive, true, Current, true), vec![Repair::Install, enable(true)]);
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

    #[test]
    fn embeds_every_extension_file() {
        fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    walk(root, &path, out);
                } else {
                    out.push(path.strip_prefix(root).unwrap().to_string_lossy().into_owned());
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("extension");
        let mut on_disk = Vec::new();
        walk(&root, &root, &mut on_disk);
        on_disk.sort();
        let mut embedded: Vec<String> = EMBEDDED.iter().map(|(n, _)| n.to_string()).collect();
        embedded.sort();
        assert_eq!(on_disk, embedded, "extension/ and EMBEDDED differ");
        let meta: serde_json::Value = serde_json::from_slice(EMBEDDED[1].1).unwrap();
        assert_eq!(meta["uuid"], EXTENSION_UUID);
    }

    #[test]
    fn compares_metadata_ignoring_added_versions() {
        let embedded = r#"{"uuid": "u", "shell-version": ["50", "51"], "version": 2}"#;
        let added = add_shell_version(embedded, "52").unwrap();
        assert!(metadata_matches(embedded, embedded));
        assert!(metadata_matches(&added, embedded));
        // An older copy: lower version, or fewer Shell versions.
        assert!(!metadata_matches(r#"{"uuid": "u", "shell-version": ["50", "51"], "version": 1}"#, embedded));
        assert!(!metadata_matches(r#"{"uuid": "u", "shell-version": ["50"], "version": 2}"#, embedded));
        assert!(!metadata_matches("{", embedded));
    }

    fn temp_dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("frontrise-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    #[test]
    fn installs_and_refreshes_into_a_directory() {
        let dir = temp_dir("install");
        assert_eq!(installed_state(&dir), Installed::Missing);

        // First install, declaring a Shell version the extension lacks.
        assert!(install_into(&dir, Some("99")).unwrap());
        assert_eq!(installed_state(&dir), Installed::Current);
        assert_eq!(std::fs::read(dir.join("extension.js")).unwrap(), EMBEDDED[0].1);
        assert!(dir.join("icons/frontrise-symbolic.svg").is_file());
        let meta: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(METADATA)).unwrap()).unwrap();
        assert!(meta["shell-version"].as_array().unwrap().contains(&"99".into()));
        // Nothing left behind, nothing to do the second time.
        assert!(!std::fs::read_dir(&dir).unwrap().any(|e| e.unwrap().file_name().to_string_lossy().starts_with('.')));
        assert!(!install_into(&dir, Some("99")).unwrap());

        // An older copy (e.g. before an app update) is refreshed.
        std::fs::write(dir.join("extension.js"), "// old").unwrap();
        assert_eq!(installed_state(&dir), Installed::Outdated);
        assert!(install_into(&dir, None).unwrap());
        assert_eq!(installed_state(&dir), Installed::Current);

        // A missing file makes the copy outdated too.
        std::fs::remove_file(dir.join("icons/frontrise-symbolic.svg")).unwrap();
        assert_eq!(installed_state(&dir), Installed::Outdated);

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
