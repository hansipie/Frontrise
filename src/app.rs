//! Application lifecycle, actions (also reachable over D-Bus through
//! `org.gtk.Actions`, which the GNOME extension uses) and command line.

use std::cell::{OnceCell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use adw::prelude::*;
use gtk::{gdk, gio, glib};

use crate::config::Source;
use crate::engine::{self, Condition, Engine};
use crate::state::State;
use crate::{autostart, power, shellext, ui, APP_ID};

pub struct Ctx {
    pub app: adw::Application,
    pub state: Rc<State>,
    pub engine: Rc<Engine>,
    pub window: RefCell<glib::WeakRef<adw::ApplicationWindow>>,
    pub stack: RefCell<glib::WeakRef<adw::ViewStack>>,
}

thread_local! {
    static CTX: OnceCell<Rc<Ctx>> = const { OnceCell::new() };
}

pub fn ctx() -> Rc<Ctx> {
    CTX.with(|c| c.get().expect("app started").clone())
}

const USAGE: &str = "\
Usage: frontrise [COMMAND]

Without a command, opens the Frontrise window.

Commands:
  --background         Start without opening the window (used at login)
  show [PAGE]          Open the window (PAGE: library, playlists, displays)
  set FILE             Import FILE and play it on every display
  backdrop://wallpaper/ID
                       Play the library wallpaper linked to that Backdrop ID
  pause | resume       Pause or resume all wallpapers
  toggle               Toggle pause
  next | previous      Skip to the next or previous playlist item
  quit                 Stop Frontrise
";

pub fn run() -> glib::ExitCode {
    let app = adw::Application::builder()
        .application_id(APP_ID)
        .flags(gio::ApplicationFlags::HANDLES_COMMAND_LINE)
        .build();
    app.connect_startup(startup);
    app.connect_activate(|_| ui::present(&ctx(), None));
    app.connect_command_line(|app, cl| command_line(app, cl));

    // Handle what must not start a background instance before registering.
    let args: Vec<String> = std::env::args().skip(1).collect();
    let command = args.first().map(String::as_str);
    match command {
        Some("--help" | "-h" | "help") => {
            print!("{USAGE}");
            return glib::ExitCode::SUCCESS;
        }
        None | Some("--background" | "show" | "set") => {}
        Some(link) if crate::config::parse_backdrop_link(link).is_some() => {}
        Some("pause" | "resume" | "toggle" | "next" | "previous" | "quit") => {
            // Control commands only make sense for a running instance.
            if app.register(gio::Cancellable::NONE).is_ok() && !app.is_remote() {
                eprintln!("frontrise: Frontrise is not running");
                return glib::ExitCode::FAILURE;
            }
        }
        Some(other) => {
            eprint!("frontrise: unknown command “{other}”\n\n{USAGE}");
            return glib::ExitCode::FAILURE;
        }
    }
    app.run()
}

fn startup(app: &adw::Application) {
    let provider = gtk::CssProvider::new();
    provider.load_from_string(include_str!("style.css"));
    if let Some(display) = gdk::Display::default() {
        gtk::style_context_add_provider_for_display(
            &display,
            &provider,
            gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
        );
    }

    let state = State::load();
    let engine = Engine::new(app, state.clone());
    let ctx = Rc::new(Ctx {
        app: app.clone(),
        state: state.clone(),
        engine: engine.clone(),
        window: RefCell::default(),
        stack: RefCell::default(),
    });
    CTX.with(|c| {
        let _ = c.set(ctx.clone());
    });

    add_actions(app, &ctx);
    power::watch(&engine);
    shellext::watch(&engine);
    if state.store.borrow().settings.autostart {
        autostart::set(true); // keep the path current
    }
    engine.queue_reconcile();

    // Keep running in the background when the window is closed.
    std::mem::forget(app.hold());
}

fn add_actions(app: &adw::Application, ctx: &Rc<Ctx>) {
    let paused = gio::SimpleAction::new_stateful("paused", None, &false.to_variant());
    let engine = ctx.engine.clone();
    paused.connect_change_state(move |action, value| {
        if let Some(value) = value {
            action.set_state(value);
            engine.set_condition(Condition::User, value.get::<bool>().unwrap_or(false));
        }
    });
    app.add_action(&paused);

    let fullscreen = gio::SimpleAction::new("fullscreen-active", Some(glib::VariantTy::BOOLEAN));
    let engine = ctx.engine.clone();
    fullscreen.connect_activate(move |_, value| {
        let on = value.and_then(|v| v.get::<bool>()).unwrap_or(false);
        engine.set_condition(Condition::Fullscreen, on);
    });
    app.add_action(&fullscreen);

    let simple = |name: &str, f: Box<dyn Fn()>| {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| f());
        app.add_action(&action);
    };
    let c = ctx.clone();
    simple("next", Box::new(move || c.engine.step_all(1)));
    let c = ctx.clone();
    simple("previous", Box::new(move || c.engine.step_all(-1)));
    let c = ctx.clone();
    simple("show", Box::new(move || ui::present(&c, None)));
    let c = ctx.clone();
    simple("preferences", Box::new(move || ui::prefs::present(&c)));
    let c = ctx.clone();
    simple("about", Box::new(move || ui::about(&c)));
    let c = ctx.clone();
    simple("quit", Box::new(move || quit(&c)));

    app.set_accels_for_action("app.quit", &["<Control>q"]);
    app.set_accels_for_action("app.preferences", &["<Control>comma"]);
    app.set_accels_for_action("win.import", &["<Control>o"]);
    app.set_accels_for_action("window.close", &["<Control>w"]);
}

