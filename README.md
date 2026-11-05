<p align="center">
  <img src="data/icons/fr.ansicode.frontrise.svg" alt="Icône de Frontrise" width="112" height="112">
</p>

<h1 align="center">Frontrise</h1>

Animated video and shader wallpapers for GNOME (Shell 50 and 51, Wayland).

- **Videos** as wallpapers, on every display, in sync or one per display.
- **Scenes**: GLSL shaders (Shadertoy-style `mainImage`), three built in
  (Aurora, Pulse, Nebula). Scenes can **react to sound** (`iSpectrum[32]`,
  `iLevel`, `iBass`): only per-band loudness is used, nothing is recorded.
- **Library** with search, favorites, thumbnails, drag & drop import.
- **Playlists**: interval, shuffle, ordering, next/previous.
- **Displays**: per-display wallpaper, “Same as …” mirroring, fill or fit,
  settings remembered for disconnected displays.
- **Desktop, Overview and lock screen**.
- **Automatic pause**: on battery, in Power Saver, while an app is fullscreen.
- **Top bar menu**, start at login, command line control.

## Prerequisites

To run Frontrise:

- **GNOME Shell 50 or 51** on a **Wayland** session (see
  [Limitations](#limitations)). Check with
  `gnome-shell --version`.
- GTK ≥ 4.14, libadwaita ≥ 1.6, GLib ≥ 2.80 and GStreamer 1.x, with the
  *plugins-base*, *plugins-good* and *libav* plugin sets.
- OpenGL/EGL libraries (present on any GNOME desktop), for the shader scenes.

To build from source, also:

- the development packages of GTK 4, libadwaita and GStreamer, a C compiler
  and `pkg-config`;
- the stable [Rust](https://rustup.rs) toolchain (`cargo`).

Exact package names per distribution are listed in
[Traditional distribution](#traditional-distribution).

## Installation

### Fedora Atomic (Silverblue, Bazzite…)

Build in a distrobox:

    distrobox create --name frontrise-dev --image registry.fedoraproject.org/fedora:44
    distrobox enter frontrise-dev -- sudo dnf install -y gcc pkgconf-pkg-config \
        gtk4-devel libadwaita-devel gstreamer1-devel gstreamer1-plugins-base-devel \
        gstreamer1-plugins-good gstreamer1-plugin-libav
    ./install.sh

`install.sh` builds (in `frontrise-dev` when it exists), then installs the
binary in `~/.local/bin`, the desktop file and icon, and the extension.
GNOME loads a new extension at the next login: log out and back in once, then
`gnome-extensions enable frontrise@ansicode.fr`.
`./install.sh --uninstall` removes everything but your settings.

### Traditional distribution

On a regular (non-immutable) distribution, install the build dependencies
with the system package manager and build directly on the host: no distrobox
is needed. Everything installs for the current user, without root (only the
package installation needs `sudo`).

#### 1. Check your GNOME version

    gnome-shell --version

The extension declares support for **GNOME Shell 50 and 51**
(`extension/metadata.json`). On another version, GNOME refuses to load it and
Frontrise shows no wallpaper. GNOME 51 is declared but untested. If a newer
Shell leaves the extension "out of date", Frontrise adds that version to the
installed `metadata.json` by itself; log out and back in once for it to take
effect. A Wayland session is required.

The app needs GTK ≥ 4.14, libadwaita ≥ 1.6, GLib ≥ 2.80 and GStreamer 1.x.

#### 2. Install the build dependencies

Fedora / RHEL family:

    sudo dnf install gcc pkgconf-pkg-config gtk4-devel libadwaita-devel \
        gstreamer1-devel gstreamer1-plugins-base-devel \
        gstreamer1-plugins-good gstreamer1-plugin-libav

On RHEL and its rebuilds, `gstreamer1-plugin-libav` may not be in the base
repositories (look in EPEL or RPM Fusion).

Debian / Ubuntu:

    sudo apt install build-essential pkg-config libgtk-4-dev libadwaita-1-dev \
        libgstreamer1.0-dev libgstreamer-plugins-base1.0-dev \
        gstreamer1.0-plugins-good gstreamer1.0-libav

Arch Linux:

    sudo pacman -S --needed base-devel gtk4 libadwaita gstreamer \
        gst-plugins-base gst-plugins-good gst-libav

openSUSE:

    sudo zypper install gcc pkgconf gtk4-devel libadwaita-devel \
        gstreamer-devel gstreamer-plugins-base-devel \
        gstreamer-plugins-good gstreamer-plugins-libav

What they are for: GTK 4 and libadwaita for the interface; GStreamer
*plugins-base* for decoding and thumbnails (`uridecodebin`, `appsink`);
*plugins-good* for the sound-reactive scenes (`pulsesrc`, `spectrum`, which
also works on PipeWire); *libav* for common video codecs (H.264, HEVC…).
Scenes use the system OpenGL/EGL libraries (`libEGL`, `libGLESv2` or `libGL`),
present on any GNOME desktop. `gnome-extensions`, `update-desktop-database`
and `xdg-mime` are used by `install.sh` and are normally already installed with
GNOME.

#### 3. Install Rust

Install the stable toolchain with [rustup](https://rustup.rs) (the
`rustup` package of your distribution works too):

    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
    source "$HOME/.cargo/env"

#### 4. Build and install

From the root of the source tree:

    ./install.sh

Without a `frontrise-dev` distrobox, `install.sh` runs `cargo build --release`
on the host, then installs:

- the binary in `~/.local/bin/frontrise`;
- the desktop file and icon in `~/.local/share/applications` and
  `~/.local/share/icons`;
- the extension in `~/.local/share/gnome-shell/extensions/frontrise@ansicode.fr`.

The extension's files are also embedded in the `frontrise` binary, so a
package (RPM, deb, PKGBUILD…) only needs to ship the binary, the desktop file
and the icon: at its first launch, Frontrise offers to install the extension
in `$XDG_DATA_HOME/gnome-shell/extensions/frontrise@ansicode.fr` (by default
`~/.local/share/…`), declaring the running GNOME Shell version. After an
update of Frontrise, it offers to refresh that copy when it differs from the
embedded one. Either way, log out and back in afterwards.

To reinstall an already built `target/release/frontrise`, use
`./install.sh --no-build`. Make sure `~/.local/bin` is in your `PATH` to use
the `frontrise` command (the app grid entry works either way).

#### 5. Enable the extension

GNOME Shell on Wayland only discovers a new extension at login. After the
first install, log out, log back in, then run:

    gnome-extensions enable frontrise@ansicode.fr

(or toggle *Frontrise* in the Extensions app). On later installs,
`install.sh` enables it by itself. If the extension is later found missing,
older than the app, turned off (or the global *Extensions* switch is off), or
out of date after a GNOME upgrade, Frontrise asks before installing or
repairing it. The answer (*Repair* or *Not
Now*) is remembered until Frontrise is updated or reinstalled; with *Repair*,
later problems are fixed automatically, once per launch. *Preferences → GNOME
Extension → Repair extension* (or the banner's *Repair* button) repairs it at
any time. After every repair, log out and back in for the extension to become
active. If a repair fails, the banner shows why and the command above.

#### 6. Start Frontrise

Open *Frontrise* from the app grid, or run `frontrise`. To play your
wallpapers at every login, turn on *Start at login* in the preferences
(System group): it writes `~/.config/autostart/fr.ansicode.frontrise.desktop`,
which runs `frontrise --background`.

#### Uninstall

    ./install.sh --uninstall

This stops Frontrise, disables the extension and removes the binary, desktop
file, icon, autostart entry and extension. Your library and settings stay in
`~/.config/frontrise` (delete it and `~/.cache/frontrise` to remove them too).
The build dependencies are left in place.

## Usage

### Command line

    frontrise                 open the window
    frontrise --background    start without a window (used at login)
    frontrise show displays   open a page (library, playlists, displays)
    frontrise set FILE        import FILE and play it on every display
    frontrise pause|resume|toggle|next|previous|quit

### Writing scenes

A scene is a `.glsl` / `.frag` file defining
`void mainImage(out vec4 fragColor, in vec2 fragCoord)`.
Uniforms: `iTime`, `iTimeDelta`, `iFrame`, `iResolution`, `iMouse` (always 0),
`iSpectrum[32]`, `iLevel`, `iBass` (0..1). Texture channels (`iChannel*`) are
not supported. A scene that fails to compile shows a plain gradient and the
error is printed on stderr.

## How it works

`frontrise` (Rust, GTK 4 + libadwaita, GStreamer) renders each display's
wallpaper in a window titled `FrontriseWallpaper:<x>,<y>:<connector>`.
The GNOME Shell extension (`extension/`, uuid `frontrise@ansicode.fr`):

- keeps these windows minimized and hides them from Alt+Tab, the Overview,
  the dash and the running-apps list;
- paints a live clone of them inside GNOME's own background actors
  (desktop, Overview workspaces, lock screen);
- reports fullscreen apps to the app and adds the top bar menu.

The app and extension talk over D-Bus (`org.gtk.Actions` on
`fr.ansicode.frontrise`). Without the extension, the app creates no wallpaper
window and shows a banner instead.

Data: `~/.config/frontrise/config.json` (library, playlists, displays,
settings — the extension reads `settings` from it), thumbnails in
`~/.cache/frontrise/thumbs`. Videos are referenced, not copied.

## Limitations

- GNOME Shell 50 and 51 (tested only in a nested headless GNOME Shell 50.6).
- On Wayland, a Shell version newly declared in `metadata.json` (or a freshly
  installed extension) is only picked up at the next login.
- Frontrise only installs, refreshes or patches its extension in the user's
  directory (`$XDG_DATA_HOME/gnome-shell/extensions`). A copy installed
  system-wide (`/usr/share/gnome-shell/extensions`) is not touched; when it is
  out of date, the user copy installed by Frontrise takes precedence over it.
- Installing the extension needs a running GNOME Shell (Frontrise reads its
  version over D-Bus). After each update of Frontrise whose extension changed,
  the user is asked again (the answer is kept per installation) and must log
  out and back in once.
- Uninstalling a package does not remove the extension copied in the user's
  directory; `./install.sh --uninstall` or `gnome-extensions uninstall
  frontrise@ansicode.fr` does.
- No playback speed control.
- Wallpapers do not receive clicks (GNOME gives no input to the background).
- The renderer windows are recognised by their title.

## Development

### End-to-end tests

    tests/e2e/launch.sh /tmp/frontrise-e2e tests/e2e/scenarios/<scenario>.sh

Runs a headless, isolated GNOME Shell 50 (private D-Bus, dconf and XDG
dirs) with the extension and a test driver; results and screenshots land in
the work directory.
