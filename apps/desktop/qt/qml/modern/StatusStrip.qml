import QtQuick
import UConsole

// -- WP LAY3 -- Under the top bar: only the states that limit what can be done
// (the banners registered as Registrar::Blocking: demo, read-only firmware,
// the installer's lock, a lost hub, a safe apply, an open file, a sender). The
// rest is the notification drawer's.
BannerHost {
    objectName: "StatusStrip"
    types: Shell.blockingBanners
}
