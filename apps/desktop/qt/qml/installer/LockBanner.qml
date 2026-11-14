import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Window
import UConsole

// -- WP P1QT2 -- The locked-user view (P1LK, docs/UI-MODES.md): while the
// installer's lock holds, only volume, mute and presets change; every other
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
    // The lock's title in the core's words (its notice row), then what still works
    readonly property string title: NoticeCenter.row(LimitsController.heldByHub ? "installer_lock.hub" : "installer_lock.locked").title
                                    || qsTr("Locked by the installer")
    //: The locked-user banner: %1 the lock's title ("Locked by the installer")
    text: [qsTr("%1: only volume, mute and presets can be changed.").arg(title),
           note, contact.length ? qsTr("Installer: %1").arg(contact) : ""].filter(t => t.length > 0).join("  ")

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
