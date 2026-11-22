import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- Devices. -- WP LAY5b -- This Computer: the open devices, those on
// USB and the demo (DevicesLocal), and what is done to the active one. The hubs
// are the next tab's (DevicesHubs). ModernShell keeps the lists live while one
// of the two shows.
Item {
    id: page
    objectName: "DevicesSection"
    property bool shown: false
    property Item host: null

    Flickable {
        anchors.fill: parent
        contentHeight: column.implicitHeight + 2 * Theme.spacingLarge
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: AppScrollBar {}

        ColumnLayout {
            id: column
            x: (parent.width - width) / 2
            y: Theme.spacingLarge
            width: Math.min(640, parent.width - 2 * Theme.spacingLarge)
            spacing: Theme.spacing

            DevicesLocal { Layout.fillWidth: true }
            // -- WP QPI3 -- a DSPi the app sees but cannot open
            RowLayout {
                objectName: "DevicesSection.access"
                Layout.fillWidth: true
                visible: DeviceAccess.count > 0
                spacing: Theme.spacingSmall
                Text {
                    Layout.fillWidth: true
                    text: qsTr("A DSPi on USB was found that cannot be opened.")
                    font: Theme.fontSmall
                    color: Theme.warning
                    wrapMode: Text.Wrap
                }
                AppButton {
                    objectName: "DevicesSection.accessHelp"
                    text: qsTr("Help…")
                    toolTip: qsTr("Show how to give access")
                    onClicked: DeviceAccess.offer()
                }
            }
            SectionHeading {
                Layout.fillWidth: true
                Layout.topMargin: Theme.spacingSmall
                visible: DeviceModel.connected
                text: qsTr("The device shown")
            }
            Flow {
                Layout.fillWidth: true
                spacing: Theme.spacingSmall
                // -- WP LAY3b -- MenuTable's place device, as the switcher
                Repeater {
                    model: MenuTable.placeRows("device")[0].map(r => r.id)
                    AppButton {
                        required property string modelData
                        objectName: "DevicesSection." + modelData
                        visible: { const r = MenuTable.row(modelData); return !r.platformHidden || DeviceModel.platformName !== r.platformHidden }
                        // Except during a hub session (behaviour.md §3)
                        enabled: MenuTable.can(modelData) && (modelData !== "device.reconnect" || !DeviceModel.remote)
                        text: MenuTable.textOf(modelData, false)
                        onClicked: Shell.trigger(modelData)
                    }
                }
            }
            SectionHeading {
                Layout.fillWidth: true
                Layout.topMargin: Theme.spacingSmall
                text: qsTr("On the network")
            }
            AppButton {
                objectName: "DevicesSection.hubs"
                iconName: "wifi"
                text: qsTr("Hubs and Their Devices")
                onClicked: if (page.host) page.host.go("devices", "hubs")
            }
        }
    }
}
