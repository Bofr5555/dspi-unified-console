import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- What the Equaliser's pages share (MODLAY 2): the channel strip
// over the channels and the page's own content, and the inspector at the
// right. One channel is always selected here (-- WP LAY3 --).
Item {
    id: page
    property bool shown: false
    property Item host: null
    // The page's own content, beside the channels; and the inspector's
    default property alias content: centre.data
    property Component inspector: null
    property bool channelsShown: true
    readonly property alias inspectorItem: pane

    readonly property int channel: SidebarController.selectedChannel
    readonly property bool isOutput: info.isOutput
    property int lastChannel: -1
    property int renamingChannel: -1

    objectName: "EqualiserSection"

    ChannelInfo { id: info; channel: page.channel }

    function selectOne() {
        if (!shown || !DeviceModel.connected || SidebarController.selectedChannel >= 0) return
        const last = lastChannel >= 0 && DeviceModel.channels.rowOfChannel(lastChannel) >= 0
        SidebarController.select(last ? lastChannel : DeviceModel.channels.get(0, "channel"))
    }
    onShownChanged: {
        selectOne()
        ChannelController.watchLimiter(shown)
    }
    Component.onCompleted: lastChannel = SidebarController.selectedChannel
    Component.onDestruction: if (shown) ChannelController.watchLimiter(false)
    Connections {
        target: SidebarController
        function onSelectionChanged() {
            page.renamingChannel = -1
            if (SidebarController.selectedChannel >= 0) page.lastChannel = SidebarController.selectedChannel
            else Qt.callLater(page.selectOne)
        }
    }
    Connections {
        target: DeviceModel
        function onSnapshotApplied() { page.selectOne() }
    }
    Binding {
        target: InspectorState
        property: "windowWidth"
        value: page.Window.width
        when: page.shown && page.Window.width > 0
        restoreMode: Binding.RestoreNone
    }
    // The limiter's settings, asked for on the strip (or by a tool)
    function showLimiter() {
        InspectorState.setExpanded("limiter", true)
        InspectorState.show()
    }
    Connections {
        target: page.shown ? ChannelController : null
        function onLimiterSettingsRequested(output) {
            SidebarController.select(ChannelController.outputChannel(output))
            page.showLimiter()
        }
    }

    Text {
        anchors.centerIn: parent
        visible: !DeviceModel.connected
        text: qsTr("No Device Connected")
        font: Theme.fontTitle
        color: Theme.textMuted
    }

    RowLayout {
        anchors.fill: parent
        visible: DeviceModel.connected
        spacing: 0

        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: Theme.spacingSmall
            Layout.topMargin: 4
            spacing: Theme.spacingSmall

            ChannelStrip {
                id: strip
                Layout.fillWidth: true
                visible: page.channelsShown
                channel: page.channel
                onLimiterWanted: page.showLimiter()
            }
            RowLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                spacing: Theme.spacingSmall

                Rectangle {
                    visible: page.channelsShown
                    Layout.preferredWidth: page.width < 1000 ? 184 : 208
                    Layout.fillHeight: true
                    radius: Theme.radius
                    color: Theme.panel

                    ListView {
                        objectName: page.objectName + ".channels"
                        anchors.fill: parent
                        anchors.bottomMargin: 4
                        clip: true
                        model: DeviceModel.channels
                        boundsBehavior: Flickable.StopAtBounds
                        ScrollBar.vertical: AppScrollBar {}
                        section.property: "kind"
                        section.delegate: Text {
                            required property string section
                            leftPadding: Theme.spacing
                            topPadding: 10
                            bottomPadding: 4
                            text: section === "input" ? qsTr("INPUTS") : qsTr("OUTPUTS")
                            font: Theme.fontHeading
                            color: Theme.textMuted
                            Accessible.role: Accessible.Heading
                            Accessible.name: text
                        }
                        delegate: ChannelRow {
                            nameWidth: SidebarController.derivedLabels ? 104 : 80
                            renaming: page.renamingChannel === channel
                            onRenameRequested: page.renamingChannel = channel
                            onRenameFinished: if (page.renamingChannel === channel) page.renamingChannel = -1
                        }
                        Accessible.role: Accessible.List
                        Accessible.name: qsTr("Channels")
                    }
                }
                Item {
                    id: centre
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                }
            }
        }
        Inspector {
            id: pane
            objectName: page.objectName + ".inspector"
            Layout.fillHeight: true
            shown: page.shown
            content: page.inspector
        }
    }

    OutputEnableQuestion {
        objectName: page.objectName + ".enableQuestion"
        active: page.shown
    }
}
