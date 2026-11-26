import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY5a -- The buttons in the row over the matrix.
// -- WP MX1 -- The matrix's tools (Link L/R pairs, Direct 1:1, Clear, Dense)
// and the inspector's button.
// -- WP UB1 -- (UXP B1) "From Speaker Layout…": every output of the confirmed
// speaker layout routed from the inputs its speaker's name says, one undo step
// (CrossoverFromLayout with its routing part ticked).
Row {
    objectName: "RoutingTools"
    spacing: 6
    AppButton {
        objectName: "RoutingTools.fromLayout"
        anchors.verticalCenter: parent.verticalCenter
        visible: MatrixController.columns > 0
        text: qsTr("From Speaker Layout…")
        toolTip: qsTr("Route each output of the confirmed speaker layout from its side's input (L, R, centre, sub, surrounds), in one undo step")
        enabled: LayoutSetupController.writable
        onClicked: fromLayout.open()
        CrossoverFromLayout {
            id: fromLayout
            parent: Overlay.overlay
            crossoversWanted: false
            routingWanted: true
        }
    }
    MatrixTools {
        objectName: "RoutingTools.matrix"
        anchors.verticalCenter: parent.verticalCenter
        prefix: "RoutingTools"
        visible: MatrixController.columns > 0
    }
    InspectorButton { objectName: "RoutingTools.inspector" }
}
