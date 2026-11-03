//! Battery (UPower) and power-saver (power-profiles-daemon via GIO) watchers.

use std::rc::Rc;

use gio::prelude::*;

use crate::engine::{Condition, Engine};

pub fn watch(engine: &Rc<Engine>) {
    let monitor = gio::PowerProfileMonitor::get_default();
    engine.set_condition(Condition::PowerSaver, monitor.is_power_saver_enabled());
    let weak = Rc::downgrade(engine);
    monitor.connect_power_saver_enabled_notify(move |m| {
        if let Some(engine) = weak.upgrade() {
            engine.set_condition(Condition::PowerSaver, m.is_power_saver_enabled());
        }
    });
    engine.keep_alive(monitor.upcast());

    let weak = Rc::downgrade(engine);
    gio::DBusProxy::for_bus(
        gio::BusType::System,
        gio::DBusProxyFlags::NONE,
        None,
        "org.freedesktop.UPower",
        "/org/freedesktop/UPower",
        "org.freedesktop.UPower",
        gio::Cancellable::NONE,
        move |result| {
            let Ok(proxy) = result else {
                eprintln!("frontrise: UPower unavailable, battery pause disabled");
                return;
            };
            let Some(engine) = weak.upgrade() else { return };
            let read = |p: &gio::DBusProxy| {
                p.cached_property("OnBattery").and_then(|v| v.get::<bool>()).unwrap_or(false)
            };
            engine.set_condition(Condition::Battery, read(&proxy));
            let weak = Rc::downgrade(&engine);
            proxy.connect_local("g-properties-changed", false, move |args| {
                if let (Some(engine), Ok(p)) = (weak.upgrade(), args[0].get::<gio::DBusProxy>()) {
                    engine.set_condition(Condition::Battery, read(&p));
                }
                None
            });
            engine.keep_alive(proxy.upcast());
        },
    );
}
