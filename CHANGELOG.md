# Changelog

## Unreleased

### Fixed
- Boomerang videos froze for about 8 s on every turn. Their clip was cached
  as Matroska, and GTK's loop hand-over stalls on that container; clips are
  now MP4 and play smoothly. Clips cached by 0.5.0 are rendered again.

## 0.5.0 - 2026-11-13

### Added
- Videos can play as a boomerang (forward, then backward, forever) instead of
  a plain loop. Pick it per video from the "⋯" menu in the library. The
  forward-and-back clip is rendered once with `ffmpeg` and cached; until it
  is ready, or if `ffmpeg` is missing, the video simply loops.

## 0.4.2 - 2026-11-05

### Fixed
- Scenes (Aurora, Pulse, Nebula…) could fail with "Unable to create a GL
  context", and Frontrise could abort, after a video wallpaper had been
  shown. When a video was released, GStreamer called `eglTerminate` on the EGL
  display it shares with GTK, which broke OpenGL for the whole app. Frontrise
  now provides a no-op `eglTerminate` that GStreamer binds to; GTK is not
  affected.
- A scene whose OpenGL context failed no longer retries on every frame, which
  flooded the journal.
- Crashes and freezes when switching wallpapers: a replaced video is now
  paused and released 1.5 s later, instead of being destroyed while
  GStreamer's playback thread is still running.

### Changed
- With "Hardware video decoding" on, NVIDIA's decoders (NVDEC) are only used
  when the NVIDIA GPU drives the display. On hybrid laptops, videos are
  decoded with VA-API on the GPU that drives the screen, without waking the
  NVIDIA GPU.

## 0.4.1 - 2026-11-05

### Fixed
- In the Overview, a strip of the static wallpaper could show below the
  animated one (notably when the dash appeared). The extension resized the
  live clone from the background's size notifications, after GNOME had
  already allocated it, which left the clone one layout behind and flooded
  the journal with "needs an allocation" warnings. The clone is now laid out
  by a layout manager, within the same allocation pass as the background.

## 0.4.0 - 2026-11-05

### Added
- Frontrise now sets a still image of the animated wallpaper as GNOME's own
  wallpaper (`picture-uri` and `picture-uri-dark`), so the fallback shown when
  Frontrise or its extension is not running matches what was playing. The
  still is a frame of the video, or a render of the scene, at the native
  resolution of the first display, and is refreshed whenever the content
  changes (choice, playlist step, resolution change).
- New *Replace the system wallpaper with a still* switch in the preferences,
  on by default. The previous system wallpaper is saved once and restored when
  the switch is turned off; a wallpaper the user changed since is left alone.

## 0.3.4 - 2026-11-05

### Fixed
- The wallpaper of a display could still be drawn in the bottom-right quarter
  of the screen after login: the extension minimized the renderer window while
  it was being mapped, which left a stale offset in Mutter's fullscreen
  surface. It now keeps the window mapped for one paint before minimizing it.

## 0.3.3 - 2026-11-05

### Added
- The GNOME extension's files are embedded in the `frontrise` binary, so
  packages (RPM, deb, PKGBUILD) only need to ship the binary, the desktop file
  and the icon. When the extension is missing from
  `$XDG_DATA_HOME/gnome-shell/extensions/frontrise@ansicode.fr`, Frontrise
  offers to install it there, adding the running GNOME Shell major version to
  `shell-version` when missing; the banner gets an *Install* button.
- Frontrise refreshes the installed extension when its files differ from the
  embedded ones (e.g. after an app update), through the same consent question.

### Changed
- Repairing an out-of-date extension now rewrites the user's copy from the
  embedded files (plus the running Shell version) instead of patching the
  installed `metadata.json` only; a system-wide copy is never modified.
- The log-out notice is shown after any install or refresh; a manual repair
  that changes nothing says so instead.
- `install.sh` still copies the extension (so it works without starting the
  app), now every file of `extension/`.

### Fixed
- The extension lays the live clone out again whenever it refreshes and
  shortly after a renderer window appears. This did not fix the wallpaper
  being drawn in the bottom-right quarter of the screen after login; see
  0.3.4.

## 0.3.2 - 2026-11-05

### Added
- *Preferences → GNOME Extension → Repair extension* button, and a *Repair*
  button in the banner, to repair the extension manually at any time.
- After every repair, Frontrise tells the user to log out and back in for the
  extension to become active (dialog, or notification when the window is
  closed); failed manual repairs explain why.

### Changed
- Frontrise now asks before repairing its GNOME extension. The answer is kept
  in `config.json` until the next update or reinstall of Frontrise (keyed to
  the version and the binary's install time); closing the question without
  answering asks again at the next launch, never twice in the same launch.
  When the window is closed, a notification invites the user to open it.

### Fixed
- The GNOME extension no longer stays dead after a GNOME Shell upgrade: the
  extension declares Shell 51 support, and Frontrise detects the "out of date"
  state, adds the running Shell version to the installed `metadata.json` and
  asks for a re-login.
- With the user's consent, Frontrise re-enables the extension when it is turned
  off (including when the global *Extensions* switch is off), once per problem
  and launch. Failures are shown in the banner with the manual command.

## 0.3.1 - 2026-11-04

### Changed
- Library: scene previews are cached as PNG instead of live GL surfaces.

## 0.3.0 - 2026-11-04

### Removed
- Backdrop support; the library now renders real scene previews.

## 0.2.2 - 2026-11-04

### Fixed
- Nebula and aurora scenes use a sine-free hash.

## 0.2.1 - 2026-11-04

### Added
- README: icon header, prerequisites and traditional distro install guide.

## 0.2.0 - 2026-11-04

- Initial release: Frontrise animated wallpaper engine.
