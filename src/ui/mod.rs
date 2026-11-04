//! Main window: Library, Playlists and Displays pages.

pub mod displays;
pub mod library;
pub mod playlists;
pub mod prefs;

use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::app::Ctx;
use crate::config::Source;
use crate::engine;
use crate::shellext::{self, ShellStatus};
use crate::APP_ID;

pub fn present(ctx: &Rc<Ctx>, page: Option<&str>) {
    let window = ctx.window.borrow().upgrade();
    let window = window.unwrap_or_else(|| build(ctx));
    if let (Some(page), Some(stack)) = (page, ctx.stack.borrow().upgrade()) {
        if stack.child_by_name(page).is_some() {
            stack.set_visible_child_name(page);
        }
    }
    window.present();
}

pub fn about(ctx: &Rc<Ctx>) {
    let dialog = adw::AboutDialog::builder()
        .application_name("Frontrise")
        .application_icon(APP_ID)
        .developer_name("ansicode")
        .version(env!("CARGO_PKG_VERSION"))
        .comments("Animated video and shader wallpapers for GNOME.")
        .build();
    dialog.present(ctx.window.borrow().upgrade().as_ref());
}

/// Connectors of the displays currently connected.
pub fn connectors() -> Vec<String> {
    engine::monitors().into_iter().map(|m| m.connector).collect()
}

pub fn toast(ctx: &Ctx, text: &str) {
    if let Some(window) = ctx.window.borrow().upgrade() {
        if let Some(overlay) = window.content().and_downcast::<adw::ToastOverlay>() {
            overlay.add_toast(adw::Toast::new(text));
        }
    }
}

pub fn parent(ctx: &Ctx) -> Option<adw::ApplicationWindow> {
    ctx.window.borrow().upgrade()
}

pub fn status_text(ctx: &Ctx) -> String {
    let engine = &ctx.engine;
    match engine.shell_status() {
        ShellStatus::Active => {}
        ShellStatus::Unknown => return "Starting…".into(),
        _ => return "Wallpapers are stopped until the Frontrise GNOME extension runs.".into(),
    }
    let n = engine.active_displays();
    if n == 0 {
        return "No display uses Frontrise. Pick a wallpaper in the Library.".into();
    }
    let reasons = engine.pause_reasons();
    if reasons.is_empty() {
        format!("Playing on {n} display{}", if n > 1 { "s" } else { "" })
    } else {
        format!("Paused — {}", reasons.join(", "))
    }
}

fn update_banner(ctx: &Ctx, banner: &adw::Banner) {
    let (text, button) = match ctx.engine.shell_status() {
        ShellStatus::Active | ShellStatus::Unknown => ("", None),
        ShellStatus::Inactive => ("The Frontrise GNOME extension is turned off.", Some("Turn On")),
        ShellStatus::NotLoaded => (
            "GNOME hasn't loaded the Frontrise extension yet. Log out and back in once after installing it.",
            None,
        ),
        ShellStatus::Failed => ("The Frontrise GNOME extension failed to start.", None),
        ShellStatus::Unavailable => ("Frontrise needs GNOME Shell to place wallpapers.", None),
    };
    banner.set_title(text);
    banner.set_button_label(button);
    banner.set_revealed(!text.is_empty());
}

/// Imports files and reports the result.
pub fn import_paths(ctx: &Ctx, paths: Vec<PathBuf>) {
    let mut added = 0;
    let mut rejected = 0;
    ctx.state.modify(|s| {
        for p in &paths {
            if crate::config::kind_for_path(p).is_some() && s.import(p).is_some() {
                added += 1;
            } else {
                rejected += 1;
            }
        }
    });
    let msg = match (added, rejected) {
        (0, _) => "Nothing imported: Frontrise accepts videos and .glsl/.frag scenes.".to_owned(),
        (a, 0) => format!("Imported {a} wallpaper{}", if a > 1 { "s" } else { "" }),
        (a, r) => format!("Imported {a}, skipped {r} unsupported file{}", if r > 1 { "s" } else { "" }),
    };
    toast(ctx, &msg);
}

