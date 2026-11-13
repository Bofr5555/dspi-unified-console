#!/bin/sh
# -- WP QP20 -- Installs uConsole for DSPi from this folder for the person
# running it, with no root: the AppImage into ~/.local/bin/uconsole-for-dspi, the
# menu entry and the icon under ~/.local/share. With --udev it also asks (with
# pkexec, or sudo in a terminal) to install the udev rule that lets this seat
# open a DSPi without root; the app offers the same when it needs it.
#
#   ./install.sh [--udev]      install, or update an earlier install
#   ./install.sh --uninstall   remove what install.sh put in place (not the
#                              udev rule, nor the app's settings)
set -eu
here=$(cd "$(dirname "$0")" && pwd)
bin=${XDG_BIN_HOME:-$HOME/.local/bin}
data=${XDG_DATA_HOME:-$HOME/.local/share}
app=$bin/uconsole-for-dspi
entry=$data/applications/uconsole-for-dspi.desktop
icon=$data/icons/hicolor/scalable/apps/uconsole-for-dspi.svg
rule=/etc/udev/rules.d/70-uconsole-for-dspi.rules

refresh() {
    command -v update-desktop-database >/dev/null 2>&1 && update-desktop-database -q "$data/applications" || true
    command -v gtk-update-icon-cache >/dev/null 2>&1 && gtk-update-icon-cache -q -t "$data/icons/hicolor" || true
}

case ${1:-} in
--uninstall)
    rm -f "$app" "$entry" "$icon"
    refresh
    echo "Removed $app, its menu entry and its icon."
    [ ! -e "$rule" ] || echo "The udev rule stays in $rule; remove it as root if no longer wanted."
    exit 0 ;;
''|--udev) ;;
*) echo "usage: install.sh [--udev] | --uninstall" >&2; exit 2 ;;
esac

image=$(ls "$here"/uConsole-for-DSPi-*-x86_64.AppImage 2>/dev/null | head -n 1)
[ -n "$image" ] || { echo "install.sh: no AppImage beside this script" >&2; exit 1; }
mkdir -p "$bin" "$data/applications" "$(dirname "$icon")"
# Replaced by a rename, so a running copy keeps its file until it quits
cp "$image" "$app.new" && chmod 755 "$app.new" && mv -f "$app.new" "$app"
# The entry starts this copy: Exec by its full path (quoted for the desktop
# entry spec), and it is the one desktops show
q=$(printf '%s' "$app" | sed 's/\\/\\\\\\\\/g; s/"/\\\\"/g; s/`/\\\\`/g; s/\$/\\\\$/g')
{ grep -v '^Exec=' "$here/uconsole-for-dspi.desktop"; printf 'Exec="%s"\n' "$q"; } >"$entry.new" && mv -f "$entry.new" "$entry"
cp "$here/uconsole-for-dspi.svg" "$icon"
refresh
echo "Installed $app (menu: uConsole for DSPi)."
case :$PATH: in *:"$bin":*) ;; *) echo "Note: $bin is not on your PATH; the menu entry works regardless." ;; esac

if [ "${1:-}" = --udev ]; then
    script='install -m 0644 "$1" /etc/udev/rules.d/70-uconsole-for-dspi.rules && udevadm control --reload-rules && udevadm trigger --subsystem-match=usb --action=change'
    if command -v pkexec >/dev/null 2>&1; then
        pkexec /bin/sh -c "$script" sh "$here/70-uconsole-for-dspi.rules"
    else
        sudo /bin/sh -c "$script" sh "$here/70-uconsole-for-dspi.rules"
    fi
    echo "Installed the udev rule $rule."
fi
