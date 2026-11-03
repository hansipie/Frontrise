// Test-only GNOME Shell extension: executes commands written as files
// "cmd-<id>" in $FR_CMD and answers in "done-<id>".
// Commands: shot <png>, overview show|hide, lock, unlock, state.
import Gio from 'gi://Gio';
import GLib from 'gi://GLib';
import Meta from 'gi://Meta';
import Shell from 'gi://Shell';
import {Extension} from 'resource:///org/gnome/shell/extensions/extension.js';
import * as Main from 'resource:///org/gnome/shell/ui/main.js';

Gio._promisify(Shell.Screenshot.prototype, 'screenshot', 'screenshot_finish');

export default class TestDriver extends Extension {
    enable() {
        this._dir = GLib.getenv('FR_CMD');
        this._busy = false;
        this._id = GLib.timeout_add(GLib.PRIORITY_DEFAULT, 250, () => {
            this._poll();
            return GLib.SOURCE_CONTINUE;
        });
    }

    disable() {
        GLib.source_remove(this._id);
    }

    _done(name, text = 'ok') {
        GLib.file_set_contents(`${this._dir}/done-${name}`, text);
    }

    async _poll() {
        if (this._busy || !this._dir)
            return;
        const dir = Gio.File.new_for_path(this._dir);
        const e = dir.enumerate_children('standard::name', 0, null);
        let info;
        while ((info = e.next_file(null))) {
            const name = info.get_name();
            if (!name.startsWith('cmd-'))
                continue;
            const id = name.slice(4);
            const file = dir.get_child(name);
            const [, bytes] = file.load_contents(null);
            file.delete(null);
            this._busy = true;
            try {
                await this._run(id, new TextDecoder().decode(bytes).trim());
            } catch (err) {
                this._done(id, `ERR ${err}`);
            }
            this._busy = false;
        }
    }

    async _run(id, cmd) {
        const [verb, arg] = cmd.split(' ');
        if (verb === 'shot') {
            const s = Gio.File.new_for_path(arg).replace(null, false, Gio.FileCreateFlags.NONE, null);
            await new Shell.Screenshot().screenshot(false, s);
            s.close(null);
            this._done(id);
        } else if (verb === 'overview') {
            if (arg === 'show')
                Main.overview.show();
            else
                Main.overview.hide();
            this._done(id);
        } else if (verb === 'lock') {
            Main.screenShield.lock(false);
            this._done(id, `locked=${Main.sessionMode.isLocked}`);
        } else if (verb === 'unlock') {
            Main.screenShield.deactivate(false);
            this._done(id);
        } else if (verb === 'state') {
            const rect = w => {
                const r = w.get_frame_rect();
                return [r.x, r.y, r.width, r.height];
            };
            const out = {
                tab: global.display.get_tab_list(Meta.TabList.NORMAL_ALL, null).map(w => w.get_title()),
                actors: global.get_window_actors().map(a => a.meta_window.get_title()),
                running: Shell.AppSystem.get_default().get_running().map(a => a.get_id()),
                windows: global.display.list_all_windows().map(w =>
                    `${w.get_title()} min=${w.minimized} mon=${w.get_monitor()} ${JSON.stringify(rect(w))}`),
                indicator: !!Main.panel.statusArea['frontrise@ansicode.fr'],
                fullscreen: Main.layoutManager.monitors.map(m => m.inFullscreen),
                locked: Main.sessionMode.isLocked,
                layers: [],
            };
            const walk = actor => {
                if (actor.constructor.name === 'Meta_BackgroundActor') {
                    const [tx, ty] = actor.get_transformed_position();
                    out.layers.push({
                        mon: actor.monitor, mapped: actor.mapped, y: ty, x: tx,
                        clones: actor.get_children().map(l => l.get_n_children()),
                    });
                }
                for (const c of actor.get_children())
                    walk(c);
            };
            walk(global.stage);
            this._done(id, JSON.stringify(out, null, 1));
        } else {
            this._done(id, 'unknown');
        }
    }
}
