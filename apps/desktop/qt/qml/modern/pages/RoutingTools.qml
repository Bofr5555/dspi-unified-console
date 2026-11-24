import QtQuick
import UConsole

// -- WP LAY5a -- The buttons in the row over the matrix.
// -- WP MX1 -- The matrix's tools (Link L/R pairs, Direct 1:1, Clear, Dense)
// and the inspector's button.
Row {
    objectName: "RoutingTools"
    spacing: 6
    MatrixTools {
        objectName: "RoutingTools.matrix"
        anchors.verticalCenter: parent.verticalCenter
        prefix: "RoutingTools"
        visible: MatrixController.columns > 0
    }
    InspectorButton { objectName: "RoutingTools.inspector" }
}
