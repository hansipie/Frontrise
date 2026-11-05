// Frontrise GNOME Shell extension.
//
// The Frontrise app renders each display's wallpaper in an undecorated window
// titled "FrontriseWallpaper:<x>,<y>:<connector>". This extension:
//  - keeps those renderer windows minimized, at the bottom and out of every
//    window list (Alt+Tab, Overview, dash, running apps);
//  - paints a live Clutter.Clone of each renderer inside GNOME's own
//    background actors, so the wallpaper shows on the desktop, in the
//    Overview and (optionally) on the lock screen;
//  - tells the app when a fullscreen window is open (auto-pause);
//  - adds a top bar menu with quick controls.
// The app and the extension talk over D-Bus through org.gtk.Actions.

import Clutter from 'gi://Clutter';
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import GObject from 'gi://GObject';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import St from 'gi://St';

import {Extension, InjectionManager} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';
import * as Background from 'resource:///org/gnome/shell/ui/background.js';
import * as PanelMenu from 'resource:///org/gnome/shell/ui/panelMenu.js';
import * as PopupMenu from 'resource:///org/gnome/shell/ui/popupMenu.js';
import {Workspace} from 'resource:///org/gnome/shell/ui/workspace.js';
import {WorkspaceThumbnail} from 'resource:///org/gnome/shell/ui/workspaceThumbnail.js';

const TITLE_RE = /^FrontriseWallpaper:(-?\d+),(-?\d+)(?::|$)/;
const APP_BUS_NAME = 'fr.ansicode.frontrise';
const APP_OBJECT_PATH = '/fr/ansicode/frontrise';
const APP_DESKTOP_ID = 'fr.ansicode.frontrise.desktop';
const FADE_MS = 400;

function isRenderer(window) {
    const title = window?.get_title?.();
    return typeof title === 'string' && TITLE_RE.test(title);
}

function rendererOrigin(window) {
    const m = TITLE_RE.exec(window.get_title() ?? '');
    return m ? [parseInt(m[1], 10), parseInt(m[2], 10)] : null;
}

function monitorIndexAt([x, y]) {
    return Main.layoutManager.monitors.findIndex(m => m.x === x && m.y === y);
}

// ---------------------------------------------------------------------------
// Settings shared with the app ($XDG_CONFIG_HOME/frontrise/config.json).

class Settings {
    constructor(onChange) {
        this._onChange = onChange;
        this.lockScreen = true;
        this.panelIcon = true;
        this._file = Gio.File.new_for_path(
            GLib.build_filenamev([GLib.get_user_config_dir(), 'frontrise', 'config.json']));
        this._read();
        try {
            this._monitor = this._file.monitor_file(Gio.FileMonitorFlags.WATCH_MOVES, null);
            this._monitor.connect('changed', () => {
                this._read();
                this._onChange();
            });
        } catch (e) {
            console.warn(`Frontrise: cannot watch settings: ${e}`);
        }
    }

    _read() {
        try {
            const [ok, bytes] = this._file.load_contents(null);
            if (!ok)
                return;
            const settings = JSON.parse(new TextDecoder().decode(bytes)).settings ?? {};
            this.lockScreen = settings.lock_screen ?? true;
            this.panelIcon = settings.panel_icon ?? true;
        } catch (_e) {
            // No config yet: keep defaults.
        }
    }

    destroy() {
        this._monitor?.cancel();
        this._monitor = null;
    }
}

// ---------------------------------------------------------------------------
// Renderer windows: keep them minimized, at the bottom, on their monitor.

class RendererKeeper {
    constructor(onChange) {
        this._onChange = onChange;
        this._windows = new Map(); // Meta.Window -> signal ids
    }

    enable() {
        this._mapId = global.window_manager.connect_after('map',
            (_wm, actor) => this._consider(actor.meta_window));
        for (const actor of global.get_window_actors())
            this._consider(actor.meta_window);
    }

    disable() {
        global.window_manager.disconnect(this._mapId);
        for (const window of [...this._windows.keys()])
            this._release(window);
    }

