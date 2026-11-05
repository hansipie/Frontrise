# Changelog

## 0.3.2 - 2026-11-05

### Fixed
- The GNOME extension no longer stays dead after a GNOME Shell upgrade: the
  extension declares Shell 51 support, and Frontrise detects the "out of date"
  state, adds the running Shell version to the installed `metadata.json` and
  asks for a re-login.
- Frontrise re-enables the extension by itself when it is turned off (including
  when the global *Extensions* switch is off), once per problem and launch.
  Failures are shown in the banner with the manual command.

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
