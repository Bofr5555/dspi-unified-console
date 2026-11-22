import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP PGQT -- The alink sender (DeviceInfoController.sender, the core's
// device kind AlinkSender): in place of the DSP screens, which it has none
// of, its device information, its audio link status and the firmware update.
Rectangle {
    id: page
    objectName: "SenderPage"
    color: Theme.window

    Flickable {
        anchors.fill: parent
        anchors.margins: Theme.spacingLarge
        contentHeight: column.implicitHeight
        clip: true
        ColumnLayout {
            id: column
            width: parent.width
            spacing: Theme.spacing

            Text {
                text: qsTr("Audio Link Sender")
                font: Theme.fontTitle
                color: Theme.text
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: qsTr("Serial %1 · firmware %2").arg(DeviceModel.serial)
                    .arg(DeviceModel.firmwareVersion.length ? DeviceModel.firmwareVersion : qsTr("unknown"))
                font: Theme.fontSmall
                color: Theme.textMuted
            }

            SettingsGroup {
                title: qsTr("Device")
                iconName: "cpu"
                DeviceInfoRows {}
            }

            SettingsGroup {
                objectName: "SenderPage.alink"
                title: qsTr("Audio Link")
                iconName: "radio"
                trailing: AppButton {
                    objectName: "SenderPage.refresh"
                    text: qsTr("Refresh")
                    enabled: !DeviceInfoController.alinkBusy
                    onClicked: DeviceInfoController.refreshAlink()
                }
                Text {
                    objectName: "SenderPage.alinkProblem"
                    Layout.fillWidth: true
                    visible: text.length > 0
                    textFormat: Text.PlainText
                    text: DeviceInfoController.alinkProblem
                    font: Theme.fontSmall
                    color: Theme.warning
                    wrapMode: Text.Wrap
                }
                Repeater {
                    model: DeviceInfoController.alinkRows
                    delegate: SettingsValueRow {
                        required property var modelData
                        title: modelData.label
                        value: modelData.value
                        mono: true
                    }
                }
            }

            SettingsGroup {
                title: qsTr("Firmware")
                iconName: "download"
                RowLayout {
                    Layout.fillWidth: true
                    Text {
                        Layout.fillWidth: true
                        text: qsTr("Update the sender's firmware with its own image.")
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                    AppButton {
                        objectName: "SenderPage.update"
                        text: qsTr("Firmware Update…")
                        onClicked: FirmwareController.openWindow()
                    }
                }
            }
        }
    }
}