pub fn set_user_paused(ctx: &Ctx, paused: bool) {
    ctx.app.change_action_state("paused", &paused.to_variant());
}

fn quit(ctx: &Ctx) {
    ctx.engine.shutdown();
    ctx.app.quit();
}

fn command_line(_app: &adw::Application, cl: &gio::ApplicationCommandLine) -> glib::ExitCode {
    let ctx = ctx();
    let args: Vec<String> = cl
        .arguments()
        .iter()
        .skip(1)
        .map(|a| a.to_string_lossy().into_owned())
        .collect();
    match args.first().map(String::as_str) {
        None => ui::present(&ctx, None),
        Some("--background") => {}
        Some("show") => ui::present(&ctx, args.get(1).map(String::as_str)),
        Some("pause") => set_user_paused(&ctx, true),
        Some("resume") => set_user_paused(&ctx, false),
        Some("toggle") => ctx.app.activate_action("paused", None),
        Some("next") => ctx.engine.step_all(1),
        Some("previous") => ctx.engine.step_all(-1),
        Some("quit") => quit(&ctx),
        Some("set") => {
            let Some(arg) = args.get(1) else {
                cl.printerr_literal("frontrise: set needs a file\n");
                return glib::ExitCode::FAILURE;
            };
            let mut path = PathBuf::from(arg);
            if path.is_relative() {
                if let Some(cwd) = cl.cwd() {
                    path = cwd.join(path);
                }
            }
            if !path.exists() {
                cl.printerr_literal(&format!("frontrise: {} does not exist\n", path.display()));
                return glib::ExitCode::FAILURE;
            }
            let connectors: Vec<String> = engine::monitors().into_iter().map(|m| m.connector).collect();
            let mut ok = false;
            ctx.state.modify(|s| {
                if let Some(id) = s.import(&path) {
                    s.apply_everywhere(&connectors, Source::Item(id));
                    ok = true;
                }
            });
            if !ok {
                cl.printerr_literal("frontrise: unsupported file type\n");
                return glib::ExitCode::FAILURE;
            }
        }
        Some("--help" | "-h" | "help") => cl.print_literal(USAGE),
        Some(link) if crate::config::parse_backdrop_link(link).is_some() => {
            ui::open_backdrop_link(&ctx, link);
        }
        Some(other) => {
            cl.printerr_literal(&format!("frontrise: unknown command “{other}”\n\n{USAGE}"));
            return glib::ExitCode::FAILURE;
        }
    }
    glib::ExitCode::SUCCESS
}
