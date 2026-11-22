import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5b -- The local part of the device list (split from DevicesContent):
// the open devices when several are, the DSPis on USB, and the demo. The list
// shows it (DevicesContent), and Modern's Devices > This Computer.
ColumnLayout {
    id: content
    spacing: 4

    // -- WP MDQT1 -- ---- Open devices: only when several are
    SectionHeading {
        objectName: "DevicesPopup.openHeading"
        Layout.fillWidth: true
        visible: DeviceRegistry.devices.length > 1
        text: qsTr("Open devices")
    }
    Repeater {
        model: DeviceRegistry.devices.length > 1 ? DeviceRegistry.devices : []
        delegate: RowLayout {
            id: openRow
            required property var modelData
            Layout.fillWidth: true
            spacing: 2
            DeviceRow {
                Layout.fillWidth: true
                objectName: "DevicesPopup.open." + openRow.modelData.id
                iconName: openRow.modelData.transport === "usb" ? "usb"
                          : openRow.modelData.transport === "demo" ? "play"
                          : openRow.modelData.transport === "file" ? "file-text" : "network"
                title: openRow.modelData.title
                subtitle: openRow.modelData.state === "lost" ? qsTr("Not connected")
                          : openRow.modelData.state === "reconnecting" ? qsTr("Reconnecting…")
                          : openRow.modelData.busy ? openRow.modelData.job   // -- WP MDQT2c --
                          : openRow.modelData.serial
                current: openRow.modelData.active
                onClicked: DevicesController.chooseOpen(openRow.modelData.id)
            }
            IconButton {
                objectName: "DevicesPopup.close." + openRow.modelData.id
                iconName: "x"
                //: %1 is a device's name
                toolTip: qsTr("Close %1").arg(openRow.modelData.title)
                Accessible.name: toolTip
                onClicked: DevicesController.closeOpen(openRow.modelData.id)
            }
        }
    }

    // ---- USB and the demo
    SectionHeading {
        Layout.fillWidth: true
        text: qsTr("USB")
    }
    Repeater {
        model: DevicesController.usbDevices
        delegate: DeviceRow {
            required property var modelData
            Layout.fillWidth: true
            objectName: "DevicesPopup.usb." + modelData.serial
            iconName: "usb"
            title: qsTr("DSPi (%1)").arg(modelData.serial)
            subtitle: qsTr("USB")
            current: DeviceModel.connected && !DeviceModel.remote && !DeviceModel.demo
                     && DeviceModel.serial === modelData.serial
            onClicked: DevicesController.chooseUsb(modelData.serial)
        }
    }
    Text {
        Layout.fillWidth: true
        Layout.leftMargin: 32
        visible: DevicesController.usbDevices.length === 0
        text: qsTr("No DSPi on USB")
        font: Theme.fontSmall
        color: Theme.textMuted
    }
    DeviceRow {
        Layout.fillWidth: true
        objectName: "DevicesPopup.demo"
        iconName: "play"
        title: qsTr("Demo DSPi")
        subtitle: qsTr("Simulated DSPi")
        current: DeviceModel.demo && DeviceModel.connected
        onClicked: DevicesController.chooseDemo()
    }
}
