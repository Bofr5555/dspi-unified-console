import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The device switcher under the device tabs: every DSPi the app
// can open (DevicesContent, with the open ones first) and what is done to the
// active one. Device > Devices… and the tabs' "+" open it; ModernShell opens
// and closes it with DevicesController.listShown.
// -- WP DL5 -- A popover of ListRows (DESIGN-LANGUAGE §9, §15): no title and no
// Done (a press outside or Escape closes it); the open devices (with several)
// with a status dot and Close on hover, the DSPis on USB and the demo (INTC:
// DeviceListRow, the rows of Devices › This Computer), and the
// hubs as one row that goes to Devices › Hubs. The rows keep the device list's
// names (DevicesPopup.open.<id>, .close.<id>, .usb.<serial>, .demo). Under them,
// what is done to the active device.
PopoverPanel {
    id: switcher
    objectName: "DeviceSwitcher"
    title: qsTr("Devices")
    // Manage…: the Devices section
    signal manage()
    // -- INTC (DL5 fold) -- the hubs' row: Devices › Hubs (ModernShell goes there)
    signal hubs()

    // Several open: they are listed, and the active one is chosen there only
    readonly property bool several: DeviceRegistry.devices.length > 1
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 600
    width: Math.min(420, hostWidth - 2 * Theme.spacingLarge)
    height: Math.min(implicitHeight, hostHeight - y - Theme.spacingLarge)
    padding: Theme.spacingSmall

    // A floating surface (DESIGN-LANGUAGE §5): PopoverPanel's FloatingBackground (INTC)

    component Heading: SectionHeading {
        Layout.fillWidth: true
        Layout.leftMargin: Theme.spacingSmall
        Layout.topMargin: Theme.spacingSmall
    }

    contentItem: ColumnLayout {
        spacing: Theme.spacingSmall

        Flickable {
            id: flick
            Layout.fillWidth: true
            Layout.fillHeight: true
            implicitHeight: rows.implicitHeight
            contentHeight: rows.implicitHeight
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}

            // The keyboard's row (a ListRow draws its ring only in a ListView)
            Rectangle {
                readonly property Item target: {
                    const f = switcher.contentItem ? switcher.contentItem.Window.activeFocusItem : null
                    return f && f.parent === rows && f.rowIndex !== undefined ? f : null
                }
                z: 1
                visible: target !== null
                x: target ? target.x : 0
                y: target ? target.y : 0
                width: target ? target.width : 0
                height: target ? target.height : 0
                radius: Theme.radiusSmall
                color: "transparent"
                border.width: Theme.focusRingWidth
                border.color: Theme.focusRing
            }
            ColumnLayout {
                id: rows
                width: flick.width
                spacing: 0

                // ---- Open devices: only when several are
                Heading {
                    objectName: "DevicesPopup.openHeading"
                    Layout.topMargin: 0
                    visible: switcher.several
                    text: qsTr("Open devices")
                }
                Repeater {
                    model: switcher.several ? DeviceRegistry.devices : []
                    delegate: DeviceListRow {   // -- INTC -- Devices › This Computer's row
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true
                        activeFocusOnTab: true
                        kind: "open"
                        device: modelData
                        rowIndex: index
                    }
                }

                // ---- USB and the demo
                Heading {
                    Layout.topMargin: switcher.several ? Theme.spacing : 0
                    text: qsTr("USB")
                }
                Repeater {
                    model: DevicesController.usbDevices
                    delegate: DeviceListRow {   // -- INTC --
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true
                        activeFocusOnTab: true
                        kind: "usb"
                        device: modelData
                        rowIndex: index
                        markChosen: !switcher.several
                    }
                }
                Text {
                    Layout.fillWidth: true
                    Layout.leftMargin: Theme.spacingSmall
                    Layout.bottomMargin: 4
                    visible: DevicesController.usbDevices.length === 0
                    text: qsTr("No DSPi on USB")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
                DeviceListRow {   // -- INTC --
                    Layout.fillWidth: true
                    activeFocusOnTab: true
                    kind: "demo"
                    markChosen: !switcher.several
                }

                // ---- The hubs: their page
                Heading {
                    Layout.topMargin: Theme.spacing
                    text: qsTr("DSPi Link")
                }
                ListRow {
                    objectName: "DeviceSwitcher.hubs"
                    activeFocusOnTab: true
                    Layout.fillWidth: true
                    iconName: "wifi"
                    title: qsTr("Hubs and Their Devices…")
                    subtitle: LinkController.hubName.length ? LinkController.hubName + ": " + LinkController.phaseText : LinkController.phaseText
                    onClicked: { switcher.close(); switcher.hubs() }
                    onActivated: { switcher.close(); switcher.hubs() }
                }
            }
        }

        // ---- What is done to the active device
        Flow {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.spacingSmall
            Layout.rightMargin: Theme.spacingSmall
            Layout.bottomMargin: 4
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
