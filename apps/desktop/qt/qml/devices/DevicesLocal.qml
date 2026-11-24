import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5b -- The local part of the device list (split from DevicesContent):
// the open devices when several are, the DSPis on USB, and the demo. The list
// shows it (DevicesContent), and Modern's Devices > This Computer.
// -- WP DL3b -- `listRows`: the shared rows (ListRow, DESIGN-LANGUAGE §9) for
// Modern's page, a status dot on each open device and its Close on hover;
// the list in a popover keeps its DeviceRows. (INTC: DeviceListRow, the
// switcher's rows too.)
ColumnLayout {
    id: content
    spacing: 4
    property bool listRows: false


    // -- WP MDQT1 -- ---- Open devices: only when several are
    SectionHeading {
        objectName: "DevicesPopup.openHeading"
        Layout.fillWidth: true
        visible: DeviceRegistry.devices.length > 1
        text: qsTr("Open devices")
    }
    Repeater {
        model: DeviceRegistry.devices.length > 1 && !content.listRows ? DeviceRegistry.devices : []   // -- WP DL3b --
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

    // -- WP DL3b -- (INTC: the shared DeviceListRow)
    Repeater {
        model: DeviceRegistry.devices.length > 1 && content.listRows ? DeviceRegistry.devices : []
        delegate: DeviceListRow {
            required property var modelData
            required property int index
            Layout.fillWidth: true
            kind: "open"
            device: modelData
            rowIndex: index
        }
    }

    // ---- USB and the demo
    SectionHeading {
        Layout.fillWidth: true
        Layout.topMargin: content.listRows ? Theme.spacing : 0   // -- WP DL3b --
        text: qsTr("USB")
    }
    // -- WP DL3b -- (INTC: the shared DeviceListRow)
    Repeater {
        model: content.listRows ? DevicesController.usbDevices : []
        delegate: DeviceListRow {
            required property var modelData
            required property int index
            Layout.fillWidth: true
            kind: "usb"
            device: modelData
            rowIndex: index
        }
    }
    Repeater {
        model: content.listRows ? [] : DevicesController.usbDevices   // -- WP DL3b --
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
        Layout.leftMargin: content.listRows ? Theme.spacingSmall : 32   // -- WP DL3b --
        visible: DevicesController.usbDevices.length === 0
        text: qsTr("No DSPi on USB")
        font: Theme.fontSmall
        color: Theme.textMuted
    }
    // -- WP DL3b -- (INTC: the shared DeviceListRow)
    DeviceListRow {
        Layout.fillWidth: true
        Layout.topMargin: Theme.spacingSmall
        visible: content.listRows
        objectName: content.listRows ? "DevicesPopup.demo" : ""
        kind: "demo"
    }
    DeviceRow {
        Layout.fillWidth: true
        visible: !content.listRows   // -- WP DL3b --
        objectName: content.listRows ? "" : "DevicesPopup.demo"
        iconName: "play"
        title: qsTr("Demo DSPi")
        subtitle: qsTr("Simulated DSPi")
        current: DeviceModel.demo && DeviceModel.connected
        onClicked: DevicesController.chooseDemo()
    }
}
