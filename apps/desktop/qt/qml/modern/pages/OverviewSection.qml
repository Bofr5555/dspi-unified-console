import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- Overview: the dashboard. Choosing a card's channel opens it on
// the Equaliser page. -- WP LAY5a -- Over the cards the device, its effects and
// its connection (OverviewSummary); without a device, the way to one.
// -- WP DL6 -- Modern's own cards (OverviewChannelCard), one per channel;
// Classic's Dashboard is not used here.
Item {
    id: page
    objectName: "OverviewSection"
    property bool shown: false
    property Item host: null

    // The device whose overview this is: another one's selection (kept per
    // device) is no card that was chosen here
    property string device: ""
    readonly property string activeDevice: DeviceRegistry.activeId
    onActiveDeviceChanged: Qt.callLater(selectNone)

    // Nothing is selected here: the cards choose
    onShownChanged: if (shown) selectNone()
    function selectNone() {
        if (!shown) return
        device = DeviceRegistry.activeId
        SidebarController.select(-1)
    }
    Connections {
        target: SidebarController
        enabled: page.shown
        function onSelectionChanged() {
            if (SidebarController.selectedChannel < 0) return
            if (page.device !== DeviceRegistry.activeId) Qt.callLater(page.selectNone)
            else if (page.host) page.host.go("equaliser", "peq")
        }
    }

    // -- WP DL6 -- No device: the way to one
    EmptyState {
        objectName: "OverviewSection.empty"
        anchors.fill: parent
        visible: !DeviceModel.connected
        iconName: "plug"
        title: qsTr("No Device Connected")
        text: qsTr("Open a DSPi, a hub's device or the demo to see its channels here.")
        actionText: qsTr("Choose a Device…")
        actionPrimary: true
        onAction: if (page.host) page.host.go("devices", "devices")
    }

    // -- WP DL6 -- The device's tiles, then one card per channel (the inputs,
    // then the outputs the device has on) in place of Classic's band grids.
    // "Dashboard" is Classic's Overview's name, kept for the smoke driver.
    Flickable {
        id: flick
        objectName: "Dashboard"
        anchors.fill: parent
        visible: DeviceModel.connected
        contentWidth: width
        contentHeight: content.implicitHeight + 2 * Theme.spacing
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: AppScrollBar {}

        // The inputs in use, each a card: routed somewhere, or with bands or a
        // preamp set (Classic's cards have inputs 1 and 2 only, while a unit of
        // a multichannel set is fed by others); the rest named in one line.
        // Then the outputs of DashboardController.cards (the ones that are on).
        function inUse(i) {
            for (let o = 0; o < ChannelController.numOutputs; ++o) if (ChannelController.route(i, o).enabled === true) return true
            for (let b = 0; b < 10; ++b) if (ChannelController.band(i, b, false).type !== 0) return true
            return ChannelController.preamp(i) !== 0
        }
        readonly property var inputs: {
            const list = []
            if (ChannelController.revision < 0) return list
            for (let i = 0; i < ChannelController.numInputs; ++i) if (inUse(i)) list.push({ channel: i, output: false })
            return list
        }
        readonly property var unused: {
            const names = []
            if (ChannelController.revision < 0 || DeviceModel.revision < 0) return names
            for (let i = 0; i < ChannelController.numInputs; ++i) {
                if (inUse(i)) continue
                const row = DeviceModel.channels.rowOfChannel(i)
                if (row >= 0) names.push(DeviceModel.channels.get(row, "label") || "")
            }
            return names
        }
        readonly property var outputs: {
            const list = []
            for (const c of DashboardController.cards) for (const col of c.columns || []) if (col.output) list.push(col)
            return list
        }
        readonly property int columns: Math.max(1, Math.floor((content.width + Theme.spacing) / (260 + Theme.spacing)))

        ColumnLayout {
            id: content
            x: Theme.spacing
            y: Theme.spacing
            width: flick.width - 2 * Theme.spacing
            spacing: Theme.spacing

            OverviewSummary {
                Layout.fillWidth: true
                host: page.host
            }
            // -- WP QP9 -- The spectrum as bars, when it is shown here
            SpectrumStrip {
                place: "dashboard"
                Layout.fillWidth: true
            }
            Repeater {
                model: [{ key: "inputs", title: qsTr("INPUTS"), list: flick.inputs },
                        { key: "outputs", title: qsTr("OUTPUTS"), list: flick.outputs }]
                ColumnLayout {
                    id: group
                    required property var modelData
                    required property int index
                    objectName: "OverviewSection." + modelData.key
                    Layout.fillWidth: true
                    Layout.topMargin: Theme.spacingSection - Theme.spacing
                    visible: modelData.list.length > 0 || (modelData.key === "inputs" && flick.unused.length > 0)
                    spacing: Theme.spacingSmall
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: Theme.spacingSmall
                        Text {
                            Layout.fillWidth: true
                            text: group.modelData.title
                            font: Theme.fontHeading
                            color: Theme.textMuted
                            Accessible.role: Accessible.Heading
                            Accessible.name: text
                        }
                        // The meters' state, by the first heading so no card moves
                        StatusChip {
                            objectName: "OverviewSection.stale"
                            visible: group.index === 0 && DashboardController.stale
                            kind: "warning"
                            text: qsTr("Meters stale")
                            toolTip: qsTr("No meter reading has come from the device for a while. The meters show the last values read.")
                        }
                    }
                    Text {
                        objectName: "OverviewSection.unused"
                        Layout.fillWidth: true
                        visible: group.modelData.key === "inputs" && flick.unused.length > 0
                        textFormat: Text.PlainText
                        //: %1 lists inputs that feed no output and have nothing set ("L, R, In 7")
                        text: qsTr("Not in use: %1").arg(flick.unused.join(", "))
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                    GridLayout {
                        Layout.fillWidth: true
                        visible: group.modelData.list.length > 0
                        columns: flick.columns
                        columnSpacing: Theme.spacing
                        rowSpacing: Theme.spacing
                        Repeater {
                            model: group.modelData.list
                            OverviewChannelCard {
                                required property var modelData
                                entry: modelData
                                Layout.fillWidth: true
                                Layout.preferredWidth: 1
                                Layout.alignment: Qt.AlignTop
                                onChosen: channel => SidebarController.select(channel)
                            }
                        }
                        // Fill a short last row, so the cards keep their width
                        Repeater {
                            model: Math.max(0, flick.columns - group.modelData.list.length)
                            Item {
                                Layout.fillWidth: true
                                Layout.preferredWidth: 1
                            }
                        }
                    }
                }
            }
        }
    }

    Accessible.role: Accessible.Pane
    Accessible.name: qsTr("Overview")
}