    _consider(window) {
        if (!window || this._windows.has(window) || !isRenderer(window))
            return;
        const ids = [
            window.connect('notify::minimized', () => {
                if (!window.minimized)
                    window.minimize();
            }),
            window.connect('raised', () => window.lower()),
            window.connect('position-changed', () => this._place(window)),
            window.connect('unmanaged', () => {
                this._windows.delete(window);
                this._onChange();
            }),
        ];
        this._windows.set(window, ids);
        // The clone paints the window: its own actor never needs to be seen.
        const actor = window.get_compositor_private();
        if (actor)
            actor.opacity = 0;
        this._place(window);
        window.lower();
        window.minimize();
        this._onChange();
        // At login the window reaches its fullscreen size a bit later: lay the
        // clones out again once it has settled.
        for (const delay of [500, 1500, 4000]) {
            GLib.timeout_add(GLib.PRIORITY_DEFAULT, delay, () => {
                if (this._windows.has(window))
                    this._onChange();
                return GLib.SOURCE_REMOVE;
            });
        }
    }

    _place(window) {
        const origin = rendererOrigin(window);
        if (!origin || window.is_fullscreen())
            return;
        const rect = window.get_frame_rect();
        if (rect.x !== origin[0] || rect.y !== origin[1])
            window.move_frame(true, origin[0], origin[1]);
    }

    _release(window) {
        for (const id of this._windows.get(window) ?? [])
            window.disconnect(id);
        this._windows.delete(window);
        const actor = window.get_compositor_private();
        if (actor)
            actor.opacity = 255;
    }

    rendererFor(monitorIndex) {
        for (const window of this._windows.keys()) {
            const origin = rendererOrigin(window);
            if (origin && monitorIndexAt(origin) === monitorIndex)
                return window.get_compositor_private();
        }
        return null;
    }
}

// ---------------------------------------------------------------------------
// Live layer: a clone of the renderer inside one GNOME background actor.

class LiveLayer {
    constructor(backgroundActor, keeper) {
        this._bg = backgroundActor;
        this._keeper = keeper;
        this._clone = null;
        this._sourceIds = [];
        this.actor = new Clutter.Actor({
            clip_to_allocation: true,
            reactive: false,
            opacity: 0,
        });
        this.actor.add_constraint(new Clutter.BindConstraint({
            source: backgroundActor,
            coordinate: Clutter.BindCoordinate.SIZE,
        }));
        backgroundActor.add_child(this.actor);
        this._bgIds = [
            backgroundActor.connect('notify::width', () => this._layout()),
            backgroundActor.connect('notify::height', () => this._layout()),
        ];
        this._destroyId = backgroundActor.connect('destroy', () => this.destroy());
        this.refresh();
    }

    refresh() {
        if (!this.actor)
            return;
        const source = this._keeper.rendererFor(this._bg.monitor);
        if (source === (this._clone?.source ?? null)) {
            // Same renderer: its size may have settled since the last layout
            // (login race), so always recompute.
            this._layout();
            return;
        }
        this._dropClone();
        if (!source) {
            this.actor.opacity = 0;
            return;
        }
        this._clone = new Clutter.Clone({source});
        this.actor.add_child(this._clone);
        this._sourceIds = [
            source.connect('notify::width', () => this._layout()),
            source.connect('notify::height', () => this._layout()),
            source.connect('destroy', () => {
                this._sourceIds = [];
                this._dropClone(true);
                this.actor?.set_opacity(0);
            }),
        ];
        this._layout();
        this.actor.remove_all_transitions();
        this.actor.ease({
            opacity: 255,
            duration: FADE_MS,
            mode: Clutter.AnimationMode.EASE_OUT_QUAD,
        });
    }

    // Cover the background: scale uniformly, centre, crop the overflow.
    _layout() {
        const clone = this._clone;
        if (!clone || !clone.source)
            return;
        const W = this._bg.width, H = this._bg.height;
        const w = clone.source.width, h = clone.source.height;
        if (!W || !H || !w || !h)
            return;
        const s = Math.max(W / w, H / h);
        clone.set_size(w * s, h * s);
        clone.set_position(Math.round((W - w * s) / 2), Math.round((H - h * s) / 2));
    }

