import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP DL0 -- The frame every graph sits in (DESIGN-LANGUAGE §12): the
// Effects previews, Measure, Live, AutoEQ. The plot itself stays the page's;
// the frame gives the rest the same everywhere:
// - the plot area in Theme.panel with radius 8 and no outline (`filled`);
// - grid lines in Theme.graphGrid at the ticks (`grid`), under the plot;
// - axis labels in fontCaption muted drawn inside with a 6 px inset, never
//   cut (AxisLabels: `xTicks`, `yTicks`, `rightTicks`, lists of { at, text },
//   `at` from 0 to 1: left to right, top to bottom);
// - the empty label centred over the plot while `empty` ("No curve yet",
//   "Disabled"), with the plot hidden;
// - the graph's toggles at the top right (`tools`: ToggleChips with
//   overlay: true), and a hint sentence under the plot in fontSmall muted.
//
//     GraphFrame {
//         objectName: "Tube.transfer"        // parts: ".plot", ".labels", ".empty", ".hint"
//         Layout.fillWidth: true
//         Layout.preferredHeight: 180
//         xTicks: [{ at: 0, text: "-1" }, { at: 0.5, text: "0" }, { at: 1, text: "+1" }]
//         yTicks: [{ at: 0, text: "+1" }, { at: 1, text: "-1" }]
//         empty: !panel.on
//         emptyText: qsTr("Disabled")
//         hint: qsTr("Drag the curve to change the drive.")
//         tools: [ ToggleChip { overlay: true; text: qsTr("Input") } ]
//         TubeTransferGraph { anchors.fill: parent }   // the plot: fills plotArea
//     }
Item {
    id: frame
    property var xTicks: []
    property var yTicks: []
    property var rightTicks: []
    property bool grid: true
    property bool filled: true
    property bool empty: false
    property string emptyText
    property string hint
    property int inset: 6
    default property alias plot: plotArea.data
    property alias tools: toolRow.data
    readonly property alias plotArea: plotArea

    implicitWidth: 320
    implicitHeight: 160 + (hintText.visible ? hintText.implicitHeight + Theme.spacingSmall : 0)

    Rectangle {
        id: surface
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.top: parent.top
        anchors.bottom: hintText.visible ? hintText.top : parent.bottom
        anchors.bottomMargin: hintText.visible ? Theme.spacingSmall : 0
        radius: Theme.radius
        color: frame.filled ? Theme.panel : "transparent"
        clip: true

        // The grid, at the ticks that are not on an edge
        Repeater {
            model: frame.grid ? frame.xTicks : []
            Rectangle {
                required property var modelData
                visible: modelData.at > 0 && modelData.at < 1
                x: Math.round(modelData.at * surface.width)
                width: 1
                height: surface.height
                color: Theme.graphGrid
            }
        }
        Repeater {
            model: frame.grid ? frame.yTicks : []
            Rectangle {
                required property var modelData
                visible: modelData.at > 0 && modelData.at < 1
                y: Math.round(modelData.at * surface.height)
                width: surface.width
                height: 1
                color: Theme.graphGrid
            }
        }

        Item {
            id: plotArea
            objectName: frame.objectName.length ? frame.objectName + ".plot" : ""
            anchors.fill: parent
            visible: !frame.empty
        }

        AxisLabels {
            objectName: frame.objectName.length ? frame.objectName + ".labels" : ""
            anchors.fill: parent
            visible: !frame.empty
            inset: frame.inset
            xTicks: frame.xTicks
            yTicks: frame.yTicks
            rightTicks: frame.rightTicks
        }

        Text {
            objectName: frame.objectName.length ? frame.objectName + ".empty" : ""
            anchors.centerIn: parent
            width: Math.min(implicitWidth, parent.width - 2 * Theme.spacing)
            visible: frame.empty && text.length > 0
            textFormat: Text.PlainText
            text: frame.emptyText
            font: Theme.fontBody
            color: Theme.textMuted
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
        }

        Row {
            id: toolRow
            anchors.top: parent.top
            anchors.right: parent.right
            anchors.margins: frame.inset
            spacing: 4
        }
    }

    Text {
        id: hintText
        objectName: frame.objectName.length ? frame.objectName + ".hint" : ""
        anchors.left: parent.left
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        visible: text.length > 0
        textFormat: Text.PlainText
        text: frame.hint
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }

    Accessible.role: Accessible.Graphic
    Accessible.name: empty ? emptyText : hint
}
