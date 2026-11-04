//! Library page: a grid of wallpapers (videos and scenes).

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::app::Ctx;
use crate::config::{Item, Kind};
use crate::engine;
use crate::audio::Audio;
use crate::scene::{self, Scene};
use crate::thumbs;

/// Moment of the animation shown in a scene's library preview.
const SCENE_PREVIEW_TIME: f64 = 8.0;

thread_local! {
    /// Thumbnails being generated or that failed, so rebuilds don't retry.
    static PENDING: RefCell<HashSet<String>> = RefCell::default();
}

pub fn page(ctx: &Rc<Ctx>) -> gtk::Widget {
    let search = gtk::SearchEntry::builder()
        .placeholder_text("Search wallpapers")
        .hexpand(true)
        .build();
    let favorites = gtk::ToggleButton::builder()
        .icon_name("starred-symbolic")
        .tooltip_text("Show Favorites Only")
        .build();
    let bar = gtk::Box::builder()
        .spacing(8)
        .margin_top(12)
        .margin_bottom(6)
        .margin_start(18)
        .margin_end(18)
        .build();
    bar.append(&search);
    bar.append(&favorites);

    let flow = gtk::FlowBox::builder()
        .homogeneous(true)
        .selection_mode(gtk::SelectionMode::None)
        .min_children_per_line(1)
        .max_children_per_line(8)
        .column_spacing(18)
        .row_spacing(18)
        .margin_top(12)
        .margin_bottom(24)
        .margin_start(18)
        .margin_end(18)
        .valign(gtk::Align::Start)
        .build();
    let c = ctx.clone();
    flow.connect_child_activated(move |_, child| {
        let id = child.widget_name();
        if !id.is_empty() {
            let _ = gtk::prelude::WidgetExt::activate_action(
                child,
                "win.apply-all",
                Some(&id.to_variant()),
            );
        }
        let _ = &c;
    });
    let scrolled = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&flow)
        .build();

    let empty = adw::StatusPage::builder()
        .icon_name("folder-videos-symbolic")
        .title("No Wallpapers Here")
        .description("Import videos or .glsl scenes with the + button, or drop files on this window.")
        .vexpand(true)
        .build();

    let content = gtk::Stack::new();
    content.add_named(&scrolled, Some("grid"));
    content.add_named(&empty, Some("empty"));

    let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
    root.append(&bar);
    root.append(&content);

    let refresh: Rc<dyn Fn()> = {
        let ctx = ctx.clone();
        let flow = flow.downgrade();
        let content = content.downgrade();
        let search = search.downgrade();
        let favorites = favorites.downgrade();
        Rc::new(move || {
            let (Some(flow), Some(content), Some(search), Some(favorites)) =
                (flow.upgrade(), content.upgrade(), search.upgrade(), favorites.upgrade())
            else {
                return;
            };
            fill(&ctx, &flow, &content, &search.text(), favorites.is_active());
        })
    };
    refresh();
    let r = refresh.clone();
    search.connect_search_changed(move |_| r());
    let r = refresh.clone();
    favorites.connect_toggled(move |_| r());
    let r = refresh.clone();
    ctx.state.connect_ui(move || r());

    root.upcast()
}

fn fill(ctx: &Rc<Ctx>, flow: &gtk::FlowBox, content: &gtk::Stack, query: &str, favorites_only: bool) {
    flow.remove_all();
    let store = ctx.state.store.borrow();
    let query = query.to_lowercase();
    let monitors = engine::monitors();
    let mut shown = 0;
    for item in &store.library {
        if favorites_only && !item.favorite {
            continue;
        }
        if !query.is_empty() && !item.title.to_lowercase().contains(&query) {
            continue;
        }
        let child = gtk::FlowBoxChild::new();
        child.set_widget_name(&item.id);
        child.set_child(Some(&card(ctx, item, &store.playlists, &monitors)));
        flow.append(&child);
        shown += 1;
    }
    content.set_visible_child_name(if shown == 0 { "empty" } else { "grid" });
}

fn card(
    ctx: &Rc<Ctx>,
    item: &Item,
    playlists: &[crate::config::Playlist],
    monitors: &[engine::MonitorInfo],
) -> gtk::Widget {
    let overlay = gtk::Overlay::new();
    overlay.set_child(Some(&thumbnail(item)));
    let badge = |text: &str, warning: bool| {
        let label = gtk::Label::builder()
            .label(text)
            .halign(gtk::Align::Start)
            .valign(gtk::Align::Start)
            .css_classes(["frontrise-badge"])
            .build();
        if warning {
            label.add_css_class("warning");
        }
        label
    };
    if !item.is_available() {
        overlay.add_overlay(&badge("Missing file", true));
    } else if item.kind == Kind::Scene {
        overlay.add_overlay(&badge("Scene", false));
    }

    let title = gtk::Label::builder()
        .label(&item.title)
        .xalign(0.0)
        .hexpand(true)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .tooltip_text(&item.title)
        .build();

    let star = gtk::ToggleButton::builder()
        .icon_name(if item.favorite { "starred-symbolic" } else { "non-starred-symbolic" })
        .active(item.favorite)
        .tooltip_text("Favorite")
        .css_classes(["flat", "circular"])
        .valign(gtk::Align::Center)
        .build();
    let c = ctx.clone();
    let id = item.id.clone();
    star.connect_toggled(move |b| {
        let on = b.is_active();
        b.set_icon_name(if on { "starred-symbolic" } else { "non-starred-symbolic" });
        c.state.modify_quiet(|s| {
            if let Some(i) = s.item_mut(&id) {
                i.favorite = on;
            }
        });
    });

    let more = gtk::MenuButton::builder()
        .icon_name("view-more-symbolic")
        .menu_model(&menu(item, playlists, monitors))
        .tooltip_text("More")
        .css_classes(["flat", "circular"])
        .valign(gtk::Align::Center)
        .build();

    let row = gtk::Box::builder().spacing(4).build();
    row.append(&title);
    row.append(&star);
    row.append(&more);

    let card = gtk::Box::builder()
        .orientation(gtk::Orientation::Vertical)
        .spacing(6)
        .css_classes(["frontrise-card"])
        .build();
    card.append(&overlay);
    card.append(&row);
    card.upcast()
}

