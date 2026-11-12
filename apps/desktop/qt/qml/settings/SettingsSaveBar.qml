import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP11 -- The bar along the bottom of every Settings page while device
// settings are held (the Mac's SettingsSaveBar; behaviour.md §8): one Save and
// one Revert for whatever is pending, on whichever page shows. A Save the
// device partly refused leaves it up with an orange line naming what was not
// taken (QD31); a saved latency the device has not applied yet keeps it up too
// (behaviour.md §16).
Rectangle {
    id: bar
    objectName: "SaveBar"
    readonly property var hw: HardwareController

    visible: hw.pending || hw.saveFailure.length > 0
    implicitHeight: visible ? content.implicitHeight + 2 * Theme.spacingSmall : 0
    color: Theme.sidebar

    Rectangle {
        anchors.top: parent.top
        width: parent.width
        height: 1
        color: Theme.separator
    }

    RowLayout {
        id: content
        anchors.fill: parent
        anchors.leftMargin: Theme.spacingLarge
        anchors.rightMargin: Theme.spacingLarge
        anchors.topMargin: Theme.spacingSmall
        anchors.bottomMargin: Theme.spacingSmall
        spacing: Theme.spacingSmall

        Icon {
            Layout.alignment: Qt.AlignTop
            name: "circle-alert"
            size: 14
            color: Theme.warning
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 1
            Text {
                objectName: "SaveBar.title"
                text: qsTr("Unsaved changes")
                font: Theme.fontSmall
                color: Theme.textMuted
            }
            Text {
                objectName: "SaveBar.subtitle"
                Layout.fillWidth: true
                // Control Surfaces changes are live already: say so when that is all
                text: bar.hw.liveOnly ? qsTr("Your controls are live now; saving keeps them across a reboot.")
                                      : qsTr("Saving writes these settings to the device's flash.")
                font: Theme.fontCaption
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            Text {
                objectName: "SaveBar.failure"
                Layout.fillWidth: true
                visible: text.length > 0
                text: bar.hw.saveFailure
                font: Theme.fontSmall
                color: Theme.warning
                wrapMode: Text.Wrap
                Accessible.role: Accessible.AlertMessage
                Accessible.name: text
            }
            Text {
                objectName: "SaveBar.held"
                Layout.fillWidth: true
                visible: text.length > 0
                text: bar.hw.heldNote
                font: Theme.fontSmall
                color: Theme.warning
                wrapMode: Text.Wrap
                Accessible.role: Accessible.AlertMessage
                Accessible.name: text
            }
        }
        Icon {
            visible: bar.hw.busy
            name: "loader-circle"
            size: 14
            color: Theme.textMuted
            accessibleName: qsTr("Saving")
        }
        AppButton {
            objectName: "SaveBar.revert"
            text: qsTr("Revert")
            // The drafts are the app's: Revert works offline, except for a
            // live preview only the device can put back
            enabled: !bar.hw.busy && !(bar.hw.liveOnly && !DeviceModel.connected)
            onClicked: bar.hw.revert()
        }
        AppButton {
            objectName: "SaveBar.save"
            text: qsTr("Save")
            primary: true
            enabled: !bar.hw.busy && bar.hw.writable && bar.hw.pending
            onClicked: bar.hw.save()
        }
    }
}
