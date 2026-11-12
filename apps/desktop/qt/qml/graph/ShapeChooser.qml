import QtQuick
import UConsole

// -- WP QP4 -- Choosing a shape, then its slope (the Mac's PeqShapeChooser):
// a header with a back arrow and the shape's name, a grid of the shapes the
// firmware offers, and for a shape with two orders a second step with its two
// slopes (or all-pass phases). Only the last click reports, with shape and
// order together. The band chip shows it as its shape page, marking the
// band's own shape; the Ctrl-click card shows it for a band not made yet.
Item {
    id: chooser
    required property PeqOverlay editor
    // The band's own shape and order (-1 for none) and its colour
    property int ownShape: -1
    property int ownOrder: 0
    property color markColor: Theme.text
    // What the back arrow does on the first step
    property string backHelp
    property string namePrefix: "PeqChip.shapes"

    // Step two's shape, and the shape under the pointer on step one
    property int pendingShape: -1
    property int hoveredShape: -1
    signal picked(int shape, int order)
    signal back()

    // Theme.text at `alpha`: the hover and marked fills
    function tint(alpha) { return Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, alpha) }
    function reset() {
        pendingShape = -1
        hoveredShape = -1
    }
    function entry(shape) { return editor.shapes.find(s => s.shape === shape) || null }
    // A shape with one order is reported now; one with two goes to step two
    function pickShape(shape) {
        const e = entry(shape)
        if (e === null) return
        if (e.orders.length > 1) {
            hoveredShape = -1
            pendingShape = shape
            return
        }
        picked(shape, e.orders[0])
    }

    // The header names the picked shape, else the one under the pointer,
    // else the band's own
    readonly property int named: pendingShape >= 0 ? pendingShape : hoveredShape >= 0 ? hoveredShape : ownShape
    readonly property real bodyTop: 27

    IconButton {
        objectName: chooser.namePrefix + ".back"
        x: 3
        y: 4
        width: 15
        height: 17
        iconName: "chevron-left"
        iconSize: 10
        iconColor: Theme.textMuted
        toolTip: chooser.pendingShape < 0 ? chooser.backHelp : qsTr("Back to shapes")
        onClicked: chooser.pendingShape >= 0 ? chooser.pendingShape = -1 : chooser.back()
    }
    Text {
        objectName: chooser.namePrefix + ".name"
        x: 18
        y: 5
        width: chooser.width - x - 8
        height: 17
        text: { const e = chooser.entry(chooser.named); return e ? e.title : qsTr("Add Band") }
        font: Theme.fontCaption
        color: Theme.text
        elide: Text.ElideRight
        verticalAlignment: Text.AlignVCenter
    }
    Rectangle {
        x: 8
        y: 24
        width: chooser.width - 2 * x
        height: 1
        color: Theme.separator
    }

    // Step one: four to a row filling the body; a short last row is centred
    Item {
        id: body
        visible: chooser.pendingShape < 0
        x: 8
        y: chooser.bodyTop
        width: chooser.width - 2 * x
        height: chooser.height - y - 4
        readonly property int perRow: 4
        readonly property int total: chooser.editor.shapes.length
        readonly property real cellW: width / perRow
        readonly property real cellH: (height + 1) / Math.max(1, Math.ceil(total / perRow))
        // Where cell `i` goes: its row's cells sit in the middle of the row
        function place(i) {
            const line = Math.floor(i / perRow)
            const used = Math.min(perRow, total - line * perRow)
            return Qt.point((perRow - used) * cellW / 2 + (i - line * perRow) * cellW, line * cellH)
        }

        Repeater {
            model: chooser.editor.shapes
            delegate: Rectangle {
                id: cell
                required property var modelData
                required property int index
                readonly property bool marked: modelData.shape === chooser.ownShape
                objectName: chooser.namePrefix + ".shape." + modelData.shape
                x: body.place(index).x
                y: body.place(index).y
                width: body.cellW
                height: body.cellH - 1
                radius: 5
                color: marked ? chooser.tint(0.16) : cellHover.hovered ? chooser.tint(0.09) : "transparent"
                Accessible.role: Accessible.Button
                Accessible.name: modelData.title
                Accessible.onPressAction: chooser.pickShape(modelData.shape)

                PeqShapeGlyph {
                    anchors.centerIn: parent
                    width: 18
                    height: 12
                    shape: cell.modelData.shape
                    ink: cell.marked ? chooser.markColor : Theme.textMuted
                }
                HoverHandler {
                    id: cellHover
                    onHoveredChanged: {
                        if (hovered) chooser.hoveredShape = cell.modelData.shape
                        else if (chooser.hoveredShape === cell.modelData.shape) chooser.hoveredShape = -1
                    }
                }
                TapHandler { onTapped: chooser.pickShape(cell.modelData.shape) }
                HelpTip {
                    text: cell.modelData.title
                    visible: cellHover.hovered
                }
            }
        }
    }

    // Step two: the picked shape's two orders; only the band's own shape
    // shows its order marked
    Row {
        visible: chooser.pendingShape >= 0
        spacing: 6
        x: (chooser.width - width) / 2
        y: Math.round((chooser.bodyTop + chooser.height - 4) / 2 - 10)
        Repeater {
            model: [1, 2]
            delegate: Rectangle {
                id: slope
                required property int modelData
                readonly property bool marked: chooser.ownShape === chooser.pendingShape && chooser.ownOrder === modelData
                readonly property string label: chooser.pendingShape < 0 ? "" : chooser.editor.orderLabel(chooser.pendingShape, modelData)
                objectName: chooser.namePrefix + ".order." + modelData
                width: 42
                height: 20
                radius: 5
                color: marked ? chooser.tint(0.16) : chooser.tint(slopeHover.hovered ? 0.14 : 0.07)
                Accessible.role: Accessible.Button
                Accessible.name: label
                Accessible.onPressAction: chooser.picked(chooser.pendingShape, modelData)
                Text {
                    anchors.centerIn: parent
                    text: slope.label
                    font: Theme.fontSmall
                    color: slope.marked ? chooser.markColor : Theme.text
                }
                HoverHandler { id: slopeHover }
                TapHandler { onTapped: chooser.picked(chooser.pendingShape, slope.modelData) }
                HelpTip {
                    text: chooser.pendingShape < 0 ? "" : chooser.editor.orderHelp(chooser.pendingShape, slope.modelData)
                    visible: slopeHover.hovered
                }
            }
        }
    }
}