fn thumbnail(item: &Item) -> gtk::Widget {
    if item.kind == Kind::Scene {
        // Same size, corners and framing as video thumbnails: a real frame
        // of the shader, drawn once.
        if let Ok(source) = item.scene_source() {
            let area = scene::widget(Scene::still(source, SCENE_PREVIEW_TIME), Audio::new());
            // The card may be taller than 16:9 (the grid gives every card the
            // height of the tallest one), so the shader must fill whatever
            // it is given. An invisible 16:9 picture drives the height
            // exactly as a video thumbnail does; the shader covers it.
            let blank = gdk::MemoryTexture::new(
                16,
                9,
                gdk::MemoryFormat::R8g8b8a8,
                &glib::Bytes::from_owned(vec![0u8; 16 * 9 * 4]),
                16 * 4,
            );
            let sizer = gtk::Picture::builder()
                .paintable(&blank)
                .content_fit(gtk::ContentFit::Cover)
                .can_shrink(true)
                .width_request(256)
                .height_request(144)
                .build();
            area.set_hexpand(true);
            area.set_vexpand(true);
            let frame = gtk::Overlay::builder()
                .overflow(gtk::Overflow::Hidden)
                .css_classes(["frontrise-thumb"])
                .child(&sizer)
                .build();
            frame.add_overlay(&area);
            return frame.upcast();
        }
        let b = gtk::Box::builder()
            .css_classes(["frontrise-scene-thumb"])
            .width_request(256)
            .height_request(144)
            .build();
        return b.upcast();
    }

    let picture = gtk::Picture::builder()
        .content_fit(gtk::ContentFit::Cover)
        .can_shrink(true)
        .width_request(256)
        .height_request(144)
        .css_classes(["frontrise-thumb"])
        .build();
    let out = thumbs::path_for(&item.id);
    if out.exists() {
        picture.set_filename(Some(&out));
        return picture.upcast();
    }
    let Some(video) = item.path.clone().filter(|p| p.exists()) else {
        return picture.upcast();
    };
    let id = item.id.clone();
    let first = PENDING.with(|p| p.borrow_mut().insert(id.clone()));
    if first {
        let weak = picture.downgrade();
        glib::spawn_future_local(async move {
            let target = out.clone();
            let result = gio::spawn_blocking(move || thumbs::generate(&video, &target)).await;
            match result {
                Ok(Ok(())) => {
                    PENDING.with(|p| p.borrow_mut().remove(&id));
                    if let Some(picture) = weak.upgrade() {
                        picture.set_filename(Some(&out));
                    } else {
                        // The grid was rebuilt meanwhile: refresh it.
                        crate::app::ctx().state.notify_ui();
                    }
                }
                Ok(Err(e)) => eprintln!("frontrise: no thumbnail for {id}: {e}"),
                Err(_) => eprintln!("frontrise: thumbnail worker panicked for {id}"),
            }
        });
    }
    picture.upcast()
}

fn menu(item: &Item, playlists: &[crate::config::Playlist], monitors: &[engine::MonitorInfo]) -> gio::Menu {
    let entry = |label: &str, action: &str, target: &str| {
        let m = gio::MenuItem::new(Some(label), None);
        m.set_action_and_target_value(Some(action), Some(&target.to_variant()));
        m
    };
    let menu = gio::Menu::new();

    let apply = gio::Menu::new();
    apply.append_item(&entry("Use on All Displays", "win.apply-all", &item.id));
    if monitors.len() > 1 {
        for m in monitors {
            apply.append_item(&entry(
                &format!("Use on {} ({})", m.label, m.connector),
                "win.apply-on",
                &format!("{}|{}", item.id, m.connector),
            ));
        }
    }
    menu.append_section(None, &apply);

    let add = gio::Menu::new();
    for p in playlists {
        add.append_item(&entry(&p.name, "win.add-to-playlist", &format!("{}|{}", item.id, p.id)));
    }
    let new_section = gio::Menu::new();
    new_section.append_item(&entry("New Playlist…", "win.new-playlist-with", &item.id));
    add.append_section(None, &new_section);
    let playlist_section = gio::Menu::new();
    playlist_section.append_submenu(Some("Add to Playlist"), &add);
    menu.append_section(None, &playlist_section);

    if !item.is_builtin() {
        let remove = gio::Menu::new();
        remove.append_item(&entry("Remove from Library", "win.remove-item", &item.id));
        menu.append_section(None, &remove);
    }
    menu
}
