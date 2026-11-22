import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY3 -- Equaliser: the channels beside the graph and the selected
// channel's page, Classic's own parts until WP LAY4 brings the channel strip,
// the tabs and the inspector. One channel is always selected here.
Item {
    id: page
    objectName: "EqualiserSection"
    property bool shown: false
    property int lastChannel: -1
    property int renamingChannel: -1

    function selectOne() {
        if (!shown || !DeviceModel.connected || SidebarController.selectedChannel >= 0) return
        const last = lastChannel >= 0 && DeviceModel.channels.rowOfChannel(lastChannel) >= 0
        SidebarController.select(last ? lastChannel : DeviceModel.channels.get(0, "channel"))
    }
    onShownChanged: selectOne()
    Component.onCompleted: lastChannel = SidebarController.selectedChannel
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

    SplitView {
        anchors.fill: parent
        orientation: Qt.Horizontal
        handle: Rectangle {
            implicitWidth: 1
            color: Theme.separator
        }
        Rectangle {
            SplitView.preferredWidth: page.width < 1000 ? 200 : 220
            SplitView.minimumWidth: 200
            SplitView.maximumWidth: 320
            color: Theme.sidebar

            ListView {
                objectName: "EqualiserSection.channels"
                anchors.fill: parent
                clip: true
                model: DeviceModel.channels
                opacity: DeviceModel.connected ? 1 : 0
                enabled: DeviceModel.connected
                ScrollBar.vertical: AppScrollBar {}
                section.property: "kind"
                section.delegate: Text {
                    required property string section
                    leftPadding: Theme.spacing
                    topPadding: Theme.spacing
                    bottomPadding: 4
                    text: section === "input" ? qsTr("INPUTS") : qsTr("OUTPUTS")
                    font: Theme.fontHeading
                    color: Theme.textMuted
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
                delegate: ChannelRow {
                    nameWidth: SidebarController.derivedLabels ? 112 : 80
                    renaming: page.renamingChannel === channel
                    onRenameRequested: page.renamingChannel = channel
                    onRenameFinished: if (page.renamingChannel === channel) page.renamingChannel = -1
                }
                Accessible.role: Accessible.List
                Accessible.name: qsTr("Channels")
            }
        }
        RightPane {
            SplitView.fillWidth: true
            selectedChannel: SidebarController.selectedChannel
        }
    }
}
