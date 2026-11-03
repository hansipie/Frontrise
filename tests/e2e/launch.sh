#!/bin/bash
# End-to-end test in a headless, fully isolated GNOME Shell (2 virtual
# 1920x1080 monitors). Nothing touches the real session: XDG dirs, dconf
# and D-Bus are private, and the environment is exported *before* the
# private bus starts so bus-activated services (dconf…) inherit it.
#
#   tests/e2e/launch.sh <empty-or-new-workdir> tests/e2e/scenarios/<name>.sh
# Results: <workdir>/result.txt, app.log, shell.log, shots/*.png
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
ROOT=$(cd "$HERE/../.." && pwd)
W=$(mkdir -p "$1" && cd "$1" && pwd)
SCEN=$(cd "$(dirname "$2")" && pwd)/$(basename "$2")

mkdir -p "$W/data/gnome-shell/extensions" "$W/data/applications" "$W/cfg" "$W/cache" "$W/cmd" "$W/shots"
cp -r "$ROOT/extension/." "$W/data/gnome-shell/extensions/frontrise@ansicode.fr/"
cp -r "$HERE/driver/." "$W/data/gnome-shell/extensions/frontrise-test@local/"
sed "s|^Exec=frontrise|Exec=$ROOT/target/release/frontrise|" "$ROOT/data/fr.ansicode.frontrise.desktop" \
    > "$W/data/applications/fr.ansicode.frontrise.desktop"
printf 'user-db:test\n' > "$W/profile"
: > "$W/result.txt"
: > "$W/app.log"

export XDG_DATA_HOME=$W/data XDG_CONFIG_HOME=$W/cfg XDG_CACHE_HOME=$W/cache
export DCONF_PROFILE=$W/profile FR_CMD=$W/cmd GTK_A11Y=none
export FR=$ROOT/target/release/frontrise ROOT
exec timeout 170 dbus-run-session -- "$HERE/run.sh" "$W" "$SCEN"
