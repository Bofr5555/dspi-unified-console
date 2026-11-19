import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP PGQT -- The open device's protocol (DeviceInfoController): board,
// generation, protocol version, page list and extension envelope, read once
// per connection, and the notes under them. Settings > About and the alink
// sender's page show it.
ColumnLayout {
    id: rows
    objectName: "DeviceInfoRows"
    Layout.fillWidth: true
    spacing: 4

    Text {
        objectName: "DeviceInfoRows.none"
        Layout.fillWidth: true
        visible: !DeviceInfoController.known
        text: DeviceModel.connected ? qsTr("Reading the device…") : qsTr("No device is connected.")
        font: Theme.fontSmall
        color: Theme.textMuted
    }
    Repeater {
        model: DeviceInfoController.rows
        delegate: SettingsValueRow {
            required property var modelData
            objectName: "DeviceInfoRows." + modelData.key
            title: modelData.label
            value: modelData.value
            mono: modelData.key === "pages" || modelData.key === "extensions"
        }
    }
    Repeater {
        model: DeviceInfoController.notes
        delegate: Text {
            required property string modelData
            required property int index
            objectName: "DeviceInfoRows.note." + index
            Layout.fillWidth: true
            text: modelData
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }
}
