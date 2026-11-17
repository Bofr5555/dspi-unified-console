# -- WP QP20 -- Sourced by the AppImage's AppRun (from usr/apprun-hooks/) before
# the app starts. The AppImage's Qt is not the desktop's, so it cannot load the
# desktop's own Qt theme plugins; the XDG desktop portal is what it can use
# for file dialogs that match the desktop. A user's own choice stays.
if [ -z "${QT_QPA_PLATFORMTHEME:-}" ] && [ -n "${DBUS_SESSION_BUS_ADDRESS:-}${XDG_RUNTIME_DIR:-}" ] \
    && [ -e "$APPDIR/usr/plugins/platformthemes/libqxdgdesktopportal.so" ]; then
    export QT_QPA_PLATFORMTHEME=xdgdesktopportal
fi

# -- review fu1 -- Qt Multimedia (the microphone) needs the system's PulseAudio
# client library, as the app needs the system's ALSA. Without it the loader
# stops the app with only "libpulse.so.0: cannot open shared object file": say
# which package brings it. Only a hint on stderr, and only when ldconfig
# answers and does not list the library; the app is started either way.
dspi_libs=$( { ldconfig -p || /sbin/ldconfig -p || /usr/sbin/ldconfig -p; } 2>/dev/null )
case "$dspi_libs" in
    '' | *libpulse.so.0*) ;;
    *) echo "uConsole for DSPi needs the PulseAudio client library (libpulse.so.0), which this system lacks: install libpulse0 (Debian, Ubuntu), pulseaudio-libs (Fedora) or libpulse (Arch, openSUSE)." >&2 ;;
esac
unset dspi_libs
