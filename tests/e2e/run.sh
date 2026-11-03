#!/bin/bash
# Started by launch.sh inside the private D-Bus session.
T=$1
SCEN=$2
dconf write /org/gnome/shell/enabled-extensions "['frontrise@ansicode.fr', 'frontrise-test@local']"
dconf write /org/gnome/shell/disable-user-extensions false
dconf write /org/gnome/shell/welcome-dialog-last-shown-version "'999'"
gnome-shell --headless --wayland --virtual-monitor 1920x1080 --virtual-monitor 1920x1080 > "$T/shell.log" 2>&1 &
SHELL_PID=$!
for _ in $(seq 1 40); do
    ls "$XDG_RUNTIME_DIR" | grep -q '^wayland-[1-9]$' && break
    sleep 0.5
done
WAYLAND_DISPLAY=$(ls "$XDG_RUNTIME_DIR" | grep '^wayland-[1-9]$' | tail -1)
export WAYLAND_DISPLAY GDK_BACKEND=wayland
sleep 6

# cmd <id> <command>: ask the driver extension, wait for its answer.
cmd() {
    echo "$2" > "$T/cmd/cmd-$1"
    for _ in $(seq 1 60); do
        [ -f "$T/cmd/done-$1" ] && break
        sleep 0.25
    done
    echo "== $1 ($2): $(cat "$T/cmd/done-$1" 2>/dev/null || echo TIMEOUT)" >> "$T/result.txt"
}

. "$SCEN"

"$FR" quit >> "$T/app.log" 2>&1
sleep 1
kill $SHELL_PID 2>/dev/null
wait $SHELL_PID 2>/dev/null
echo "== finished" >> "$T/result.txt"
