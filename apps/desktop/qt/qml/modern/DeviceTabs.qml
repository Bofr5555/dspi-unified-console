import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The open devices as tabs (MPLAN 1.2 item 7), from
// DeviceRegistry.devices: the state's dot, the name and the transport, a close
// button, and "+" for the device switcher. One device is a tab like any other;
// with none a tab says so. Choosing a tab makes its device the active one;
// the active tab itself opens the switcher. `vertical` is the rail's narrow
// column of them (LayoutController.arrangement.deviceTabs).
Item {
    id: tabs
    objectName: "DeviceTabs"
    property bool vertical: false
    readonly property var rows: DeviceRegistry.devices.length > 0 ? DeviceRegistry.devices : [none]
    readonly property var none: ({ id: "", title: "", transport: "", state: "none", active: true, clip: false, serial: "" })
    readonly property real tabWidth: vertical ? width : Math.max(110, Math.min(230, (width - 34) / rows.length))

    implicitWidth: vertical ? 64 : 230 * rows.length + 34
    implicitHeight: vertical ? column.implicitHeight : 36
    Accessible.role: Accessible.PageTabList
    Accessible.name: qsTr("Devices")

    function transportName(row) {
        if (row.transport === "usb") return qsTr("USB")
        if (row.transport === "demo") return qsTr("Demo")
        if (row.transport === "file") return qsTr("File")
        if (row.transport === "hub") return qsTr("Hub")
        if (row.transport === "link") return qsTr("DSPi Link")
        return ""
    }

    // Across the top bar
    RowLayout {
        anchors.fill: parent
        visible: !tabs.vertical
        spacing: 2
        ListView {
            id: strip
            objectName: tabs.objectName + ".strip"
            Layout.fillHeight: true
            Layout.preferredWidth: Math.min(contentWidth, tabs.width - 34)
            orientation: ListView.Horizontal
            boundsBehavior: Flickable.StopAtBounds
            clip: true
            spacing: 2
            model: tabs.vertical ? [] : tabs.rows
            currentIndex: tabs.rows.findIndex(r => r.active)
            highlightFollowsCurrentItem: true
            highlightMoveDuration: 0
            interactive: contentWidth > width
            delegate: DeviceTab {
                width: tabs.tabWidth - 2
                height: ListView.view ? ListView.view.height : 36
            }
        }
        Loader {
            active: !tabs.vertical
            sourceComponent: AddButton {}
        }
        Item { Layout.fillWidth: true }
    }

    // At the top of the rail
    Column {
        id: column
        width: parent.width
        visible: tabs.vertical
        spacing: 2
        Repeater {
            model: tabs.vertical ? tabs.rows : []
            delegate: DeviceTab {
                width: column.width
                height: 40
            }
        }
        Loader {
            anchors.horizontalCenter: parent.horizontalCenter
            active: tabs.vertical
            sourceComponent: AddButton {}
        }
    }

    component AddButton: IconButton {
        objectName: tabs.objectName + ".add"
        iconName: "plus"
        toolTip: qsTr("Open a Device…")
        onClicked: Shell.trigger("device.devices")
    }

    component DeviceTab: AbstractButton {
        id: tab
        required property var modelData
        readonly property bool active: modelData.active === true
        readonly property bool none: modelData.state === "none"
        // -- WP SECQT -- the active hub session is not encrypted: in words, not colour alone
        readonly property bool unsecured: active && DeviceModel.remote && LinkController.unsecured && !FilesController.offline
        readonly property string name: none ? (FilesController.offline ? FilesController.offlineFile : qsTr("No Device"))
                                            : modelData.title
        // -- WP INT3 -- a job on this device (MDQT2c) and what keeps it from closing (MDQT2b)
        readonly property bool busy: !none && modelData.busy === true
        readonly property string pinned: {
            FirmwareController.runDevice; CoreVoltageController.calibrating   // re-read when either changes
            return none ? "" : DevicesController.pinnedText(modelData.id)
        }
        // What the tab shows, for the smoke driver
        readonly property string shownName: name + (unsecured ? " (" + LinkController.unsecuredLabel + ")" : "")
        readonly property string detail: none ? qsTr("Choose a device")
            : modelData.state === "lost" ? qsTr("Not connected")
            : modelData.state === "reconnecting" ? qsTr("Reconnecting…")
            : busy && modelData.job ? modelData.job   // -- WP INT3 --
            : tabs.transportName(modelData)
              //: %1 is the firmware's version
              + (active && DeviceModel.firmwareVersion.length ? " · " + qsTr("firmware %1").arg(DeviceModel.firmwareVersion) : "")
        objectName: tabs.objectName + ".tab." + (none ? "none" : modelData.id)
        checked: active
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        onClicked: {
            if (active) Shell.trigger("device.devices")
            else DevicesController.chooseOpen(modelData.id)
        }

        contentItem: Item {
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 8
                anchors.rightMargin: 2
                visible: !tabs.vertical
                spacing: 6
                StateDot { row: tab.modelData; busy: tab.busy }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 0
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 4
                        NameText {
                            Layout.fillWidth: true
                            text: tab.name
                            active: tab.active
                        }
                        Text {
                            objectName: tabs.objectName + ".unsecured"
                            visible: tab.unsecured
                            text: "(" + LinkController.unsecuredLabel + ")"
                            font: Theme.fontSmall
                            color: Theme.dangerText
                        }
                    }
                    Text {
                        Layout.fillWidth: true
                        text: tab.detail
                        textFormat: Text.PlainText
                        font: Theme.fontCaption
                        color: Theme.textMuted
                        elide: Text.ElideRight
                    }
                }
                UnsecuredInfoButton {
                    visible: tab.unsecured
                }
                IconButton {
                    objectName: tabs.objectName + ".access"
                    // -- WP QPI3 -- a DSPi the app sees but cannot open
                    visible: tab.none && DeviceAccess.count > 0
                    iconName: "key-round"
                    iconSize: 12
                    toolTip: qsTr("A DSPi was found that cannot be opened. Show how to give access.")
                    onClicked: DeviceAccess.offer()
                }
                IconButton {
                    objectName: tabs.objectName + ".close." + tab.modelData.id
                    visible: !tab.none
                    iconName: "x"
                    iconSize: 12
                    enabled: tab.pinned === ""   // -- WP INT3 -- closeOpen refuses it anyway, with a notice
                    //: %1 is a device's name
                    toolTip: tab.pinned !== "" ? tab.pinned : qsTr("Close %1").arg(tab.name)
                    onClicked: DevicesController.closeOpen(tab.modelData.id)
                }
            }
            Column {
                anchors.centerIn: parent
                visible: tabs.vertical
                spacing: 3
                StateDot {
                    anchors.horizontalCenter: parent.horizontalCenter
                    row: tab.modelData
                    busy: tab.busy
                }
                NameText {
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: Math.min(implicitWidth, tab.width - 4)
                    font: Theme.fontCaption
                    text: tab.name
                    active: tab.active
                }
            }
        }
        background: Rectangle {
            radius: Theme.radiusSmall
            color: tab.unsecured ? Theme.bannerFill("error")
                 : tab.active ? Theme.control : tab.hovered ? Theme.controlHover : "transparent"
            border.width: tab.visualFocus ? 2 : tab.unsecured ? 1 : 0
            border.color: tab.visualFocus ? Theme.focusRing : Theme.danger
        }

        // -- WP AIVIS -- an agent changed something on it while another is shown
        AiTouched {
            objectName: tabs.objectName + ".touched." + tab.modelData.id
            device: tab.none || tab.active ? "-" : tab.modelData.id
        }
        // Right-click reconnects, as Classic's device button does, and Close
        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: point => tabMenu.popupFor({}, tab, point.position.x, point.position.y)
        }
        // -- WP LAY3b -- from MenuTable; a pinned device's Close says why not
        ActionMenu {
            id: tabMenu
            objectName: tabs.objectName + ".menu." + tab.modelData.id
            place: "device"
            handlers: ({
                "device.identify": { enabled: () => tab.active },
                // Except during a hub session (behaviour.md §3)
                "device.reconnect": { enabled: () => tab.active && !DeviceModel.remote },
                "device.firmwareUpdate": { enabled: () => tab.active },
                "device.close": {
                    enabled: () => !tab.none,
                    why: () => tab.pinned,
                    run: () => DevicesController.closeOpen(tab.modelData.id)
                }
            })
        }
        HelpTip {
            visible: tab.hovered
            text: [tab.shownName, tab.detail,
                   tab.active && DeviceModel.remote ? LinkController.sessionTip : tab.modelData.serial,
                   tab.pinned].filter(t => t && t.length).join("\n")   // -- WP INT3 -- pinned
        }
        Accessible.role: Accessible.PageTab
        Accessible.name: shownName
        Accessible.description: detail
        Accessible.selected: active
        Accessible.onPressAction: tab.clicked()
    }

    component StateDot: Rectangle {
        required property var row
        property bool busy: false   // -- WP INT3 -- a ring while a job runs on the device
        implicitWidth: 8
        implicitHeight: 8
        radius: 4
        color: row.state === "live" ? (row.clip ? Theme.danger : Theme.connected)
             : row.state === "reconnecting" ? Theme.warning
             : row.state === "none" ? Theme.offline : Theme.disconnected
        border.width: busy ? 2 : 0
        border.color: Theme.accent
        Rectangle {
            visible: parent.busy
            anchors.centerIn: parent
            width: parent.width + 6
            height: width
            radius: width / 2
            color: "transparent"
            border.width: 1
            border.color: Theme.accent
        }
    }
    component NameText: Text {
        property bool active: false
        // A device's name is the device's text, never markup
        textFormat: Text.PlainText
        font: Theme.fontBody
        color: active ? Theme.text : Theme.textMuted
        elide: Text.ElideRight
    }
}
