#!/bin/bash
# Installs Frontrise for the current user (no root needed).
#   ./install.sh            build (in the frontrise-dev distrobox) and install
#   ./install.sh --no-build install an existing target/release/frontrise
#   ./install.sh --uninstall
set -euo pipefail

cd "$(dirname "$0")"
UUID=frontrise@ansicode.fr
APP_ID=fr.ansicode.frontrise
BIN_DIR=$HOME/.local/bin
DATA=${XDG_DATA_HOME:-$HOME/.local/share}
EXT_DIR=$DATA/gnome-shell/extensions/$UUID

if [ "${1:-}" = "--uninstall" ]; then
    command -v frontrise >/dev/null && frontrise quit 2>/dev/null || true
    gnome-extensions disable "$UUID" 2>/dev/null || true
    rm -f "$BIN_DIR/frontrise" "$DATA/applications/$APP_ID.desktop" \
          "$DATA/icons/hicolor/scalable/apps/$APP_ID.svg" \
          "${XDG_CONFIG_HOME:-$HOME/.config}/autostart/$APP_ID.desktop"
    rm -rf "$EXT_DIR"
    echo "Frontrise removed. Your library and settings stay in ~/.config/frontrise."
    exit 0
fi

if [ "${1:-}" != "--no-build" ]; then
    if command -v distrobox >/dev/null && distrobox list | grep -q frontrise-dev; then
        distrobox enter frontrise-dev -- cargo build --release
    else
        cargo build --release
    fi
fi

install -Dm755 target/release/frontrise "$BIN_DIR/frontrise"
install -Dm644 data/icons/$APP_ID.svg "$DATA/icons/hicolor/scalable/apps/$APP_ID.svg"
sed "s|^Exec=frontrise|Exec=$BIN_DIR/frontrise|" data/$APP_ID.desktop \
    | install -Dm644 /dev/stdin "$DATA/applications/$APP_ID.desktop"
mkdir -p "$EXT_DIR/icons"
install -m644 extension/extension.js extension/metadata.json "$EXT_DIR/"
install -m644 extension/icons/frontrise-symbolic.svg "$EXT_DIR/icons/"

# Open backdrop:// links with Frontrise.
update-desktop-database "$DATA/applications" 2>/dev/null || true
xdg-mime default "$APP_ID.desktop" x-scheme-handler/backdrop 2>/dev/null || true

if gnome-extensions info "$UUID" >/dev/null 2>&1; then
    gnome-extensions enable "$UUID" && echo "Extension enabled."
    echo "Frontrise installed. Start it from the app grid or run: frontrise"
else
    echo "Frontrise installed."
    echo "GNOME loads new extensions at login: log out and back in once, then run:"
    echo "    gnome-extensions enable $UUID"
fi
