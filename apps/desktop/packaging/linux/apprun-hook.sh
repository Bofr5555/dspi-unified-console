# -- WP QP20 -- Sourced by the AppImage's AppRun (from usr/apprun-hooks/) before
# the app starts. The AppImage's Qt is not the desktop's, so it cannot load the
# desktop's own Qt theme plugins; the XDG desktop portal is what it can use
# for file dialogs that match the desktop. A user's own choice stays.
if [ -z "${QT_QPA_PLATFORMTHEME:-}" ] && [ -n "${DBUS_SESSION_BUS_ADDRESS:-}${XDG_RUNTIME_DIR:-}" ] \
    && [ -e "$APPDIR/usr/plugins/platformthemes/libqxdgdesktopportal.so" ]; then
    export QT_QPA_PLATFORMTHEME=xdgdesktopportal
fi
