import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP1 -- The device button at the foot of the sidebar (Components.swift's
// ConnectionStatusIndicator): the connection dot and the device's name. A
// click opens the device list; a right-click forces a reconnect (PARITY M57),
// except during a hub session (behaviour.md §3). While another console holds a
// hub device's lock (LinkController.lockNotice), the line shows that notice in
// the warning colour instead; during a hub session the tooltip is the Link
// client's (LinkController.sessionTip: the device via the hub).
// -- WP SECQT -- A hub session that is not encrypted (a plaintext or upstream
// hub, or one in compatibility mode) turns the line red and says so in words,
// " (unsecured)" after the name (the core's dlink.unsecured), with an info
// button beside it: not colour alone.
AbstractButton {
    id: button
    objectName: "DeviceStatus.button"
    // What the button shows, for the smoke driver
    // -- WP P1QT1 -- an offline document says so in place of a device
    readonly property string shownName: FilesController.offline
        //: %1 is the open file's name
        ? qsTr("File: %1").arg(FilesController.offlineFile)
        : DeviceModel.connected ? activeTitle || DeviceModel.deviceTitle : qsTr("No Devices")
    // -- WP LAY7 -- Of several open devices (left open by Modern) the one shown,
    // as its tab and the device list name it (Demo 2)
    readonly property string activeTitle: {
        const row = DeviceRegistry.devices.length > 1 ? DeviceRegistry.devices.find(d => d.active) : null
        return row ? row.title : ""
    }
    readonly property string notice: LinkController.lockNotice
    // -- WP SECQT --
    readonly property bool unsecured: DeviceModel.remote && LinkController.unsecured && !FilesController.offline
    readonly property string shownLine: unsecured
        ? shownName + " (" + LinkController.unsecuredLabel + ")" : shownName
    readonly property string tip: FilesController.offline   // -- WP P1QT1 --
        ? qsTr("A file is open, not a device: edits change the file once you save it (File > Save File).")
        : DeviceModel.remote && LinkController.sessionTip.length > 0
        ? LinkController.sessionTip
        : DeviceModel.connected
        //: %1 is the device's serial number
        ? qsTr("Connected (%1). Right-click the device name to reconnect.").arg(DeviceModel.serial)
        : qsTr("Not connected. Right-click the device name to retry.")

    implicitHeight: 28
    hoverEnabled: true
    onClicked: Shell.trigger("device.devices")

    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: if (!DeviceModel.remote) DeviceModel.reconnect()
    }

    contentItem: RowLayout {
        spacing: 6
        Rectangle {
            width: 6
            height: 6
            radius: 3
            color: DeviceModel.connected ? Theme.connected : Theme.disconnected
        }
        Icon {   // -- WP P1QT1 --
            objectName: "DeviceStatus.file"
            visible: FilesController.offline && button.notice.length === 0
            name: "folder-open"
            size: 12
            color: Theme.textMuted
        }
        Icon {
            visible: button.notice.length > 0
            name: "lock"
            size: 12
            color: Theme.warning
        }
        Text {
            objectName: "DeviceStatus.text"
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: button.notice.length ? button.notice : button.shownName
            font: Theme.fontSmall
            color: button.notice.length ? Theme.warning
                 : button.unsecured || !DeviceModel.connected ? Theme.dangerText : Theme.text
            elide: Text.ElideRight
        }
        // -- WP SECQT -- " (unsecured)", never elided, and what it means, calmly
        Text {
            objectName: "DeviceStatus.unsecured"
            visible: button.unsecured
            text: "(" + LinkController.unsecuredLabel + ")"
            font: Theme.fontSmall
            color: Theme.dangerText
        }
        UnsecuredInfoButton {
            objectName: "DeviceStatus.unsecuredInfo"
            visible: button.unsecured
        }
        // -- WP QPI3 -- a DSPi the app sees but cannot open: the help again
        // (behaviour.md §13, the status area offers it)
        IconButton {
            objectName: "DeviceStatus.access"
            visible: !DeviceModel.connected && DeviceAccess.count > 0
            iconName: "key-round"
            iconSize: 12
            toolTip: qsTr("A DSPi was found that cannot be opened. Show how to give access.")
            onClicked: DeviceAccess.offer()
        }
        Icon {
            name: "chevrons-up-down"
            size: 12
            color: Theme.textMuted
        }
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        // -- WP SECQT -- red while unsecured
        color: button.down ? Theme.controlPressed : button.hovered ? Theme.controlHover
             : button.unsecured ? Theme.bannerFill("error") : "transparent"
        border.width: button.visualFocus ? 2 : button.unsecured ? 1 : 0
        border.color: button.visualFocus ? Theme.focusRing : Theme.danger
    }

    HelpTip {
        visible: button.hovered
        text: button.notice.length ? button.notice + "\n" + button.tip : button.tip
    }

    Accessible.role: Accessible.Button
    Accessible.name: shownLine   // -- WP SECQT --
    Accessible.description: notice.length ? notice + ". " + tip : tip
}
