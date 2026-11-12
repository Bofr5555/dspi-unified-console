import QtQuick
import QtQuick.Layouts
import UConsole

// The right pane (the Mac's ContentView detail column): the response graph on
// top and, below it, the overview (dashboard) or the selected channel's page,
// an input's (InputDetail) or an output's (OutputDetail).
Rectangle {
    id: pane
    objectName: "RightPane"
    color: Theme.window
    property int selectedChannel: -1
    // "overview", "input" or "output"
    readonly property string selectedKind: selectedChannel < 0 ? "overview" : SidebarController.selectedKind

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacing

        // -- WP QP3 -- The response graph: header, graph, drag strip
        GraphHost {
            Layout.fillWidth: true
            selectedChannel: pane.selectedChannel
            // -- WP QPI1 -- an input page shows its linked partner's curve too
            linkedPartner: pane.selectedKind === "input"
                ? (ChannelController.revision < 0 ? -1 : ChannelController.linkedPartner(pane.selectedChannel)) : -1
        }

        // The overview's or the channel detail's place
        Rectangle {
            objectName: "RightPane.detail"
            Layout.fillWidth: true
            Layout.fillHeight: true
            radius: Theme.radius
            color: Theme.panel
            border.width: 1
            border.color: Theme.separator

            Text {
                anchors.centerIn: parent
                visible: !DeviceModel.connected
                text: qsTr("No Device Connected")
                font: Theme.fontTitle
                color: Theme.textMuted
            }

            // -- WP QP5 -- The Overview while no channel is selected
            Dashboard {
                anchors.fill: parent
                visible: pane.selectedChannel < 0
            }

            // -- WP QPI1 -- The selected channel's page
            Loader {
                objectName: "RightPane.page"
                anchors.fill: parent
                anchors.margins: Theme.spacing
                active: DeviceModel.connected && pane.selectedChannel >= 0
                sourceComponent: pane.selectedKind === "input" ? inputPage
                               : pane.selectedKind === "output" ? outputPage : null
            }
            Component {
                id: inputPage
                InputDetail { input: SidebarController.selectedIndex }
            }
            Component {
                id: outputPage
                OutputDetail { output: SidebarController.selectedIndex }
            }
        }
    }
}
