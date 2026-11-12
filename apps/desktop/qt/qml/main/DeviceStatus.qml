import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// The device button at the foot of the sidebar (the Mac's
// ConnectionStatusIndicator): the connection dot and the device's name. A
// click opens the device list; a right-click forces a reconnect.
AbstractButton {
    id: button
    objectName: "DeviceStatus.button"
    // What the button shows, for the smoke driver
    readonly property string shownName: DeviceModel.connected ? DeviceModel.deviceTitle : qsTr("No Devices")

    implicitHeight: 28
    hoverEnabled: true
    onClicked: Shell.trigger("device.devices")

    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: DeviceModel.reconnect()
    }

    contentItem: RowLayout {
        spacing: 6
        Rectangle {
            width: 6
            height: 6
            radius: 3
            color: DeviceModel.connected ? Theme.connected : Theme.disconnected
        }
        Text {
            Layout.fillWidth: true
            text: button.shownName
            font: Theme.fontSmall
            color: DeviceModel.connected ? Theme.text : Theme.disconnected
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
        text: DeviceModel.connected ? qsTr("Connected") : qsTr("Not connected. Right-click the device name to retry.")
    }

    Accessible.role: Accessible.Button
    Accessible.name: shownName
    Accessible.description: DeviceModel.connected ? qsTr("Connected") : qsTr("Not connected. Right-click the device name to retry.")
}