fn import_dialog(ctx: &Rc<Ctx>) {
    let filter = gtk::FileFilter::new();
    filter.set_name(Some("Videos and scenes"));
    filter.add_mime_type("video/*");
    filter.add_mime_type("image/gif");
    for pattern in ["*.glsl", "*.frag", "*.fs", "*.shader"] {
        filter.add_pattern(pattern);
    }
    let filters = gio::ListStore::new::<gtk::FileFilter>();
    filters.append(&filter);
    let dialog = gtk::FileDialog::builder()
        .title("Import Wallpapers")
        .filters(&filters)
        .modal(true)
        .build();
    let ctx = ctx.clone();
    glib::spawn_future_local(async move {
        let parent = parent(&ctx);
        if let Ok(files) = dialog.open_multiple_future(parent.as_ref()).await {
            let paths = (0..files.n_items())
                .filter_map(|i| files.item(i).and_downcast::<gio::File>())
                .filter_map(|f| f.path())
                .collect();
            import_paths(&ctx, paths);
        }
    });
}

/// Asks for a name, then calls `done` with it.
pub fn ask_name(ctx: &Ctx, heading: &str, initial: &str, confirm: &str, done: impl Fn(String) + 'static) {
    let entry = gtk::Entry::builder().text(initial).activates_default(true).build();
    let dialog = adw::AlertDialog::new(Some(heading), None);
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", confirm);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("ok"));
    dialog.set_close_response("cancel");
    dialog.set_extra_child(Some(&entry));
    dialog.connect_response(None, move |_, response| {
        let name = entry.text().trim().to_owned();
        if response == "ok" && !name.is_empty() {
            done(name);
        }
    });
    dialog.present(parent(ctx).as_ref());
}

/// Asks for confirmation of a destructive action.
pub fn confirm(ctx: &Ctx, heading: &str, body: &str, label: &str, done: impl Fn() + 'static) {
    let dialog = adw::AlertDialog::new(Some(heading), Some(body));
    dialog.add_response("cancel", "Cancel");
    dialog.add_response("ok", label);
    dialog.set_response_appearance("ok", adw::ResponseAppearance::Destructive);
    dialog.set_close_response("cancel");
    dialog.connect_response(None, move |_, response| {
        if response == "ok" {
            done();
        }
    });
    dialog.present(parent(ctx).as_ref());
}

fn string_action(
    window: &adw::ApplicationWindow,
    name: &str,
    f: impl Fn(String) + 'static,
) {
    let action = gio::SimpleAction::new(name, Some(glib::VariantTy::STRING));
    action.connect_activate(move |_, value| {
        if let Some(s) = value.and_then(|v| v.get::<String>()) {
            f(s);
        }
    });
    window.add_action(&action);
}

fn split2(s: &str) -> Option<(String, String)> {
    s.split_once('|').map(|(a, b)| (a.to_owned(), b.to_owned()))
}

fn add_window_actions(ctx: &Rc<Ctx>, window: &adw::ApplicationWindow) {
    let c = ctx.clone();
    string_action(window, "apply-all", move |id| {
        let title = c.state.store.borrow().item(&id).map(|i| i.title.clone()).unwrap_or_default();
        let conns = connectors();
        c.state.modify(|s| s.apply_everywhere(&conns, Source::Item(id.clone())));
        toast(&c, &format!("“{title}” is now on every display"));
    });
    let c = ctx.clone();
    string_action(window, "apply-on", move |arg| {
        let Some((id, connector)) = split2(&arg) else { return };
        c.state.modify(|s| s.display_mut(&connector).source = Source::Item(id.clone()));
        toast(&c, &format!("Wallpaper set on {connector}"));
    });
    let c = ctx.clone();
    string_action(window, "play-playlist", move |id| {
        let conns = connectors();
        c.state.modify(|s| s.apply_everywhere(&conns, Source::Playlist(id.clone())));
        toast(&c, "Playlist playing on every display");
    });
    let c = ctx.clone();
    string_action(window, "add-to-playlist", move |arg| {
        let Some((id, playlist)) = split2(&arg) else { return };
        let mut name = String::new();
        c.state.modify(|s| {
            if let Some(p) = s.playlist_mut(&playlist) {
                if !p.items.contains(&id) {
                    p.items.push(id.clone());
                }
                name = p.name.clone();
            }
        });
        toast(&c, &format!("Added to “{name}”"));
    });
    let c = ctx.clone();
    string_action(window, "new-playlist-with", move |id| {
        let c2 = c.clone();
        ask_name(&c, "New Playlist", "", "Create", move |name| {
            let id = id.clone();
            c2.state.modify(|s| {
                s.playlists.push(crate::config::Playlist {
                    id: crate::config::new_id(),
                    name,
                    items: if id.is_empty() { vec![] } else { vec![id] },
                    interval_minutes: 10,
                    shuffle: false,
                })
            });
        });
    });
    let c = ctx.clone();
    string_action(window, "remove-item", move |id| {
        let title = c.state.store.borrow().item(&id).map(|i| i.title.clone()).unwrap_or_default();
        let c2 = c.clone();
        confirm(
            &c,
            &format!("Remove “{title}”?"),
            "It is removed from your library and playlists. The file itself is not deleted.",
            "Remove",
            move || {
                let _ = std::fs::remove_file(crate::thumbs::path_for(&id));
                c2.state.modify(|s| s.remove_item(&id));
            },
        );
    });
    let import = gio::SimpleAction::new("import", None);
    let c = ctx.clone();
    import.connect_activate(move |_, _| import_dialog(&c));
    window.add_action(&import);
}

