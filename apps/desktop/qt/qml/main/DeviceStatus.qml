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
AbstractButton {
    id: button
    objectName: "DeviceStatus.button"
    // What the button shows, for the smoke driver
    readonly property string shownName: DeviceModel.connected ? DeviceModel.deviceTitle : qsTr("No Devices")
    readonly property string notice: LinkController.lockNotice
    readonly property string tip: DeviceModel.remote && LinkController.sessionTip.length > 0
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
        Icon {
            visible: button.notice.length > 0
            name: "lock"
            size: 12
            color: Theme.warning
        }
        Text {
            objectName: "DeviceStatus.text"
            Layout.fillWidth: true
            text: button.notice.length ? button.notice : button.shownName
            font: Theme.fontSmall
            color: button.notice.length ? Theme.warning : DeviceModel.connected ? Theme.text : Theme.dangerText
            elide: Text.ElideRight
        }
        Icon {
            name: "chevrons-up-down"
            size: 12
            color: Theme.textMuted
        }
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        color: button.down ? Theme.controlPressed : button.hovered ? Theme.controlHover : "transparent"
        border.width: button.visualFocus ? 2 : 0
        border.color: Theme.focusRing
    }

    HelpTip {
        visible: button.hovered
        text: button.notice.length ? button.notice + "\n" + button.tip : button.tip
    }

    Accessible.role: Accessible.Button
    Accessible.name: shownName
    Accessible.description: notice.length ? notice + ". " + tip : tip
}
