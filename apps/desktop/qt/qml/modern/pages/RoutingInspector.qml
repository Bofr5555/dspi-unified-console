import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5a -- The inspector beside the matrix (MODLAY 2): the output whose
// column is selected, or the input whose row is.
Inspector {
    id: inspector
    objectName: "RoutingInspector"
    property Item host: null

    // The limiter's state, while its section can be seen
    readonly property bool watching: open
    onWatchingChanged: ChannelController.watchLimiter(watching)
    Component.onDestruction: if (watching) ChannelController.watchLimiter(false)
    Binding {
        target: InspectorState
        property: "windowWidth"
        value: inspector.Window.width
        when: inspector.shown && inspector.Window.width > 0
        restoreMode: Binding.RestoreNone
    }

    content: Component {
        ColumnLayout {
            spacing: Theme.spacing
            ChannelInfo { id: info; channel: SidebarController.selectedChannel }

            Text {
                objectName: "RoutingInspector.none"
                Layout.fillWidth: true
                Layout.rightMargin: 28   // the close button
                visible: !info.known
                text: qsTr("Choose an output's column, or an input's row, in the matrix to see it here.")
                font: Theme.fontBody
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            ChannelInspector {
                Layout.fillWidth: true
                visible: info.known
                channel: SidebarController.selectedChannel
                host: inspector.host
                routingLink: false
            }
            AppButton {
                objectName: "RoutingInspector.equaliser"
                visible: info.known
                text: qsTr("Open in Equaliser")
                onClicked: inspector.host.go("equaliser", "peq")
            }
        }
    }
}
