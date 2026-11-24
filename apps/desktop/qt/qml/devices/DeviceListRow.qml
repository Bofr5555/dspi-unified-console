import QtQuick
import UConsole

// -- INTC (DL5 fold) -- One DSPi as a ListRow (DESIGN-LANGUAGE §9): the row
// Devices › This Computer (DevicesLocal with listRows) and Modern's device
// switcher share. `kind`:
//   "open"  an open device (`device`: a DeviceRegistry.devices entry): its
//           transport's icon, a status dot, what it is doing, Close on hover
//   "usb"   a DSPi on USB (`device`: a DevicesController.usbDevices entry)
//   "demo"  the demo DSPi
// USB and the demo get a green dot while they are the one shown, and the
// selection tone too unless `markChosen` is false (a list that marks the
// shown device among its open ones). A click or Enter chooses it. The rows
// keep the device list's names: DevicesPopup.open.<id>, DevicesPopup.close.<id>,
// DevicesPopup.usb.<serial>, DevicesPopup.demo.
ListRow {
    id: row
    property string kind: "open"
    property var device: ({})
    property bool markChosen: true

    readonly property bool open: kind === "open"
    readonly property bool chosen: open ? device.active === true
        : kind === "usb" ? DeviceModel.connected && !DeviceModel.remote && !DeviceModel.demo
                           && DeviceModel.serial === device.serial
        : DeviceModel.demo && DeviceModel.connected

    function choose() {
        if (open) DevicesController.chooseOpen(device.id)
        else if (kind === "usb") DevicesController.chooseUsb(device.serial)
        else DevicesController.chooseDemo()
    }

    objectName: open ? "DevicesPopup.open." + device.id
              : kind === "usb" ? "DevicesPopup.usb." + device.serial : "DevicesPopup.demo"
    iconName: kind === "usb" ? "usb" : kind === "demo" ? "play"
            : device.transport === "usb" ? "usb"
            : device.transport === "demo" ? "play"
            : device.transport === "file" ? "file-text" : "network"
    title: open ? device.title
         : kind === "usb" ? qsTr("DSPi (%1)").arg(device.serial) : qsTr("Demo DSPi")
    subtitle: kind === "usb" ? qsTr("USB")
            : kind === "demo" ? qsTr("Simulated DSPi")
            : device.state === "lost" ? qsTr("Not connected")
            : device.state === "reconnecting" ? qsTr("Reconnecting…")
            : device.busy ? device.job   // -- WP MDQT2c --
            : device.serial
    selected: open ? chosen : chosen && markChosen
    leading: Rectangle {
        implicitWidth: 8
        implicitHeight: 8
        radius: 4
        color: !row.open ? (row.chosen ? Theme.success : "transparent")
             : row.device.state === "lost" ? Theme.textMuted
             : row.device.state === "reconnecting" ? Theme.warning : Theme.success
    }
    onClicked: choose()
    onActivated: choose()

    IconButton {
        visible: row.open
        objectName: row.open ? "DevicesPopup.close." + row.device.id : ""
        iconName: "x"
        //: %1 is a device's name
        toolTip: qsTr("Close %1").arg(row.device.title)
        Accessible.name: toolTip
        onClicked: DevicesController.closeOpen(row.device.id)
    }
}
