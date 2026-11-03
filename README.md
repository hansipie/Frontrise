# Frontrise

Animated video and shader wallpapers for GNOME (Shell 50, Wayland).

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

## Build and install

On Fedora Atomic (Silverblue, Bazzite…), build in a distrobox:

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

## Command line

    frontrise                 open the window
    frontrise --background    start without a window (used at login)
    frontrise show displays   open a page (library, playlists, displays)
    frontrise set FILE        import FILE and play it on every display
    frontrise backdrop://wallpaper/ID   play the video linked to that ID
    frontrise pause|resume|toggle|next|previous|quit

## backdrop:// links

Frontrise registers itself for `backdrop://` links. A
`backdrop://wallpaper/<UUID>` link plays the library video linked to that
ID (⋯ → Link to Backdrop… on a card). For an unknown ID, Frontrise offers to
choose a video file and link it. The Backdrop catalog itself is never
contacted: links only resolve to your own files. Other `backdrop://` links
(collections, creators, profiles) are not supported.

## End-to-end tests

    tests/e2e/launch.sh /tmp/frontrise-e2e tests/e2e/scenarios/backdrop-links.sh

Runs a headless, isolated GNOME Shell 50 (private D-Bus, dconf and XDG
dirs) with the extension and a test driver; results and screenshots land in
the work directory.

## Writing scenes

A scene is a `.glsl` / `.frag` file defining
`void mainImage(out vec4 fragColor, in vec2 fragCoord)`.
Uniforms: `iTime`, `iTimeDelta`, `iFrame`, `iResolution`, `iMouse` (always 0),
`iSpectrum[32]`, `iLevel`, `iBass` (0..1). Texture channels (`iChannel*`) are
not supported. A scene that fails to compile shows a plain gradient and the
error is printed on stderr.

## Limitations

- GNOME Shell 50 only (tested in a nested headless GNOME Shell 50.6).
- No playback speed control.
- Wallpapers do not receive clicks (GNOME gives no input to the background).
- The renderer windows are recognised by their title.
