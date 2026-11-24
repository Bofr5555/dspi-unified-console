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
// with a status dot and Close on hover, the DSPis on USB and the demo, and the
// hubs as one row that goes to Devices › Hubs. The rows keep the device list's
// names (DevicesPopup.open.<id>, .close.<id>, .usb.<serial>, .demo). Under them,
// what is done to the active device.
PopoverPanel {
    id: switcher
    objectName: "DeviceSwitcher"
    title: qsTr("Devices")
    // Manage…: the Devices section
    signal manage()

    // Several open: they are listed, and the active one is chosen there only
    readonly property bool several: DeviceRegistry.devices.length > 1
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 600
    width: Math.min(420, hostWidth - 2 * Theme.spacingLarge)
    height: Math.min(implicitHeight, hostHeight - y - Theme.spacingLarge)
    padding: Theme.spacingSmall

    // An open device's state as a dot (as Devices › This Computer)
    function stateColor(d) {
        return d.state === "lost" ? Theme.textMuted : d.state === "reconnecting" ? Theme.warning : Theme.success
    }
    // Devices › Hubs: through the shell's page host, else Manage…
    function hubs() {
        close()
        const host = find(parent, "PageHost")
        if (host) host.go("devices", "hubs")
        else manage()
    }
    function find(item, name) {
        if (!item) return null
        if (item.objectName === name) return item
        for (let i = 0; i < item.children.length; ++i) {
            const f = find(item.children[i], name)
            if (f) return f
        }
        return null
    }

    // A floating surface (DESIGN-LANGUAGE §5): panel tone, radius 8, a shadow,
    // and a 1 px edge in the dark scheme only
    background: Item {
        Repeater {
            model: 4
            Rectangle {
                required property int index
                readonly property real spread: Theme.shadowBlur * (index + 1) / 8
                x: -spread
                y: Theme.shadowOffset * (index + 1) / 4 - spread
                width: parent.width + 2 * spread
                height: parent.height + 2 * spread
                radius: Theme.radius + spread
                color: Qt.rgba(Theme.shadowColor.r, Theme.shadowColor.g, Theme.shadowColor.b, Theme.shadowColor.a / 6)
            }
        }
        Rectangle {
            anchors.fill: parent
            radius: Theme.radius
            color: Theme.panel
            border.width: Theme.dark ? 1 : 0
            border.color: Theme.separator
        }
        Accessible.role: Accessible.Dialog
        Accessible.name: switcher.title
    }

    component Dot: Rectangle {
        implicitWidth: 8
        implicitHeight: 8
        radius: 4
    }
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
                    delegate: ListRow {
                        id: openRow
                        required property var modelData
                        required property int index
                        Layout.fillWidth: true
                        objectName: "DevicesPopup.open." + modelData.id
                        rowIndex: index
                        activeFocusOnTab: true
                        iconName: modelData.transport === "usb" ? "usb"
                                  : modelData.transport === "demo" ? "play"
                                  : modelData.transport === "file" ? "file-text" : "network"
                        title: modelData.title
                        subtitle: modelData.state === "lost" ? qsTr("Not connected")
                                  : modelData.state === "reconnecting" ? qsTr("Reconnecting…")
                                  : modelData.busy ? modelData.job   // -- WP MDQT2c --
                                  : modelData.serial
                        selected: modelData.active
                        leading: Dot { color: switcher.stateColor(openRow.modelData) }
                        onClicked: DevicesController.chooseOpen(modelData.id)
                        onActivated: DevicesController.chooseOpen(modelData.id)
                        IconButton {
                            objectName: "DevicesPopup.close." + openRow.modelData.id
                            iconName: "x"
                            //: %1 is a device's name
                            toolTip: qsTr("Close %1").arg(openRow.modelData.title)
                            Accessible.name: toolTip
                            onClicked: DevicesController.closeOpen(openRow.modelData.id)
                        }
                    }
                }

                // ---- USB and the demo
                Heading {
                    Layout.topMargin: switcher.several ? Theme.spacing : 0
                    text: qsTr("USB")
                }
                Repeater {
                    model: DevicesController.usbDevices
                    delegate: ListRow {
                        id: usbRow
                        required property var modelData
                        required property int index
                        readonly property bool chosen: DeviceModel.connected && !DeviceModel.remote && !DeviceModel.demo
                                                       && DeviceModel.serial === modelData.serial
                        Layout.fillWidth: true
                        objectName: "DevicesPopup.usb." + modelData.serial
                        rowIndex: index
                        activeFocusOnTab: true
                        iconName: "usb"
                        title: qsTr("DSPi (%1)").arg(modelData.serial)
                        subtitle: qsTr("USB")
                        selected: chosen && !switcher.several
                        leading: Dot { color: usbRow.chosen ? Theme.success : "transparent" }
                        onClicked: DevicesController.chooseUsb(modelData.serial)
                        onActivated: DevicesController.chooseUsb(modelData.serial)
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
                ListRow {
                    id: demoRow
                    readonly property bool chosen: DeviceModel.demo && DeviceModel.connected
                    Layout.fillWidth: true
                    objectName: "DevicesPopup.demo"
                    activeFocusOnTab: true
                    iconName: "play"
                    title: qsTr("Demo DSPi")
                    subtitle: qsTr("Simulated DSPi")
                    selected: chosen && !switcher.several
                    leading: Dot { color: demoRow.chosen ? Theme.success : "transparent" }
                    onClicked: DevicesController.chooseDemo()
                    onActivated: DevicesController.chooseDemo()
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
                    onClicked: switcher.hubs()
                    onActivated: switcher.hubs()
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
