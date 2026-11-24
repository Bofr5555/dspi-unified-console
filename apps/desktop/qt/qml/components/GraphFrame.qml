import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP DL0 -- The frame every graph sits in (DESIGN-LANGUAGE §12): the
// Effects previews, Measure, Live, AutoEQ. The plot itself stays the page's;
// the frame gives the rest the same everywhere:
// - the plot area in Theme.panel with radius 8 and no outline (`filled`);
// - grid lines in Theme.graphGrid at the ticks (`grid`), under the plot,
//   stopping short of the axis labels (none while `empty`);
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

        // The grid, at the ticks that are not on an edge. A Canvas, not line
        // items, and each line stops short of the axis labels, so no label is
        // drawn on a grid line (lay7's contrast check reads the tone under it).
        Canvas {
            id: gridCanvas
            anchors.fill: parent
            visible: frame.grid && !frame.empty
            renderStrategy: Canvas.Cooperative
            readonly property color tone: Theme.graphGrid
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                ctx.fillStyle = tone
                const w = width, h = height
                for (const t of frame.xTicks || []) {
                    if (!(t.at > 0 && t.at < 1)) continue
                    ctx.fillRect(Math.round(t.at * w), 0, 1, Math.max(0, h - labels.bottomExtent))
                }
                const left = labels.leftExtent, right = w - labels.rightExtent
                for (const t of frame.yTicks || []) {
                    if (!(t.at > 0 && t.at < 1)) continue
                    ctx.fillRect(left, Math.round(t.at * h), Math.max(0, right - left), 1)
                }
            }
            onWidthChanged: requestPaint()
            onHeightChanged: requestPaint()
            onToneChanged: requestPaint()
            onVisibleChanged: if (visible) requestPaint()
            Connections {
                target: frame
                function onXTicksChanged() { gridCanvas.requestPaint() }
                function onYTicksChanged() { gridCanvas.requestPaint() }
                function onRightTicksChanged() { gridCanvas.requestPaint() }
            }
            Connections {
                target: labels
                function onLeftExtentChanged() { gridCanvas.requestPaint() }
                function onRightExtentChanged() { gridCanvas.requestPaint() }
                function onBottomExtentChanged() { gridCanvas.requestPaint() }
            }
        }

        Item {
            id: plotArea
            objectName: frame.objectName.length ? frame.objectName + ".plot" : ""
            anchors.fill: parent
            visible: !frame.empty
        }

        AxisLabels {
            id: labels
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
