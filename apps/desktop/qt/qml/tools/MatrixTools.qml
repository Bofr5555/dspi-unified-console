import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP MX1 -- The matrix's tools (DESREV's mockup): Link L/R pairs (a
// toggle, off by default, kept per device), Direct 1:1 and Clear (each one
// undo step; Clear asks when more than four crosspoints would go) and the
// dense grid. The Routing page shows them in the row over the page
// (RoutingTools, names "RoutingTools.*"); the matrix's own window over its
// grid (names "Matrix.*").
Row {
    id: tools
    property string prefix: "Matrix"
    spacing: 6

    ToggleChip {
        objectName: tools.prefix + ".link"
        anchors.verticalCenter: parent.verticalCenter
        text: qsTr("Link L/R pairs")
        iconName: "link"
        toolTip: qsTr("A change on one input of a stereo pair is made on the other one too, one column over (L → Out 1 and R → Out 2)")
        checked: MatrixController.linkPairs
        onToggled: MatrixController.linkPairs = checked
    }
    AppButton {
        objectName: tools.prefix + ".direct"
        anchors.verticalCenter: parent.verticalCenter
        text: qsTr("Direct 1:1")
        toolTip: qsTr("Route each input to the matching output (FL→OUT1, FR→OUT2, …) and disable the PDM sub")
        enabled: MatrixController.writable
        onClicked: MatrixController.directRouting()
    }
    AppButton {
        objectName: tools.prefix + ".clear"
        anchors.verticalCenter: parent.verticalCenter
        text: qsTr("Clear")
        toolTip: qsTr("Disconnect every crosspoint")
        enabled: MatrixController.writable
        onClicked: {
            const n = MatrixController.model.connectedCount()
            if (n <= 4) {
                MatrixController.clearRoutes()
                return
            }
            clearQuestion.text = qsTr("All %1 crosspoints will be disconnected. Their gain and phase are kept.").arg(n)
            clearQuestion.open()
        }
    }
    ToggleChip {
        objectName: tools.prefix + ".dense"
        anchors.verticalCenter: parent.verticalCenter
        text: qsTr("Dense")
        toolTip: qsTr("Smaller cells, without the trims and the gain and delay rows (always past 12 outputs)")
        checked: MatrixController.dense
        onToggled: MatrixController.dense = checked
    }

    QuestionDialog {
        id: clearQuestion
        objectName: tools.prefix + ".clearQuestion"
        ofDevice: true
        title: qsTr("Clear the routing?")
        acceptText: qsTr("Clear")
        destructive: true
        parent: Overlay.overlay
        onAnswered: answer => { if (answer === "accept") MatrixController.clearRoutes() }
    }
}
