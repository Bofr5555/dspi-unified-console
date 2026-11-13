import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP16 -- A DSPi is connected but cannot be opened (DeviceAccess,
// behaviour.md §13). On Linux it offers the device rule, installed through
// pkexec; on Windows it says what usually holds the device; elsewhere it says
// that the device cannot be opened. DeviceAccess makes it in the main window
// and it opens itself while DeviceAccess.offered.
Dialog {
    id: dialog
    objectName: "UdevDialog"
    readonly property var access: DeviceAccess
    readonly property bool linux: DeviceAccess.system === "linux"
    readonly property bool windows: DeviceAccess.system === "windows"

    modal: true
    closePolicy: Popup.CloseOnEscape
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 320
    width: Math.min(440, hostWidth - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: Math.max(Theme.spacingLarge, (hostHeight - implicitHeight) / 3)
    padding: Theme.spacingLarge
    topPadding: Theme.spacingSmall
    title: linux ? qsTr("DSPi Needs Permission") : qsTr("DSPi Cannot Be Opened")

    onRejected: DeviceAccess.dismiss()
    Connections {
        target: DeviceAccess
        function onChanged() {
            if (DeviceAccess.offered && !dialog.opened) dialog.open()
            else if (!DeviceAccess.offered && dialog.opened) dialog.close()
        }
    }

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.color: Theme.border
    }
    header: Text {
        leftPadding: Theme.spacingLarge
        rightPadding: Theme.spacingLarge
        topPadding: Theme.spacingLarge
        text: dialog.title
        font: Theme.fontTitle
        color: Theme.text
        wrapMode: Text.Wrap
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }
    contentItem: ColumnLayout {
        spacing: Theme.spacingSmall
        Accessible.role: Accessible.AlertMessage
        Accessible.name: dialog.title
        Accessible.description: body.text
        Text {
            id: body
            objectName: "UdevDialog.text"
            Layout.fillWidth: true
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
            text: dialog.linux
                  ? (DeviceAccess.installed
                     ? qsTr("The device rule is installed. If the DSPi does not appear, unplug it and plug it in again.")
                     : qsTr("A DSPi is connected, but this account may not open it. A device rule lets the person signed in use it; installing it asks for an administrator's password once."))
                  : dialog.windows
                    ? qsTr("A DSPi is connected but cannot be opened. Close other programs that may be using it, then try again. Very old firmware needs the WinUSB driver, which Zadig installs.")
                    : qsTr("A DSPi is connected but cannot be opened. Close other programs that may be using it, then try again.")
        }
        Text {
            objectName: "UdevDialog.rule"
            Layout.fillWidth: true
            visible: dialog.linux && !DeviceAccess.installed
            text: qsTr("The rule goes in %1.").arg("/etc/udev/rules.d/70-dspi.rules")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        Text {
            objectName: "UdevDialog.error"
            Layout.fillWidth: true
            visible: text.length > 0
            text: DeviceAccess.error
            font: Theme.fontSmall
            color: Theme.dangerText
            wrapMode: Text.Wrap
            Accessible.role: Accessible.AlertMessage
            Accessible.name: text
        }
    }
    footer: RowLayout {
        spacing: Theme.spacingSmall
        Item { Layout.fillWidth: true }
        Icon {
            visible: DeviceAccess.installing
            Layout.bottomMargin: Theme.spacing
            name: "loader-circle"
            size: 14
            color: Theme.textMuted
            accessibleName: qsTr("Installing")
        }
        AppButton {
            objectName: "UdevDialog.close"
            Layout.bottomMargin: Theme.spacing
            text: DeviceAccess.installed ? qsTr("Done") : dialog.linux ? qsTr("Not Now") : qsTr("Close")
            onClicked: DeviceAccess.dismiss()
        }
        AppButton {
            objectName: "UdevDialog.install"
            Layout.bottomMargin: Theme.spacing
            Layout.rightMargin: Theme.spacingLarge
            visible: !DeviceAccess.installed
            primary: true
            enabled: !DeviceAccess.installing
            text: dialog.linux ? qsTr("Install Rule…") : qsTr("Try Again")
            onClicked: dialog.linux ? DeviceAccess.installRule() : DeviceAccess.rescan()
        }
    }
}
