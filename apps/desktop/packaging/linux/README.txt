uConsole for DSPi for Linux (x86_64)

This folder holds the app as an AppImage, which runs on most Linux systems
from 2024 on (glibc 2.39 or newer) and brings its own Qt.

Run it as it is:
    ./uConsole-for-DSPi-<version>-x86_64.AppImage
(It needs FUSE 3 to mount itself. Without it, run it with
--appimage-extract-and-run, or set APPIMAGE_EXTRACT_AND_RUN=1.)

Like every AppImage it uses a few of the system's own libraries. Desktops have
them; a minimal system may lack the PulseAudio client library (libpulse.so.0,
also what PipeWire's desktops use), which the app needs to start: install
libpulse0 (Debian, Ubuntu), pulseaudio-libs (Fedora) or libpulse (Arch, openSUSE).

Or install it for yourself, with a menu entry:
    ./install.sh            (./install.sh --uninstall removes it again)

Opening a DSPi needs permission to use its USB interface. Your distribution
may already give it to the person at the seat; if not, the app says so and
offers to install a udev rule (it asks for an administrator's password once).
Or install this folder's rule now, which also covers boards with older
firmware: ./install.sh --udev puts 70-uconsole-for-dspi.rules in
/etc/udev/rules.d/ and reloads udev's rules.

Updating the firmware puts the board in BOOTSEL mode, where it shows as a USB
drive; your desktop mounts it as it does any drive.

Settings live in ~/.config/uconsole-for-dspi/. The licences of the app and of everything
it bundles are in NOTICE.txt and LICENSES/ (also in the app: Settings > About).
