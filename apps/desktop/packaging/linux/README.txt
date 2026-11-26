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
This copies the AppImage to ~/.local/bin/uconsole-for-dspi and adds the menu
entry and the icon under ~/.local/share; no administrator is needed. To update,
run the new version's install.sh the same way. --uninstall leaves the udev rule
and your settings in place.

Opening a DSPi needs permission to use its USB interface. If this account
does not have it, the app says so and offers to install the same udev rule as
this folder's (it asks for an administrator's password once). Or install this
folder's rule now: ./install.sh --udev puts 70-dspi.rules in
/etc/udev/rules.d/ and reloads udev's rules. The rule lets every account open
a DSPi (MODE 0666) and also covers the bootloader a DSPi shows during a
firmware update; it is the same file the Terminal's `uconsole --install-udev`
and the .deb, .rpm and AUR packages install.

Updating the firmware puts the board in BOOTSEL mode, where it shows as a USB
drive; your desktop mounts it as it does any drive.

The app can tell you when a newer release is out (Settings › Updates, off until
you turn it on); it never downloads or installs anything itself.

If you connect Claude Desktop with Settings › AI Agents › Install in Claude
Desktop… (or with the configuration you copied from that page), it names the
AppImage where it was when you made it: make it again after you move the
AppImage or install it with install.sh.

Settings live in ~/.config/uconsole-for-dspi/. The licences of the app and of everything
it bundles are in NOTICE.txt and LICENSES/ (also in the app: Settings › About).
