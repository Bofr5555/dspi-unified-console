import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP18 -- Window > Mode... (docs/UI-MODES.md §5): Simple or Advanced,
// for this device or for every device on this console. It never writes to
// the device and asks for no PIN.
ToolWindow {
    id: window
    objectName: "ModeWindow"
    windowId: "mode"
    defaultWidth: 380
    defaultHeight: 190
    minimumWidth: 320
    minimumHeight: 170
    title: qsTr("Mode")

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacingLarge
        spacing: Theme.spacing

        AppTabBar {
            id: modes
            objectName: "ModeWindow.modes"
            Layout.fillWidth: true
            label: qsTr("Mode")
            currentIndex: ModeController.mode === "simple" ? 0 : 1
            Repeater {
                model: ModeController.modes
                AppTabButton {
                    required property var modelData
                    objectName: "ModeWindow.mode." + modelData.id
                    text: modelData.title
                    onClicked: ModeController.choose(modelData.id, everyDevice.checked)
                }
            }
        }
        AppCheckBox {
            id: everyDevice
            objectName: "ModeWindow.everyDevice"
            text: qsTr("Use for every device on this console")
            checked: !ModeController.rememberedForDevice
            Accessible.name: text
        }
        Text {
            Layout.fillWidth: true
            wrapMode: Text.Wrap
            font: Theme.fontSmall
            color: Theme.textMuted
            text: everyDevice.checked ? qsTr("Devices without a mode of their own open in this mode.")
                                      : qsTr("Remembered for this device.")
        }
        Item { Layout.fillHeight: true }
        AppButton {
            objectName: "ModeWindow.done"
            Layout.alignment: Qt.AlignRight
            text: qsTr("Done")
            primary: true
            onClicked: window.close()
        }
    }
}
