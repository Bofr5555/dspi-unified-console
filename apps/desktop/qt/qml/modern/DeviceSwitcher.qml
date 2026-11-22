import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The device switcher under the device tabs: every DSPi the app
// can open (DevicesContent, with the open ones first) and what is done to the
// active one. Device > Devices… and the tabs' "+" open it; ModernShell opens
// and closes it with DevicesController.listShown.
PopoverPanel {
    id: switcher
    objectName: "DeviceSwitcher"
    title: qsTr("Devices")
    // Manage…: the Devices section
    signal manage()

    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 600
    width: Math.min(460, hostWidth - 2 * Theme.spacingLarge)
    height: Math.min(implicitHeight, hostHeight - y - Theme.spacingLarge)

    onOpened: content.takeFocus()

    contentItem: ColumnLayout {
        spacing: Theme.spacingSmall
        DevicesContent {
            id: content
            title: switcher.title
            Layout.fillHeight: true
            onDone: switcher.close()
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }
        Flow {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            // -- WP LAY3b -- MenuTable's place device, as in a tab's menu
            Repeater {
                model: MenuTable.placeRows("device")[0].map(r => r.id)
                ActionButton {
                    required property string modelData
                    actionId: modelData
                    visible: { const r = MenuTable.row(actionId); return !r.platformHidden || DeviceModel.platformName !== r.platformHidden }
                    // Except during a hub session (behaviour.md §3)
                    enabled: MenuTable.can(actionId) && (actionId !== "device.reconnect" || !DeviceModel.remote)
                }
            }
            AppButton {
                objectName: "DeviceSwitcher.manage"
                text: qsTr("Manage…")
                onClicked: switcher.manage()
            }
        }
    }

    component ActionButton: AppButton {
        required property string actionId
        objectName: "DeviceSwitcher." + actionId
        text: MenuTable.textOf(actionId, false)
        onClicked: {
            switcher.close()
            Shell.trigger(actionId)
        }
    }
}