    _dropClone(sourceGone = false) {
        if (!this._clone)
            return;
        if (!sourceGone) {
            for (const id of this._sourceIds)
                this._clone.source?.disconnect(id);
        }
        this._sourceIds = [];
        this._clone.destroy();
        this._clone = null;
    }

    destroy() {
        if (!this.actor)
            return;
        this._dropClone();
        for (const id of this._bgIds)
            this._bg.disconnect(id);
        this._bg.disconnect(this._destroyId);
        this.actor.destroy();
        this.actor = null;
        this.onDestroy?.();
    }
}

// ---------------------------------------------------------------------------
// Talking to the app (org.gtk.Actions on its bus name).

class AppLink {
    constructor(onChange) {
        this._onChange = onChange;
        this.running = false;
        this.paused = false;
        this._watchId = Gio.bus_watch_name(Gio.BusType.SESSION, APP_BUS_NAME,
            Gio.BusNameWatcherFlags.NONE,
            () => {
                this.running = true;
                this._refreshPaused();
                this._onChange();
                this.onAppeared?.();
            },
            () => {
                this.running = false;
                this.paused = false;
                this._onChange();
            });
        this._signalId = Gio.DBus.session.signal_subscribe(APP_BUS_NAME,
            'org.gtk.Actions', 'Changed', APP_OBJECT_PATH, null,
            Gio.DBusSignalFlags.NONE,
            (_conn, _sender, _path, _iface, _signal, params) => {
                const [, , stateChanges] = params.deepUnpack();
                if ('paused' in stateChanges) {
                    this.paused = stateChanges.paused.unpack();
                    this._onChange();
                }
            });
    }

    _call(method, params, callback) {
        Gio.DBus.session.call(APP_BUS_NAME, APP_OBJECT_PATH, 'org.gtk.Actions',
            method, params, null, Gio.DBusCallFlags.NONE, -1, null,
            (conn, res) => {
                try {
                    const result = conn.call_finish(res);
                    callback?.(result);
                } catch (e) {
                    console.debug(`Frontrise: ${method} failed: ${e.message}`);
                }
            });
    }

    activate(name, parameter = null) {
        this._call('Activate',
            new GLib.Variant('(sava{sv})', [name, parameter ? [parameter] : [], {}]));
    }

    setState(name, value) {
        this._call('SetState', new GLib.Variant('(sva{sv})', [name, value, {}]));
    }

    _refreshPaused() {
        this._call('Describe', new GLib.Variant('(s)', ['paused']), result => {
            const [[, , state]] = result.deepUnpack();
            if (state.length) {
                this.paused = state[0].unpack();
                this._onChange();
            }
        });
    }

    launch() {
        try {
            const info = Gio.DesktopAppInfo.new(APP_DESKTOP_ID);
            if (info) {
                info.launch([], global.create_app_launch_context(0, -1));
                return;
            }
        } catch (e) {
            console.debug(`Frontrise: desktop launch failed: ${e.message}`);
        }
        GLib.spawn_command_line_async('frontrise');
    }

    destroy() {
        Gio.bus_unwatch_name(this._watchId);
        Gio.DBus.session.signal_unsubscribe(this._signalId);
    }
}

// ---------------------------------------------------------------------------
// Top bar menu.

