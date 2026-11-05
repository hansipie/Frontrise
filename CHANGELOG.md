# Changelog

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
