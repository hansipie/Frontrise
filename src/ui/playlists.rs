//! Playlists page: create playlists, set interval and shuffle, reorder items.

use std::cell::RefCell;
use std::rc::Rc;

use adw::prelude::*;
use gtk::glib;

use crate::app::Ctx;
use crate::config::Playlist;

pub fn page(ctx: &Rc<Ctx>) -> gtk::Widget {
    let page = adw::PreferencesPage::new();
    let groups: Rc<RefCell<Vec<adw::PreferencesGroup>>> = Rc::default();

    let refresh: Rc<dyn Fn()> = {
        let ctx = ctx.clone();
        let page = page.downgrade();
        let groups = groups.clone();
        Rc::new(move || {
            let Some(page) = page.upgrade() else { return };
            for g in groups.borrow_mut().drain(..) {
                page.remove(&g);
            }
            for g in build_groups(&ctx) {
                page.add(&g);
                groups.borrow_mut().push(g);
            }
        })
    };
    refresh();
    let r = refresh.clone();
    ctx.state.connect_ui(move || r());
    page.upcast()
}

fn icon_button(icon: &str, tooltip: &str) -> gtk::Button {
    gtk::Button::builder()
        .icon_name(icon)
        .tooltip_text(tooltip)
        .css_classes(["flat"])
        .valign(gtk::Align::Center)
        .build()
}

fn build_groups(ctx: &Rc<Ctx>) -> Vec<adw::PreferencesGroup> {
    let store = ctx.state.store.borrow();
    let mut groups = vec![];

    let intro = adw::PreferencesGroup::builder()
        .title("Playlists")
        .description("Cycle through wallpapers automatically. Add wallpapers from the Library with ⋯ → Add to Playlist.")
        .build();
    let new = gtk::Button::builder()
        .label("New Playlist")
        .css_classes(["suggested-action"])
        .valign(gtk::Align::Center)
        .build();
    new.connect_clicked(|b| {
        let _ = b.activate_action("win.new-playlist-with", Some(&"".to_variant()));
    });
    intro.set_header_suffix(Some(&new));
    if store.playlists.is_empty() {
        intro.add(
            &adw::ActionRow::builder()
                .title("No playlists yet")
                .subtitle("Create one, then add wallpapers to it from the Library.")
                .build(),
        );
    }
    groups.push(intro);

    for p in &store.playlists {
        groups.push(playlist_group(ctx, p, &store));
    }
    groups
}

fn playlist_group(ctx: &Rc<Ctx>, p: &Playlist, store: &crate::config::Store) -> adw::PreferencesGroup {
    let n = p.items.len();
    let group = adw::PreferencesGroup::builder()
        .title(glib::markup_escape_text(&p.name).as_str())
        .description(format!("{n} wallpaper{}", if n == 1 { "" } else { "s" }))
        .build();

    let actions = gtk::Box::builder().spacing(4).build();
    let play = icon_button("media-playback-start-symbolic", "Play on All Displays");
    let id = p.id.clone();
    play.connect_clicked(move |b| {
        let _ = b.activate_action("win.play-playlist", Some(&id.to_variant()));
    });
    let rename = icon_button("document-edit-symbolic", "Rename");
    let (c, id, name) = (ctx.clone(), p.id.clone(), p.name.clone());
    rename.connect_clicked(move |_| {
        let (c2, id) = (c.clone(), id.clone());
        crate::ui::ask_name(&c, "Rename Playlist", &name, "Rename", move |new_name| {
            c2.state.modify(|s| {
                if let Some(p) = s.playlist_mut(&id) {
                    p.name = new_name;
                }
            });
        });
    });
    let delete = icon_button("user-trash-symbolic", "Delete Playlist");
    let (c, id, name) = (ctx.clone(), p.id.clone(), p.name.clone());
    delete.connect_clicked(move |_| {
        let (c2, id) = (c.clone(), id.clone());
        crate::ui::confirm(
            &c,
            &format!("Delete “{name}”?"),
            "Its wallpapers stay in your library.",
            "Delete",
            move || c2.state.modify(|s| s.remove_playlist(&id)),
        );
    });
    actions.append(&play);
    actions.append(&rename);
    actions.append(&delete);
    group.set_header_suffix(Some(&actions));

    let interval = adw::SpinRow::with_range(1.0, 1440.0, 1.0);
    interval.set_title("Change wallpaper every");
    interval.set_subtitle("minutes");
    interval.set_value(p.interval_minutes as f64);
    let (c, id) = (ctx.clone(), p.id.clone());
    interval.connect_value_notify(move |row| {
        let v = row.value() as u32;
        c.state.modify_quiet(|s| {
            if let Some(p) = s.playlist_mut(&id) {
                p.interval_minutes = v;
            }
        });
    });
    group.add(&interval);

    let shuffle = adw::SwitchRow::builder().title("Shuffle").active(p.shuffle).build();
    let (c, id) = (ctx.clone(), p.id.clone());
    shuffle.connect_active_notify(move |row| {
        let on = row.is_active();
        c.state.modify_quiet(|s| {
            if let Some(p) = s.playlist_mut(&id) {
                p.shuffle = on;
            }
        });
    });
    group.add(&shuffle);

    if p.items.is_empty() {
        group.add(
            &adw::ActionRow::builder()
                .title("Empty playlist")
                .subtitle("In the Library, use ⋯ → Add to Playlist.")
                .build(),
        );
    }
    for (index, item_id) in p.items.iter().enumerate() {
        let (title, subtitle) = match store.item(item_id) {
            Some(i) if i.is_available() => (i.title.clone(), i.kind_label().to_owned()),
            Some(i) => (i.title.clone(), "Missing file — skipped".to_owned()),
            None => ("Removed wallpaper".to_owned(), String::new()),
        };
        let row = adw::ActionRow::builder()
            .title(glib::markup_escape_text(&title).as_str())
            .subtitle(subtitle)
            .build();
        let up = icon_button("go-up-symbolic", "Move Up");
        up.set_sensitive(index > 0);
        let down = icon_button("go-down-symbolic", "Move Down");
        down.set_sensitive(index + 1 < p.items.len());
        let remove = icon_button("list-remove-symbolic", "Remove from Playlist");
        for (button, delta) in [(&up, -1isize), (&down, 1), (&remove, 0)] {
            let (c, id) = (ctx.clone(), p.id.clone());
            button.connect_clicked(move |_| {
                c.state.modify(|s| {
                    let Some(p) = s.playlist_mut(&id) else { return };
                    if index >= p.items.len() {
                        return;
                    }
                    if delta == 0 {
                        p.items.remove(index);
                    } else {
                        let other = index as isize + delta;
                        if other >= 0 && (other as usize) < p.items.len() {
                            p.items.swap(index, other as usize);
                        }
                    }
                });
            });
            row.add_suffix(button);
        }
        group.add(&row);
    }
    group
}
