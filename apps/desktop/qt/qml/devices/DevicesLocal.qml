import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5b -- The local part of the device list (split from DevicesContent):
// the open devices when several are, the DSPis on USB, and the demo. The list
// shows it (DevicesContent), and Modern's Devices > This Computer.
// -- WP DL3b -- `listRows`: the shared rows (ListRow, DESIGN-LANGUAGE §9) for
// Modern's page, a status dot on each open device and its Close on hover;
// the list in a popover keeps its DeviceRows.
ColumnLayout {
    id: content
    spacing: 4
    property bool listRows: false

    // -- WP DL3b -- an open device's state as a dot
    function stateColor(d) {
        return d.state === "lost" ? Theme.textMuted : d.state === "reconnecting" ? Theme.warning : Theme.success
    }
    component Dot: Rectangle {
        implicitWidth: 8
        implicitHeight: 8
        radius: 4
    }

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

    // -- WP DL3b --
    Repeater {
        model: DeviceRegistry.devices.length > 1 && content.listRows ? DeviceRegistry.devices : []
        delegate: ListRow {
            id: openItem
            required property var modelData
            required property int index
            Layout.fillWidth: true
            objectName: "DevicesPopup.open." + modelData.id
            rowIndex: index
            iconName: modelData.transport === "usb" ? "usb"
                      : modelData.transport === "demo" ? "play"
                      : modelData.transport === "file" ? "file-text" : "network"
            title: modelData.title
            subtitle: modelData.state === "lost" ? qsTr("Not connected")
                      : modelData.state === "reconnecting" ? qsTr("Reconnecting…")
                      : modelData.busy ? modelData.job
                      : modelData.serial
            selected: modelData.active
            leading: Dot { color: content.stateColor(openItem.modelData) }
            onClicked: DevicesController.chooseOpen(modelData.id)
            onActivated: DevicesController.chooseOpen(modelData.id)
            IconButton {
                objectName: "DevicesPopup.close." + openItem.modelData.id
                iconName: "x"
                //: %1 is a device's name
                toolTip: qsTr("Close %1").arg(openItem.modelData.title)
                Accessible.name: toolTip
                onClicked: DevicesController.closeOpen(openItem.modelData.id)
            }
        }
    }

    // ---- USB and the demo
    SectionHeading {
        Layout.fillWidth: true
        Layout.topMargin: content.listRows ? Theme.spacing : 0   // -- WP DL3b --
        text: qsTr("USB")
    }
    // -- WP DL3b --
    Repeater {
        model: content.listRows ? DevicesController.usbDevices : []
        delegate: ListRow {
            id: usbItem
            required property var modelData
            required property int index
            Layout.fillWidth: true
            objectName: "DevicesPopup.usb." + modelData.serial
            rowIndex: index
            iconName: "usb"
            title: qsTr("DSPi (%1)").arg(modelData.serial)
            subtitle: qsTr("USB")
            readonly property bool chosen: DeviceModel.connected && !DeviceModel.remote && !DeviceModel.demo
                                            && DeviceModel.serial === modelData.serial
            selected: chosen
            leading: Dot { color: usbItem.chosen ? Theme.success : "transparent" }
            onClicked: DevicesController.chooseUsb(modelData.serial)
            onActivated: DevicesController.chooseUsb(modelData.serial)
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
    // -- WP DL3b --
    ListRow {
        id: demoItem
        Layout.fillWidth: true
        Layout.topMargin: Theme.spacingSmall
        visible: content.listRows
        objectName: content.listRows ? "DevicesPopup.demo" : ""
        iconName: "play"
        title: qsTr("Demo DSPi")
        subtitle: qsTr("Simulated DSPi")
        readonly property bool chosen: DeviceModel.demo && DeviceModel.connected
        selected: chosen
        leading: Dot { color: demoItem.chosen ? Theme.success : "transparent" }
        onClicked: DevicesController.chooseDemo()
        onActivated: DevicesController.chooseDemo()
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
