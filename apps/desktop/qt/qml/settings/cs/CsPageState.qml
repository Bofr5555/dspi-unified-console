import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- What a Control page shows before it has the device's tables
// (the Mac's awaitingCapsSection and disconnectedSection), or when reading
// them failed. Visible only then.
ColumnLayout {
    id: state
    readonly property bool reading: CsController.connected && !CsController.loaded && CsController.loadError.length === 0
    visible: !CsController.loaded
    Layout.fillWidth: true
    spacing: 10

    RowLayout {
        visible: state.reading
        spacing: 8
        Icon { name: "loader-circle"; size: 14; color: Theme.textMuted }
        Text {
            objectName: "Cs.reading"
            text: qsTr("Reading control-surface capabilities from the device...")
            font: Theme.fontSmall
            color: Theme.textMuted
        }
    }
    RowLayout {
        visible: CsController.connected && CsController.loadError.length > 0
        Layout.fillWidth: true
        spacing: 8
        Icon { name: "triangle-alert"; size: 12; color: Theme.warning }
        Text {
            objectName: "Cs.loadError"
            Layout.fillWidth: true
            text: CsController.loadError
            font: Theme.fontSmall
            color: Theme.warning
            wrapMode: Text.Wrap
        }
        AppButton {
            text: qsTr("Try Again")
            onClicked: CsController.fetch()
        }
    }
    CsEmptyState {
        visible: !CsController.connected
        iconName: "unplug"
        title: qsTr("No Device Connected")
        text: qsTr("Control surfaces are stored on the device. Connect a DSPi to view and configure its wired controls.")
    }
}
