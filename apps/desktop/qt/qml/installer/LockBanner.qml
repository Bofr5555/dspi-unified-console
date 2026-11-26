import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Window
import UConsole

// -- WP P1QT2 -- The locked-user view (P1LK, docs/UI-MODES.md): while the
// installer's lock holds, only volume, mute, source and presets change; every other
// write is refused by the core with its notice. The banner says so, with the
// installer's note and contact, and offers the one unlock prompt. With no PIN
// set it never shows.
Banner {
    id: banner
    objectName: "LockBanner"
    kind: "info"
    iconName: "lock"
    visible: LimitsController.locked
    readonly property string note: LimitsController.lock.note || ""
    readonly property string contact: LimitsController.lock.contact || ""
    // -- WP P1EFIX -- the banner's line is the core's row (installer_lock.banner),
    // the same in every client; the row's title before the table is read
    readonly property string line: NoticeCenter.row("installer_lock.banner").reason || qsTr("Locked by the installer")
    text: [line, note, contact.length ? qsTr("Installer: %1").arg(contact) : ""].filter(t => t.length > 0).join("  ")

    AppButton {
        objectName: "LockBanner.unlock"
        text: qsTr("Unlock…")
        onClicked: unlockDialog.ask()
    }

    UnlockDialog {
        id: unlockDialog
        parent: Overlay.overlay
        // Unlocked, the banner is gone with the button the keyboard would go
        // back to: the window's content takes it instead
        onClosed: if (!banner.visible && banner.Window.window) banner.Window.window.contentItem.forceActiveFocus()
    }
}