const Indicator = GObject.registerClass(
class FrontriseIndicator extends PanelMenu.Button {
    _init(extension, link) {
        super._init(0.0, 'Frontrise');
        this._link = link;
        this.add_child(new St.Icon({
            gicon: Gio.icon_new_for_string(`${extension.path}/icons/frontrise-symbolic.svg`),
            style_class: 'system-status-icon',
        }));

        this._status = new PopupMenu.PopupMenuItem('', {reactive: false});
        this.menu.addMenuItem(this._status);
        this._pause = new PopupMenu.PopupSwitchMenuItem('Pause wallpapers', false);
        this._pause.connect('toggled', (_item, state) =>
            this._link.setState('paused', GLib.Variant.new_boolean(state)));
        this.menu.addMenuItem(this._pause);
        this._next = this.menu.addAction('Next wallpaper', () => this._link.activate('next'));
        this.menu.addMenuItem(new PopupMenu.PopupSeparatorMenuItem());
        this._open = this.menu.addAction('Open Frontrise', () => {
            if (this._link.running)
                this._link.activate('show');
            else
                this._link.launch();
        });
        this._quit = this.menu.addAction('Quit Frontrise', () => this._link.activate('quit'));
        this.sync();
    }

    sync() {
        const {running, paused} = this._link;
        this._status.label.text = !running ? 'Frontrise is not running'
            : paused ? 'Wallpapers paused' : 'Wallpapers playing';
        this._pause.visible = running;
        this._pause.setToggleState(paused);
        this._next.visible = running;
        this._quit.visible = running;
        this._open.label.text = running ? 'Open Frontrise' : 'Start Frontrise';
    }
});

// ---------------------------------------------------------------------------

export default class FrontriseExtension extends Extension {
    enable() {
        this._layers = new Set();
        this._settings = new Settings(() => this._syncIndicator());
        this._keeper = new RendererKeeper(() => this._refreshLayers());
        this._keeper.enable();
        // Display mode switches (mirror/extended/single) change the monitor
        // indices: re-match renderers to backgrounds, now and once the app has
        // rebuilt its windows for the new layout.
        this._monitorsId = Main.layoutManager.connect('monitors-changed', () => {
            this._refreshLayers();
            this._monitorsTimeout = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 700, () => {
                this._monitorsTimeout = 0;
                this._refreshLayers();
                return GLib.SOURCE_REMOVE;
            });
        });

        this._injections = new InjectionManager();
        this._injectBackgrounds();
        this._hideRenderers();
        this._patchFullscreen();
        Main.layoutManager._updateBackgrounds?.();

