import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- Devices. -- WP LAY5b -- This Computer: the open devices, those on
// USB and the demo (DevicesLocal), and what is done to the active one. The hubs
// are the next tab's (DevicesHubs). ModernShell keeps the lists live while one
// of the two shows. -- WP DL3b -- A 760 px column at the left
// (DESIGN-LANGUAGE §3), the devices as ListRows, sections 24 apart.
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
            x: Theme.spacing
            y: Theme.spacing
            width: Math.min(Theme.contentMaxWidth, parent.width - 2 * Theme.spacing)
            spacing: Theme.spacingSmall

            // -- WP QPI3 -- a DSPi the app sees but cannot open (-- WP DL3b -- a notice, on top)
            NoticeRow {
                objectName: "DevicesSection.access"
                Layout.fillWidth: true
                visible: DeviceAccess.count > 0
                kind: "warning"
                text: qsTr("A DSPi on USB was found that cannot be opened.")
                AppButton {
                    objectName: "DevicesSection.accessHelp"
                    text: qsTr("Help…")
                    toolTip: qsTr("Show how to give access")
                    onClicked: DeviceAccess.offer()
                }
            }
            DevicesLocal {
                Layout.fillWidth: true
                listRows: true   // -- WP DL3b --
            }
            SectionHeading {
                Layout.fillWidth: true
                Layout.topMargin: Theme.spacingSection - Theme.spacingSmall   // -- WP DL3b --
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
                Layout.topMargin: Theme.spacingSection - Theme.spacingSmall   // -- WP DL3b --
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
