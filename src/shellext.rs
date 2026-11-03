//! Tracks whether the Frontrise GNOME Shell extension is running.
//! Wallpaper windows are only created while it is active: without it they
//! would show up as ordinary windows.

use std::rc::Rc;

use gio::prelude::*;

use crate::engine::Engine;
use crate::EXTENSION_UUID;

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
}

fn status_from_info(info: &glib::Variant) -> ShellStatus {
    let Some(state) = glib::VariantDict::new(Some(info)).lookup_value("state", None).and_then(|v| v.get::<f64>()) else {
        return ShellStatus::NotLoaded;
    };
    match state as i32 {
        1 => ShellStatus::Active,
        3 | 4 => ShellStatus::Failed,
        _ => ShellStatus::Inactive,
    }
}

fn query(proxy: &gio::DBusProxy, engine: &Rc<Engine>) {
    if proxy.name_owner().is_none() {
        engine.set_shell_status(ShellStatus::Unavailable);
        return;
    }
    let weak = Rc::downgrade(engine);
    proxy.call(
        "GetExtensionInfo",
        Some(&(EXTENSION_UUID,).to_variant()),
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        move |result| {
            let Some(engine) = weak.upgrade() else { return };
            let status = match result {
                Ok(v) => status_from_info(&v.child_value(0)),
                Err(_) => ShellStatus::Unavailable,
            };
            engine.set_shell_status(status);
        },
    );
}

pub fn watch(engine: &Rc<Engine>) {
    let weak = Rc::downgrade(engine);
    gio::DBusProxy::for_bus(
        gio::BusType::Session,
        gio::DBusProxyFlags::DO_NOT_LOAD_PROPERTIES,
        None,
        "org.gnome.Shell.Extensions",
        "/org/gnome/Shell/Extensions",
        "org.gnome.Shell.Extensions",
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
                let signal: String = args[2].get().ok()?;
                let params: glib::Variant = args[3].get().ok()?;
                if signal == "ExtensionStateChanged"
                    && params.child_value(0).get::<String>().as_deref() == Some(EXTENSION_UUID)
                {
                    engine.set_shell_status(status_from_info(&params.child_value(1)));
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
pub fn enable(proxy: &gio::DBusProxy) {
    proxy.call(
        "EnableExtension",
        Some(&(EXTENSION_UUID,).to_variant()),
        gio::DBusCallFlags::NONE,
        -1,
        gio::Cancellable::NONE,
        |result| {
            if let Err(e) = result {
                eprintln!("frontrise: could not enable the extension: {e}");
            }
        },
    );
}
