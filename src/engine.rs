//! The wallpaper engine: decides what plays on each display, owns the media,
//! the playlists and the per-monitor renderer windows.
//!
//! One `Player` exists per *root* display (a display whose source is an item or
//! a playlist). Displays set to "Same as …" share their root's player, so they
//! show the very same media object, in sync, decoded once.
//!
//! Each connected display that shows something gets an undecorated window
//! titled `FrontriseWallpaper:<x>,<y>:<connector>`. The GNOME Shell extension
//! keeps these windows minimized and out of sight, and paints a live clone of
//! them inside GNOME's own background actors (desktop, Overview, lock screen).

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use gtk::prelude::*;
use gtk::{gdk, glib};

use crate::audio::Audio;
use crate::boomerang;
use crate::config::{Fit, Item, Kind, Playback, Source, Store};
use crate::scene::{self, Scene};
use crate::shellext::{ShellEvent, ShellStatus};
use crate::state::State;
use crate::WINDOW_TITLE_PREFIX;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Condition {
    /// The user pressed pause.
    User,
    Battery,
    PowerSaver,
    /// An application is fullscreen (reported by the GNOME extension).
    Fullscreen,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum PlayerSource {
    Item(String),
    Playlist(String),
}

pub enum Media {
    Video(gtk::MediaFile),
    Scene(Rc<Scene>),
    Missing,
}

/// What a player shows right now.
pub struct Content {
    /// Unique per Content ever created (pointer identity can be reused).
    serial: u64,
    pub item_id: String,
    pub media: Media,
    /// The video file being played (the original, or its boomerang clip).
    file: Option<PathBuf>,
    /// Restarts the video when it ends (see [`loop_video`]).
    loop_handler: Option<glib::SignalHandlerId>,
}

thread_local! {
    static NEXT_SERIAL: Cell<u64> = const { Cell::new(1) };
}

impl Content {
    fn new(store: &Store, item_id: &str) -> Rc<Self> {
        let mut file = None;
        let mut loop_handler = None;
        let media = match store.item(item_id) {
            Some(item) if item.is_available() => match item.kind {
                Kind::Video => {
                    let path = video_file(item);
                    let media = gtk::MediaFile::for_filename(&path);
                    media.set_muted(true);
                    loop_handler = Some(loop_video(&media));
                    file = Some(path);
                    Media::Video(media)
                }
                Kind::Scene => match item.scene_source() {
                    Ok(src) => Media::Scene(Scene::new(src)),
                    Err(e) => {
                        eprintln!("frontrise: {e}");
                        Media::Missing
                    }
                },
            },
            _ => Media::Missing,
        };
        let serial = NEXT_SERIAL.with(|n| n.replace(n.get() + 1));
        Rc::new(Self { serial, item_id: item_id.to_owned(), media, file, loop_handler })
    }

    /// A video whose playback mode changed, or whose boomerang clip just
    /// became ready, must be loaded again from the right file.
    fn is_stale(&self, store: &Store) -> bool {
        let Some(file) = &self.file else { return false };
        store.item(&self.item_id).filter(|i| i.is_available()).is_some_and(|i| video_file(i) != *file)
    }

    fn set_playing(&self, playing: bool) {
        match &self.media {
            Media::Video(m) if playing => m.play(),
            Media::Video(m) => m.pause(),
            Media::Scene(s) if playing => s.clock.play(),
            Media::Scene(s) => s.clock.pause(),
            Media::Missing => {}
        }
    }

    fn uses_audio(&self) -> bool {
        matches!(&self.media, Media::Scene(s) if s.uses_audio)
    }
}

/// The file to play for a video item. A boomerang is a pre-rendered clip
/// (forward then backward) that simply loops; until it is ready the original
/// video loops, and its rendering is started in the background.
fn video_file(item: &Item) -> PathBuf {
    let original = item.path.clone().unwrap_or_default();
    if item.playback == Playback::Boomerang {
        match boomerang::ready(&item.id, &original) {
            Some(clip) => return clip,
            None => boomerang::request(&item.id, &original),
        }
    }
    original
}

/// Plays `media` again from the start each time it ends.
///
/// GTK's own loop (`set_loop(true)`) hands the same file to GStreamer again
/// when the clip is about to finish, and that leaks one open file descriptor
/// per turn: a 5 s wallpaper piles up ~700 of them an hour, until the process
/// runs out. Seeking back to the start reuses the pipeline and opens nothing.
fn loop_video(media: &gtk::MediaFile) -> glib::SignalHandlerId {
    media.connect_ended_notify(|m| {
        if m.is_ended() {
            m.seek(0);
            m.play();
        }
    })
}

/// How long a replaced video stays alive, paused, before it is released.
const VIDEO_RETIRE_DELAY_MS: u64 = 1500;

impl Drop for Content {
    fn drop(&mut self) {
        if let Media::Video(media) = &self.media {
            // A video ending while it is retired must not start playing again.
            if let Some(handler) = self.loop_handler.take() {
                media.disconnect(handler);
            }
            retire_video(media.clone());
        }
    }
}

/// Releases a video that is no longer shown, without crashing or freezing.
///
/// Disposing a `GtkMediaFile` stops its GstPlay pipeline and joins its thread
/// on the main thread. Doing it while the clip plays races GStreamer's own
/// threads: when the stop meets the loop's "about-to-finish" the pipeline
/// deadlocks (the app freezes, wallpapers no longer change), otherwise GstPlay
/// can touch itself after being freed (segfault in `gst_play_main`). Pausing
/// first and releasing once the pipeline is idle avoids both.
fn retire_video(media: gtk::MediaFile) {
    media.pause();
    glib::timeout_add_local_once(std::time::Duration::from_millis(VIDEO_RETIRE_DELAY_MS), move || {
        drop(media);
    });
}

struct Player {
    source: PlayerSource,
    /// Playlist order (item ids), shuffled if requested.
    order: RefCell<Vec<String>>,
    pos: Cell<usize>,
    content: RefCell<Rc<Content>>,
    interval_minutes: Cell<u32>,
    shuffle: Cell<bool>,
    timer: RefCell<Option<glib::SourceId>>,
}

impl Player {
    fn new(source: PlayerSource, store: &Store) -> Rc<Self> {
        let (order, interval, shuffle) = match &source {
            PlayerSource::Item(id) => (vec![id.clone()], 0, false),
            PlayerSource::Playlist(id) => {
                let p = store.playlist(id).expect("playlist exists");
                (playable(store, &p.items, p.shuffle), p.interval_minutes, p.shuffle)
            }
        };
        let first = order.first().cloned().unwrap_or_default();
        Rc::new(Self {
            source,
            order: RefCell::new(order),
            pos: Cell::new(0),
            content: RefCell::new(Content::new(store, &first)),
            interval_minutes: Cell::new(interval),
            shuffle: Cell::new(shuffle),
            timer: RefCell::new(None),
        })
    }

    /// Picks up playlist edits; keeps the current item if it is still there.
    /// Returns true if the content changed.
    fn sync(&self, store: &Store) -> bool {
        let stale = self.content.borrow().is_stale(store);
        if stale {
            let id = self.content.borrow().item_id.clone();
            *self.content.borrow_mut() = Content::new(store, &id);
        }
        let PlayerSource::Playlist(id) = &self.source else { return stale };
        let Some(p) = store.playlist(id) else { return false };
        self.interval_minutes.set(p.interval_minutes);
        let wanted: Vec<String> = playable(store, &p.items, false);
        let current_set: Vec<String> = {
            let mut v = self.order.borrow().clone();
            v.sort();
            v
        };
        let mut wanted_sorted = wanted.clone();
        wanted_sorted.sort();
        if current_set == wanted_sorted && self.shuffle.get() == p.shuffle {
            return false;
        }
        self.shuffle.set(p.shuffle);
        let current = self.content.borrow().item_id.clone();
        let mut order = wanted;
        if p.shuffle {
            fastrand::shuffle(&mut order);
        }
        let pos = order.iter().position(|i| *i == current);
        *self.order.borrow_mut() = order;
        match pos {
            Some(pos) => {
                self.pos.set(pos);
                false
            }
            None => {
                self.pos.set(0);
                let first = self.order.borrow().first().cloned().unwrap_or_default();
                *self.content.borrow_mut() = Content::new(store, &first);
                true
            }
        }
    }

    fn step(&self, store: &Store, delta: isize) -> bool {
        let len = self.order.borrow().len();
        if len < 2 {
            return false;
        }
        let mut pos = self.pos.get() as isize + delta;
        if pos >= len as isize {
            pos = 0;
            if self.shuffle.get() {
                let last = self.content.borrow().item_id.clone();
                let mut order = self.order.borrow_mut();
                fastrand::shuffle(&mut order);
                // Avoid playing the same item twice in a row.
                if order.first() == Some(&last) {
                    order.swap(0, len - 1);
                }
            }
        } else if pos < 0 {
            pos = len as isize - 1;
        }
        self.pos.set(pos as usize);
        let id = self.order.borrow()[pos as usize].clone();
        *self.content.borrow_mut() = Content::new(store, &id);
        true
    }
}

fn playable(store: &Store, items: &[String], shuffle: bool) -> Vec<String> {
    let mut v: Vec<String> = items
        .iter()
        .filter(|id| store.item(id).is_some_and(|i| i.is_available()))
        .cloned()
        .collect();
    if shuffle {
        fastrand::shuffle(&mut v);
    }
    v
}

/// A connected monitor.
#[derive(Clone, Debug)]
pub struct MonitorInfo {
    pub monitor: gdk::Monitor,
    pub connector: String,
    pub label: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

pub fn monitors() -> Vec<MonitorInfo> {
    let Some(display) = gdk::Display::default() else { return vec![] };
    let model = display.monitors();
    (0..model.n_items())
        .filter_map(|i| model.item(i).and_downcast::<gdk::Monitor>())
        .filter_map(|m| {
            let connector = m.connector()?.to_string();
            let g = m.geometry();
            let label = m
                .description()
                .map(|d| d.to_string())
                .or_else(|| m.model().map(|d| d.to_string()))
                .unwrap_or_else(|| connector.clone());
            Some(MonitorInfo {
                monitor: m.clone(),
                connector,
                label,
                x: g.x(),
                y: g.y(),
                width: g.width(),
                height: g.height(),
            })
        })
        .collect()
}

/// A wall window must be rebuilt when its monitor moved/resized, or when GDK
/// handed out a new monitor object for the same connector (mode switches).
fn wall_is_stale(old_geometry: (i32, i32, i32, i32), same_monitor: bool, new_geometry: (i32, i32, i32, i32)) -> bool {
    !same_monitor || old_geometry != new_geometry
}

/// Follows "Same as" links to the display that owns the content.
fn resolve(store: &Store, connector: &str) -> Option<(String, PlayerSource)> {
    let mut current = connector.to_owned();
    for _ in 0..16 {
        match store.displays.get(&current).map(|d| &d.source) {
            None | Some(Source::Off) => return None,
            Some(Source::Item(id)) => {
                return store.item(id).map(|_| (current, PlayerSource::Item(id.clone())));
            }
            Some(Source::Playlist(id)) => {
                return store.playlist(id).map(|_| (current, PlayerSource::Playlist(id.clone())));
            }
            Some(Source::SameAs(other)) => current = other.clone(),
        }
    }
    None // cycle
}

struct Wall {
    window: gtk::ApplicationWindow,
    /// The GDK monitor object the window was made fullscreen on. A display
    /// mode switch can replace it with a new object that has the same
    /// connector and geometry; the window must then be recreated.
    monitor: gdk::Monitor,
    root: String,
    geometry: (i32, i32, i32, i32),
    /// Serial of the content and fit currently shown.
    shown: RefCell<Option<(u64, Fit)>>,
}

impl Wall {
    fn show(&self, content: &Rc<Content>, fit: Fit, audio: &Rc<Audio>) {
        let key = (content.serial, fit);
        if *self.shown.borrow() == Some(key) {
            return;
        }
        let child: gtk::Widget = match &content.media {
            Media::Video(media) => {
                let picture = gtk::Picture::for_paintable(media);
                picture.set_content_fit(match fit {
                    Fit::Cover => gtk::ContentFit::Cover,
                    Fit::Contain => gtk::ContentFit::Contain,
                });
                picture.upcast()
            }
            Media::Scene(s) => scene::widget(s.clone(), audio.clone()).upcast(),
            Media::Missing => gtk::Box::new(gtk::Orientation::Vertical, 0).upcast(),
        };
        self.window.set_child(Some(&child));
        *self.shown.borrow_mut() = Some(key);
    }
}

type ShellListener = Rc<dyn Fn(&ShellEvent)>;

pub struct Engine {
    app: gtk::Application,
    state: Rc<State>,
    audio: Rc<Audio>,
    players: RefCell<HashMap<String, Rc<Player>>>,
    walls: RefCell<HashMap<String, Wall>>,
    conditions: RefCell<HashMap<Condition, bool>>,
    shell_status: Cell<ShellStatus>,
    shell_proxy: RefCell<Option<gio::DBusProxy>>,
    /// What the automatic repair of the extension did, or why it failed.
    shell_note: RefCell<Option<String>>,
    /// The extension needs a repair the user has not allowed yet.
    repair_wanted: Cell<bool>,
    shell_listeners: RefCell<Vec<ShellListener>>,
    reconcile_queued: Cell<bool>,
    /// What the system wallpaper still was last made from: item, width, height.
    last_still: RefCell<Option<(String, i32, i32)>>,
    status_listeners: RefCell<Vec<Rc<dyn Fn()>>>,
    keep_alive: RefCell<Vec<glib::Object>>,
    weak_self: Weak<Engine>,
}

impl Engine {
    pub fn new(app: &impl IsA<gtk::Application>, state: Rc<State>) -> Rc<Self> {
        let engine = Rc::new_cyclic(|weak| Self {
            app: app.clone().upcast(),
            state: state.clone(),
            audio: Audio::new(),
            players: RefCell::default(),
            walls: RefCell::default(),
            conditions: RefCell::default(),
            shell_status: Cell::new(ShellStatus::Unknown),
            shell_proxy: RefCell::default(),
            shell_note: RefCell::default(),
            repair_wanted: Cell::new(false),
            shell_listeners: RefCell::default(),
            reconcile_queued: Cell::new(false),
            last_still: RefCell::new(None),
            status_listeners: RefCell::default(),
            keep_alive: RefCell::default(),
            weak_self: weak.clone(),
        });
        let weak = Rc::downgrade(&engine);
        state.set_engine_hook(move || {
            if let Some(e) = weak.upgrade() {
                e.queue_reconcile();
            }
        });
        if let Some(display) = gdk::Display::default() {
            let weak = Rc::downgrade(&engine);
            let model = display.monitors();
            let watched: Rc<RefCell<Vec<gdk::Monitor>>> = Rc::default();
            let watch = {
                let weak = weak.clone();
                let watched = watched.clone();
                move |model: &gtk::gio::ListModel| {
                    // Mode switches move/resize/rescale monitors without
                    // adding or removing them: watch each monitor too.
                    for i in 0..model.n_items() {
                        let Some(m) = model.item(i).and_downcast::<gdk::Monitor>() else { continue };
                        if watched.borrow().contains(&m) {
                            continue;
                        }
                        for prop in ["notify::geometry", "notify::scale-factor"] {
                            let weak = weak.clone();
                            m.connect_local(prop, false, move |_| {
                                if let Some(e) = weak.upgrade() {
                                    e.queue_reconcile();
                                }
                                None
                            });
                        }
                        watched.borrow_mut().push(m);
                    }
                }
            };
            watch(model.upcast_ref());
            model.connect_items_changed(move |model, _, _, _| {
                watch(model.upcast_ref());
                if let Some(e) = weak.upgrade() {
                    e.queue_reconcile();
                }
            });
        }
        engine
    }

    pub fn keep_alive(&self, object: glib::Object) {
        self.keep_alive.borrow_mut().push(object);
    }

    pub fn connect_status(&self, f: impl Fn() + 'static) {
        self.status_listeners.borrow_mut().push(Rc::new(f));
    }

    fn notify_status(&self) {
        let listeners: Vec<Rc<dyn Fn()>> = self.status_listeners.borrow().clone();
        for l in listeners {
            l();
        }
    }

    pub fn shell_status(&self) -> ShellStatus {
        self.shell_status.get()
    }

    pub fn shell_proxy(&self) -> Option<gio::DBusProxy> {
        self.shell_proxy.borrow().clone()
    }

    pub fn set_shell_proxy(&self, proxy: gio::DBusProxy) {
        *self.shell_proxy.borrow_mut() = Some(proxy);
    }

    pub fn shell_note(&self) -> Option<String> {
        self.shell_note.borrow().clone()
    }

    pub fn set_shell_note(&self, note: Option<String>) {
        if self.shell_note.replace(note.clone()) != note {
            self.notify_status();
        }
    }

    pub fn state(&self) -> &Rc<State> {
        &self.state
    }

    pub fn repair_wanted(&self) -> bool {
        self.repair_wanted.get()
    }

    pub fn set_repair_wanted(&self, wanted: bool) {
        if self.repair_wanted.replace(wanted) != wanted {
            self.notify_status();
        }
    }

    pub fn connect_shell_event(&self, f: impl Fn(&ShellEvent) + 'static) {
        self.shell_listeners.borrow_mut().push(Rc::new(f));
    }

    pub fn emit_shell_event(&self, event: ShellEvent) {
        let listeners: Vec<ShellListener> = self.shell_listeners.borrow().clone();
        for l in listeners {
            l(&event);
        }
    }

    pub fn set_shell_status(&self, status: ShellStatus) {
        if self.shell_status.replace(status) != status {
            self.queue_reconcile();
            self.notify_status();
        }
    }

    pub fn set_condition(&self, condition: Condition, on: bool) {
        let old = self.conditions.borrow_mut().insert(condition, on).unwrap_or(false);
        if old != on {
            self.apply_pause();
            self.notify_status();
        }
    }

    pub fn condition(&self, condition: Condition) -> bool {
        self.conditions.borrow().get(&condition).copied().unwrap_or(false)
    }

    /// Reasons the wallpapers are paused right now (empty when playing).
    pub fn pause_reasons(&self) -> Vec<&'static str> {
        let s = self.state.store.borrow().settings.clone();
        let mut reasons = vec![];
        if self.condition(Condition::User) {
            reasons.push("paused");
        }
        if s.pause_on_battery && self.condition(Condition::Battery) {
            reasons.push("on battery");
        }
        if s.pause_on_power_saver && self.condition(Condition::PowerSaver) {
            reasons.push("power saver");
        }
        if s.pause_when_fullscreen && self.condition(Condition::Fullscreen) {
            reasons.push("fullscreen app");
        }
        reasons
    }

    fn paused(&self) -> bool {
        !self.pause_reasons().is_empty()
    }

    /// Number of displays currently showing a Frontrise wallpaper.
    pub fn active_displays(&self) -> usize {
        self.walls.borrow().len()
    }

    /// Title of what plays on `connector`, if anything.
    pub fn now_playing(&self, connector: &str) -> Option<String> {
        let root = self.walls.borrow().get(connector)?.root.clone();
        let player = self.players.borrow().get(&root)?.clone();
        let id = player.content.borrow().item_id.clone();
        self.state.store.borrow().item(&id).map(|i| i.title.clone())
    }

    pub fn queue_reconcile(&self) {
        if self.reconcile_queued.replace(true) {
            return;
        }
        let weak = self.weak_self.clone();
        glib::idle_add_local_once(move || {
            if let Some(e) = weak.upgrade() {
                e.reconcile_queued.set(false);
                e.reconcile();
            }
        });
    }

    /// Brings players and windows in line with the configuration and monitors.
    pub fn reconcile(&self) {
        let monitors = monitors();
        {
            // Remember monitor names for the "disconnected displays" list.
            let mut store = self.state.store.borrow_mut();
            let mut changed = false;
            for m in &monitors {
                let d = store.display_mut(&m.connector);
                if d.label != m.label {
                    d.label = m.label.clone();
                    changed = true;
                }
            }
            if changed {
                store.save();
            }
        }

        let store = self.state.store.borrow();
        let routes: Vec<(MonitorInfo, Option<(String, PlayerSource)>)> =
            monitors.into_iter().map(|m| { let r = resolve(&store, &m.connector); (m, r) }).collect();

        // Players: one per root display.
        let mut wanted: HashMap<String, PlayerSource> = HashMap::new();
        for (_, route) in &routes {
            if let Some((root, source)) = route {
                wanted.insert(root.clone(), source.clone());
            }
        }
        {
            let mut players = self.players.borrow_mut();
            players.retain(|root, p| {
                let keep = wanted.get(root) == Some(&p.source);
                if !keep {
                    if let Some(t) = p.timer.borrow_mut().take() {
                        t.remove();
                    }
                    p.content.borrow().set_playing(false);
                }
                keep
            });
            for (root, source) in &wanted {
                match players.get(root) {
                    Some(p) => {
                        p.sync(&store);
                    }
                    None => {
                        players.insert(root.clone(), Player::new(source.clone(), &store));
                    }
                }
            }
        }

        // Windows: only while the GNOME extension can hide and place them.
        let shell_ok = self.shell_status.get() == ShellStatus::Active;
        let mut walls = self.walls.borrow_mut();
        let mut keep = vec![];
        for (m, route) in &routes {
            let Some((root, _)) = route else { continue };
            if !shell_ok {
                continue;
            }
            let geometry = (m.x, m.y, m.width, m.height);
            let recreate = walls
                .get(&m.connector)
                .is_none_or(|w| wall_is_stale(w.geometry, w.monitor == m.monitor, geometry));
            if recreate {
                if let Some(old) = walls.remove(&m.connector) {
                    old.window.destroy();
                }
                walls.insert(m.connector.clone(), self.create_wall(m, root));
            }
            let wall = walls.get_mut(&m.connector).unwrap();
            wall.root = root.clone();
            let fit = store.displays.get(&m.connector).map(|d| d.fit).unwrap_or_default();
            if let Some(player) = self.players.borrow().get(root) {
                wall.show(&player.content.borrow(), fit, &self.audio);
            }
            keep.push(m.connector.clone());
        }
        walls.retain(|connector, wall| {
            let k = keep.contains(connector);
            if !k {
                wall.window.destroy();
            }
            k
        });
        drop(walls);
        drop(store);

        self.apply_pause();
        self.notify_status();
        self.sync_still();
    }

    fn create_wall(&self, m: &MonitorInfo, root: &str) -> Wall {
        let window = gtk::ApplicationWindow::builder()
            .application(&self.app)
            .title(format!("{WINDOW_TITLE_PREFIX}{},{}:{}", m.x, m.y, m.connector))
            .decorated(false)
            .default_width(m.width)
            .default_height(m.height)
            .build();
        window.add_css_class("frontrise-wallpaper");
        // Fullscreen gives the exact monitor size and output (scale). The
        // extension keeps the window minimized, so it never covers anything.
        window.fullscreen_on_monitor(&m.monitor);
        window.present();
        Wall {
            window,
            monitor: m.monitor.clone(),
            root: root.to_owned(),
            geometry: (m.x, m.y, m.width, m.height),
            shown: RefCell::new(None),
        }
    }

    /// Plays or pauses everything, (re)arms playlist timers, starts or stops
    /// sound analysis.
    fn apply_pause(&self) {
        let playing = !self.paused();
        let shown_roots: Vec<String> = self.walls.borrow().values().map(|w| w.root.clone()).collect();
        let mut needs_audio = false;
        for (root, player) in self.players.borrow().iter() {
            let visible = shown_roots.contains(root);
            let content = player.content.borrow().clone();
            content.set_playing(playing && visible);
            needs_audio |= visible && content.uses_audio();

            let has_timer = player.timer.borrow().is_some();
            let wants_timer = playing && visible && player.order.borrow().len() > 1;
            if wants_timer && !has_timer {
                self.arm_timer(root, player);
            } else if !wants_timer && has_timer {
                if let Some(t) = player.timer.borrow_mut().take() {
                    t.remove();
                }
            }
        }
        let react = self.state.store.borrow().settings.react_to_sound;
        self.audio.set_active(playing && needs_audio && react);
    }

    fn arm_timer(&self, root: &str, player: &Rc<Player>) {
        let minutes = player.interval_minutes.get().max(1);
        let weak = self.weak_self.clone();
        let root = root.to_owned();
        let id = glib::timeout_add_seconds_local(minutes * 60, move || {
            if let Some(engine) = weak.upgrade() {
                engine.step_root(&root, 1);
            }
            glib::ControlFlow::Continue
        });
        *player.timer.borrow_mut() = Some(id);
    }

    fn step_root(&self, root: &str, delta: isize) {
        let Some(player) = self.players.borrow().get(root).cloned() else { return };
        let changed = {
            let store = self.state.store.borrow();
            player.step(&store, delta)
        };
        if changed {
            // Restart the interval so a manual skip gets a full turn.
            if let Some(t) = player.timer.borrow_mut().take() {
                t.remove();
            }
            self.refresh_walls();
        }
    }

    /// Next/previous item on every playlist on screen.
    pub fn step_all(&self, delta: isize) {
        let roots: Vec<String> = self.players.borrow().keys().cloned().collect();
        for root in roots {
            self.step_root(&root, delta);
        }
    }

    fn refresh_walls(&self) {
        let store = self.state.store.borrow();
        for (connector, wall) in self.walls.borrow().iter() {
            let fit = store.displays.get(connector).map(|d| d.fit).unwrap_or_default();
            if let Some(player) = self.players.borrow().get(&wall.root) {
                wall.show(&player.content.borrow(), fit, &self.audio);
            }
        }
        drop(store);
        self.apply_pause();
        self.notify_status();
        self.sync_still();
    }

    /// Settings changed (pause rules, sound): re-evaluate without rebuilding.
    pub fn settings_changed(&self) {
        self.apply_pause();
        self.notify_status();
        self.sync_still();
    }

    /// Makes GNOME's own wallpaper a still of what plays on the first display
    /// (GNOME has one system wallpaper for all displays), or gives the user's
    /// previous wallpaper back when the option is off.
    fn sync_still(&self) {
        const STILL_TIME: f64 = 8.0;
        let (enabled, original) = {
            let store = self.state.store.borrow();
            (store.settings.sync_system_wallpaper, store.original_wallpaper.clone())
        };
        if !enabled {
            self.last_still.replace(None);
            if let Some(original) = original {
                crate::still::restore(&original);
                self.state.modify_quiet(|s| s.original_wallpaper = None);
            }
            return;
        }

        // The first display that shows something, by its native pixel size.
        let target = monitors().into_iter().find_map(|m| {
            let root = self.walls.borrow().get(&m.connector)?.root.clone();
            let id = self.players.borrow().get(&root)?.content.borrow().item_id.clone();
            let scale = m.monitor.scale_factor().max(1);
            Some((id, m.width * scale, m.height * scale))
        });
        let Some((id, width, height)) = target else { return };
        let key = (id.clone(), width, height);
        if self.last_still.borrow().as_ref() == Some(&key) {
            return;
        }
        let Some(item) = self.state.store.borrow().item(&id).cloned() else { return };
        if !item.is_available() {
            return;
        }
        self.last_still.replace(Some(key.clone()));

        let out = crate::still::next_path();
        let target = out.clone();
        let weak = self.weak_self.clone();
        let finish = move |result: Result<(), String>| {
            let Some(engine) = weak.upgrade() else { return };
            // Superseded while it was being made: the newer request applies.
            if engine.last_still.borrow().as_ref() != Some(&key) {
                return;
            }
            let applied = result.and_then(|()| {
                // Keep the user's own wallpaper, once, before replacing it.
                if engine.state.store.borrow().original_wallpaper.is_none() {
                    if let Some(original) = crate::still::current() {
                        engine.state.modify_quiet(|s| s.original_wallpaper = Some(original));
                    }
                }
                crate::still::apply(&out)
            });
            if let Err(e) = applied {
                eprintln!("frontrise: system wallpaper not updated: {e}");
                engine.last_still.replace(None);
            }
        };

        match item.kind {
            Kind::Scene => {
                let result = item.scene_source().and_then(|src| {
                    crate::thumbs::generate_scene_sized(&src, STILL_TIME, &target, width, height)
                });
                finish(result);
            }
            Kind::Video => {
                let Some(video) = item.path.clone() else { return };
                glib::spawn_future_local(async move {
                    let result = gio::spawn_blocking(move || {
                        crate::thumbs::generate_sized(&video, &target, width)
                    })
                    .await
                    .unwrap_or_else(|_| Err("worker panicked".to_owned()));
                    finish(result);
                });
            }
        }
    }

    pub fn shutdown(&self) {
        for (_, wall) in self.walls.borrow_mut().drain() {
            wall.window.destroy();
        }
        for (_, player) in self.players.borrow_mut().drain() {
            if let Some(t) = player.timer.borrow_mut().take() {
                t.remove();
            }
            player.content.borrow().set_playing(false);
        }
        self.audio.set_active(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::DisplayConfig;

    #[test]
    fn wall_rebuilt_when_monitor_object_replaced_even_if_geometry_same() {
        let g = (0, 0, 1920, 1080);
        assert!(!wall_is_stale(g, true, g));
        assert!(wall_is_stale(g, false, g)); // regression: mode switch
        assert!(wall_is_stale(g, true, (1920, 0, 1920, 1080)));
    }

    #[test]
    fn assignments_survive_a_monitor_disappearing() {
        let mut store = Store::default();
        store.display_mut("DP-1").source = Source::Off;
        store.display_mut("HDMI-1").source = Source::SameAs("DP-1".into());
        // Reconcile never prunes displays absent from the monitor list.
        assert!(store.displays.contains_key("HDMI-1"));
        assert_eq!(store.displays["HDMI-1"].source, Source::SameAs("DP-1".into()));
        let _ = DisplayConfig::default();
        assert!(resolve(&store, "HDMI-1").is_none());
    }
}
