//! Displays page: what plays on each monitor, and how it fits.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, glib};

use crate::app::Ctx;
use crate::config::{Fit, Source, Store};
use crate::engine::{self, MonitorInfo};

pub fn page(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let groups: Rc<RefCell<Vec<adw::PreferencesGroup>>> = Rc::default();
    // "Now playing" rows, updated in place when the engine changes.
    let now_rows: Rc<RefCell<Vec<(String, adw::ActionRow)>>> = Rc::default();

    let refresh: Rc<dyn Fn()> = {
        let ctx = ctx.clone();
        let page = page.downgrade();
        let groups = groups.clone();
        let now_rows = now_rows.clone();
        Rc::new(move || {
            let Some(page) = page.upgrade() else { return };
            for g in groups.borrow_mut().drain(..) {
                page.remove(&g);
            }
            now_rows.borrow_mut().clear();
            for g in build_groups(&ctx, &now_rows) {
                page.add(&g);
                groups.borrow_mut().push(g);
            }
            update_now_playing(&ctx, &now_rows.borrow());
        })
    };
    refresh();
    let r = refresh.clone();
    ctx.state.connect_ui(move || r());
    if let Some(display) = gdk::Display::default() {
        let r = refresh.clone();
        display.monitors().connect_items_changed(move |_, _, _, _| {
            let r = r.clone();
            // Let the engine record the new monitor first.
            glib::idle_add_local_once(move || r());
        });
    }
    let c = ctx.clone();
    let rows = now_rows.clone();
    ctx.engine.connect_status(move || update_now_playing(&c, &rows.borrow()));
    page.upcast()
}

fn update_now_playing(ctx: &Ctx, rows: &[(String, adw::ActionRow)]) {
    for (connector, row) in rows {
        let text = ctx
            .engine
            .now_playing(connector)
            .unwrap_or_else(|| "Nothing — GNOME's wallpaper is shown".into());
        row.set_subtitle(&glib::markup_escape_text(&text));
    }
}

/// Options offered for a display, with their labels.
fn options(store: &Store, connector: &str, monitors: &[MonitorInfo]) -> Vec<(String, Source)> {
    let mut opts = vec![("GNOME wallpaper (Frontrise off)".to_owned(), Source::Off)];
    for m in monitors.iter().filter(|m| m.connector != connector) {
        opts.push((format!("Same as {} ({})", m.label, m.connector), Source::SameAs(m.connector.clone())));
    }
    for p in &store.playlists {
        opts.push((format!("Playlist: {}", p.name), Source::Playlist(p.id.clone())));
    }
    for i in &store.library {
        let suffix = match i.kind {
            crate::config::Kind::Scene => " (scene)",
            crate::config::Kind::Video => "",
        };
        opts.push((format!("{}{suffix}", i.title), Source::Item(i.id.clone())));
    }
    // Keep a current choice that isn't listed (e.g. a disconnected display).
    if let Some(d) = store.displays.get(connector) {
        if !opts.iter().any(|(_, s)| *s == d.source) {
            if let Source::SameAs(other) = &d.source {
                opts.push((format!("Same as {other} (disconnected)"), d.source.clone()));
            }
        }
    }
    opts
}

fn build_groups(ctx: &Rc<Ctx>, now_rows: &Rc<RefCell<Vec<(String, adw::ActionRow)>>>) -> Vec<adw::PreferencesGroup> {
    let monitors = engine::monitors();
    let store = ctx.state.store.borrow();
    let mut groups = vec![];

    for m in &monitors {
        let group = adw::PreferencesGroup::builder()
            .title(glib::markup_escape_text(&m.label).as_str())
            .description(format!("{} · {} × {}", m.connector, m.width, m.height))
            .build();

        let now = adw::ActionRow::builder().title("Now playing").build();
        now.add_css_class("property");
        group.add(&now);
        now_rows.borrow_mut().push((m.connector.clone(), now));

        let opts = options(&store, &m.connector, &monitors);
        let current = store.displays.get(&m.connector).map(|d| d.source.clone()).unwrap_or_default();
        let labels: Vec<&str> = opts.iter().map(|(l, _)| l.as_str()).collect();
        let source_row = adw::ComboRow::builder()
            .title("Wallpaper")
            .model(&gtk::StringList::new(&labels))
            .enable_search(true)
            .build();
        source_row.set_selected(opts.iter().position(|(_, s)| *s == current).unwrap_or(0) as u32);
        let (c, connector) = (ctx.clone(), m.connector.clone());
        let sources: Vec<Source> = opts.into_iter().map(|(_, s)| s).collect();
        source_row.connect_selected_notify(move |row| {
            if let Some(source) = sources.get(row.selected() as usize).cloned() {
                c.state.modify_quiet(|s| s.display_mut(&connector).source = source);
            }
        });
        group.add(&source_row);

        let fit = store.displays.get(&m.connector).map(|d| d.fit).unwrap_or_default();
        let fit_row = adw::ComboRow::builder()
            .title("Fit")
            .subtitle("Fill crops the edges; Fit shows the whole video with bars.")
            .model(&gtk::StringList::new(&["Fill screen", "Fit to screen"]))
            .selected(if fit == Fit::Contain { 1 } else { 0 })
            .build();
        let (c, connector) = (ctx.clone(), m.connector.clone());
        fit_row.connect_selected_notify(move |row| {
            let fit = if row.selected() == 1 { Fit::Contain } else { Fit::Cover };
            c.state.modify_quiet(|s| s.display_mut(&connector).fit = fit);
        });
        group.add(&fit_row);
        groups.push(group);
    }

    let disconnected: Vec<(&String, &crate::config::DisplayConfig)> = store
        .displays
        .iter()
        .filter(|(c, _)| !monitors.iter().any(|m| &m.connector == *c))
        .collect();
    if !disconnected.is_empty() {
        let group = adw::PreferencesGroup::builder()
            .title("Disconnected Displays")
            .description("Their settings come back when you plug them in again.")
            .build();
        for (connector, d) in disconnected {
            let label = if d.label.is_empty() { connector.clone() } else { d.label.clone() };
            let row = adw::ActionRow::builder()
                .title(glib::markup_escape_text(&label).as_str())
                .subtitle(connector.as_str())
                .build();
            let forget = gtk::Button::builder()
                .label("Forget")
                .valign(gtk::Align::Center)
                .build();
            let (c, connector) = (ctx.clone(), connector.clone());
            forget.connect_clicked(move |_| {
                c.state.modify(|s| {
                    s.displays.remove(&connector);
                    for d in s.displays.values_mut() {
                        if d.source == Source::SameAs(connector.clone()) {
                            d.source = Source::Off;
                        }
                    }
                });
            });
            row.add_suffix(&forget);
            group.add(&row);
        }
        groups.push(group);
    }

    if monitors.is_empty() {
        let group = adw::PreferencesGroup::new();
        group.add(&adw::ActionRow::builder().title("No display detected").build());
        groups.push(group);
    }
    groups
}
