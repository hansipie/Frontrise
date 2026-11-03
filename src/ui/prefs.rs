//! Preferences dialog.

use std::rc::Rc;

use adw::prelude::*;

use crate::app::Ctx;
use crate::autostart;
use crate::config::Settings;

fn switch(
    ctx: &Rc<Ctx>,
    group: &adw::PreferencesGroup,
    title: &str,
    subtitle: &str,
    get: fn(&Settings) -> bool,
    set: fn(&mut Settings, bool),
    after: Option<fn(bool)>,
) {
    let row = adw::SwitchRow::builder()
        .title(title)
        .subtitle(subtitle)
        .active(get(&ctx.state.store.borrow().settings))
        .build();
    let c = ctx.clone();
    row.connect_active_notify(move |row| {
        let on = row.is_active();
        c.state.modify_quiet(|s| set(&mut s.settings, on));
        c.engine.settings_changed();
        if let Some(after) = after {
            after(on);
        }
    });
    group.add(&row);
}

pub fn present(ctx: &Rc<Ctx>) {
    let page = adw::PreferencesPage::builder()
        .title("General")
        .icon_name("emblem-system-symbolic")
        .build();

    let pause = adw::PreferencesGroup::builder()
        .title("Automatic Pause")
        .description("Pausing keeps the current frame on screen and saves energy.")
        .build();
    switch(ctx, &pause, "On battery", "Pause while the computer runs on battery",
        |s| s.pause_on_battery, |s, v| s.pause_on_battery = v, None);
    switch(ctx, &pause, "In power saver mode", "Pause while the Power Saver profile is on",
        |s| s.pause_on_power_saver, |s, v| s.pause_on_power_saver = v, None);
    switch(ctx, &pause, "When an app is fullscreen", "Pause during games and fullscreen videos",
        |s| s.pause_when_fullscreen, |s, v| s.pause_when_fullscreen = v, None);
    page.add(&pause);

    let scenes = adw::PreferencesGroup::builder().title("Scenes").build();
    switch(ctx, &scenes, "React to sound",
        "Sound-reactive scenes follow what your computer plays. Only the loudness of each pitch is used; nothing is recorded or leaves your computer.",
        |s| s.react_to_sound, |s, v| s.react_to_sound = v, None);
    page.add(&scenes);

    let system = adw::PreferencesGroup::builder().title("System").build();
    switch(ctx, &system, "Start at login", "Play your wallpapers as soon as you log in",
        |s| s.autostart, |s, v| s.autostart = v, Some(autostart::set));
    switch(ctx, &system, "Show on lock screen", "Takes effect the next time the screen locks",
        |s| s.lock_screen, |s, v| s.lock_screen = v, None);
    switch(ctx, &system, "Top bar icon", "Quick controls in the GNOME top bar",
        |s| s.panel_icon, |s, v| s.panel_icon = v, None);
    switch(ctx, &system, "Hardware video decoding",
        "Decode with the GPU. Can use more memory and CPU on some NVIDIA setups. Takes effect after restarting Frontrise",
        |s| s.hardware_decoding, |s, v| s.hardware_decoding = v, None);
    page.add(&system);

    let dialog = adw::PreferencesDialog::new();
    dialog.add(&page);
    dialog.present(crate::ui::parent(ctx).as_ref());
}