fn build(ctx: &Rc<Ctx>) -> adw::ApplicationWindow {
    let window = adw::ApplicationWindow::builder()
        .application(&ctx.app)
        .title("Frontrise")
        .default_width(1120)
        .default_height(760)
        .width_request(360)
        .height_request(320)
        .build();
    *ctx.window.borrow_mut() = window.downgrade();
    add_window_actions(ctx, &window);

    let stack = adw::ViewStack::new();
    *ctx.stack.borrow_mut() = stack.downgrade();
    let library = library::page(ctx);
    stack.add_titled_with_icon(&library, Some("library"), "Library", "folder-pictures-symbolic");
    let playlists = playlists::page(ctx);
    stack.add_titled_with_icon(&playlists, Some("playlists"), "Playlists", "view-list-symbolic");
    let displays = displays::page(ctx);
    stack.add_titled_with_icon(&displays, Some("displays"), "Displays", "video-display-symbolic");

    let header = adw::HeaderBar::new();
    let switcher = adw::ViewSwitcher::builder()
        .stack(&stack)
        .policy(adw::ViewSwitcherPolicy::Wide)
        .build();
    header.set_title_widget(Some(&switcher));

    let import = gtk::Button::builder()
        .icon_name("list-add-symbolic")
        .tooltip_text("Import Videos or Scenes… (Ctrl+O)")
        .action_name("win.import")
        .build();
    header.pack_start(&import);

    let menu = gio::Menu::new();
    menu.append(Some("Preferences"), Some("app.preferences"));
    menu.append(Some("About Frontrise"), Some("app.about"));
    let quit_section = gio::Menu::new();
    quit_section.append(Some("Quit Frontrise"), Some("app.quit"));
    menu.append_section(None, &quit_section);
    let menu_button = gtk::MenuButton::builder()
        .icon_name("open-menu-symbolic")
        .menu_model(&menu)
        .primary(true)
        .tooltip_text("Main Menu")
        .build();
    header.pack_end(&menu_button);

    let next = gtk::Button::builder()
        .icon_name("media-skip-forward-symbolic")
        .tooltip_text("Next Wallpaper in Playlists")
        .action_name("app.next")
        .build();
    header.pack_end(&next);
    let pause = gtk::ToggleButton::builder()
        .icon_name("media-playback-pause-symbolic")
        .tooltip_text("Pause All Wallpapers")
        .action_name("app.paused")
        .build();
    header.pack_end(&pause);

    let banner = adw::Banner::new("");
    let c = ctx.clone();
    banner.connect_button_clicked(move |_| {
        if let Some(proxy) = c.engine.shell_proxy() {
            shellext::enable(&proxy);
        }
    });

    let status = gtk::Label::builder()
        .xalign(0.0)
        .css_classes(["dim-label", "caption", "frontrise-status"])
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.add_top_bar(&banner);
    toolbar.set_content(Some(&stack));
    toolbar.add_bottom_bar(&status);

    let overlay = adw::ToastOverlay::new();
    overlay.set_child(Some(&toolbar));
    window.set_content(Some(&overlay));

    let refresh_status = {
        let c = ctx.clone();
        let banner = banner.downgrade();
        let status = status.downgrade();
        move || {
            if let (Some(banner), Some(status)) = (banner.upgrade(), status.upgrade()) {
                update_banner(&c, &banner);
                status.set_label(&status_text(&c));
            }
        }
    };
    refresh_status();
    ctx.engine.connect_status(refresh_status);

    // Drop files anywhere on the window to import them.
    let drop = gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
    let c = ctx.clone();
    drop.connect_drop(move |_, value, _, _| {
        let Ok(list) = value.get::<gdk::FileList>() else { return false };
        let paths: Vec<PathBuf> = list.files().iter().filter_map(|f| f.path()).collect();
        import_paths(&c, paths);
        true
    });
    window.add_controller(drop);

    window
}