        this._link = new AppLink(() => this._indicator?.sync());
        this._link.onAppeared = () => this._reportFullscreen(true);
        this._fullscreenId = global.display.connect('in-fullscreen-changed',
            () => this._reportFullscreen());
        this._sessionId = Main.sessionMode.connect('updated', () => this._syncIndicator());
        this._syncIndicator();
    }

    disable() {
        global.display.disconnect(this._fullscreenId);
        Main.sessionMode.disconnect(this._sessionId);
        this._indicator?.destroy();
        this._indicator = null;
        this._link.destroy();
        this._link = null;

        Main.layoutManager.disconnect(this._monitorsId);
        if (this._monitorsTimeout)
            GLib.source_remove(this._monitorsTimeout);
        this._monitorsTimeout = 0;
        this._injections.clear();
        this._injections = null;
        this._unpatchFullscreen();
        for (const layer of [...this._layers])
            layer.destroy();
        this._layers = null;
        this._keeper.disable();
        this._keeper = null;
        this._settings.destroy();
        this._settings = null;
        Main.layoutManager._updateBackgrounds?.();
    }

    _refreshLayers() {
        for (const layer of this._layers ?? [])
            layer.refresh();
    }

    // Every background GNOME creates (desktop, Overview workspaces, lock
    // screen) gets a live layer on top of the static wallpaper.
    _injectBackgrounds() {
        const self = this;
        this._injections.overrideMethod(Background.BackgroundManager.prototype,
            '_createBackgroundActor', original => function (...args) {
                const backgroundActor = original.call(this, ...args);
                const container = this._container;
                const onLockScreen = container?.style_class?.includes('screen-shield-background') ?? false;
                if (onLockScreen && !self._settings.lockScreen)
                    return backgroundActor;
                const layer = new LiveLayer(backgroundActor, self._keeper);
                self._layers.add(layer);
                layer.onDestroy = () => self._layers?.delete(layer);
                return backgroundActor;
            });
    }

    // Renderer windows must never appear as windows or as a running app.
    _hideRenderers() {
        const im = this._injections;
        im.overrideMethod(Shell.Global.prototype, 'get_window_actors', original =>
            function (...args) {
                return original.call(this, ...args).filter(a => !isRenderer(a.meta_window));
            });
        im.overrideMethod(Workspace.prototype, '_isOverviewWindow', original =>
            function (window) {
                return !isRenderer(window) && original.call(this, window);
            });
        im.overrideMethod(WorkspaceThumbnail.prototype, '_isOverviewWindow', original =>
            function (actor) {
                return !isRenderer(actor.get_meta_window()) && original.call(this, actor);
            });
        im.overrideMethod(Meta.Display.prototype, 'get_tab_list', original =>
            function (...args) {
                return original.call(this, ...args).filter(w => !isRenderer(w));
            });
        im.overrideMethod(Shell.App.prototype, 'get_windows', original =>
            function (...args) {
                return original.call(this, ...args).filter(w => !isRenderer(w));
            });
        im.overrideMethod(Shell.App.prototype, 'get_n_windows', () =>
            function () {
                return this.get_windows().length;
            });
        im.overrideMethod(Shell.AppSystem.prototype, 'get_running', original =>
            function (...args) {
                return original.call(this, ...args).filter(app =>
                    app.get_id() !== APP_DESKTOP_ID || app.get_n_windows() > 0);
            });
        if (typeof Main.wm._shouldAnimateActor === 'function') {
            im.overrideMethod(Main.wm, '_shouldAnimateActor', original =>
                function (actor, ...rest) {
                    return !isRenderer(actor?.meta_window) && original.call(this, actor, ...rest);
                });
        }
    }

    // Renderer windows are fullscreen (exact monitor size) but minimized.
    // Mutter still counts them as "in fullscreen", which would hide the top
    // bar and auto-pause the wallpapers. Keep GNOME's answer, but only when
    // a real window is fullscreen (or monitor-sized) on that monitor.
    _patchFullscreen() {
        const proto = Object.getPrototypeOf(Main.layoutManager.primaryMonitor ?? {});
        const original = Object.getOwnPropertyDescriptor(proto, 'inFullscreen');
        if (!original?.get)
            return;
        this._fullscreenProto = proto;
        this._fullscreenOriginal = original;
        Object.defineProperty(proto, 'inFullscreen', {
            configurable: true,
            get() {
                if (!original.get.call(this))
                    return false;
                return global.display.list_all_windows().some(w => {
                    if (isRenderer(w) || w.minimized || w.get_monitor() !== this.index)
                        return false;
                    if (w.is_fullscreen())
                        return true;
                    const r = w.get_frame_rect();
                    return r.x === this.x && r.y === this.y &&
                        r.width === this.width && r.height === this.height;
                });
            },
        });
        Main.layoutManager._updateFullscreen?.();
    }

    _unpatchFullscreen() {
        if (this._fullscreenProto && this._fullscreenOriginal) {
            Object.defineProperty(this._fullscreenProto, 'inFullscreen',
                this._fullscreenOriginal);
        }
        this._fullscreenProto = this._fullscreenOriginal = null;
        Main.layoutManager._updateFullscreen?.();
    }

    _reportFullscreen(force = false) {
        const fullscreen = Main.layoutManager.monitors.some(m => m.inFullscreen);
        if (!force && fullscreen === this._lastFullscreen)
            return;
        this._lastFullscreen = fullscreen;
        if (this._link?.running)
            this._link.activate('fullscreen-active', GLib.Variant.new_boolean(fullscreen));
    }

    _syncIndicator() {
        const wanted = this._settings.panelIcon && !Main.sessionMode.isLocked;
        if (wanted && !this._indicator) {
            this._indicator = new Indicator(this, this._link);
            Main.panel.addToStatusArea(this.uuid, this._indicator);
        } else if (!wanted && this._indicator) {
            this._indicator.destroy();
            this._indicator = null;
        }
    }
}
