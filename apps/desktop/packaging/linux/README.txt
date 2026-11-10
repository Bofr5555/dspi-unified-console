DSPi uConsole for Linux
======================

Install (recommended)
---------------------
    ./install.sh

This copies the AppImage to ~/.local/bin, adds DSPi uConsole to your
application menu, and installs a udev rule so the app can open the DSPi
without root (it asks for your password once for that). Run it again to
update; run ./install.sh --uninstall to remove everything.

Run without installing
----------------------
    chmod +x DSPi-uConsole-*-x86_64.AppImage
    ./DSPi-uConsole-*-x86_64.AppImage

You still need the udev rule once, or the device can't be opened:

    sudo cp 70-dspi.rules /etc/udev/rules.d/
    sudo udevadm control --reload-rules
    (then unplug and replug the DSPi)

Requirements
------------
64-bit x86 Linux with glibc 2.39 or newer (Ubuntu 24.04, Debian 13,
Fedora 42, or anything newer). X11 and Wayland are both supported. The
AppImage brings its own Qt 6; it uses your system's graphics (OpenGL),
fonts and sound (ALSA) libraries, which every desktop has.

If the AppImage won't start with a FUSE error, run it with
--appimage-extract-and-run, or install your distribution's FUSE package
(libfuse2 / fuse).
