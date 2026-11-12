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
    function entry(shape) {
        for (const s of editor.shapes) if (s.shape === shape) return s
        return null
    }
    function pickShape(shape) {
        const e = entry(shape)
        if (!e) return
        if (e.orders.length === 2) {
            pendingShape = shape
            hoveredShape = -1
        } else {
            picked(shape, e.orders[0])
        }
    }

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
        width: parent.width - 26
        height: 17
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
        text: { const e = chooser.entry(chooser.named); return e ? e.title : qsTr("Add Band") }
        font: Theme.fontCaption
        color: Theme.text
    }
    Rectangle {
        x: 8
        y: 24
        width: parent.width - 16
        height: 1
        color: Theme.separator
    }

    // Step one: the grid; a short last row is centred
    Item {
        id: grid
        visible: chooser.pendingShape < 0
        x: 8
        y: chooser.bodyTop
        width: parent.width - 16
        height: parent.height - chooser.bodyTop - 4
        readonly property int columns: 4
        readonly property int count: chooser.editor.shapes.length
        readonly property int rows: Math.max(1, Math.ceil(count / columns))
        readonly property real cellWidth: width / columns
        readonly property real cellHeight: (height + 1) / rows

        Repeater {
            model: chooser.editor.shapes
            delegate: Rectangle {
                id: cell
                required property var modelData
                required property int index
                readonly property int row: Math.floor(index / grid.columns)
                readonly property int inRow: Math.min(grid.columns, grid.count - row * grid.columns)
                readonly property bool marked: modelData.shape === chooser.ownShape
                objectName: chooser.namePrefix + ".shape." + modelData.shape
                x: (grid.columns - inRow) * grid.cellWidth / 2 + (index % grid.columns) * grid.cellWidth
                y: row * grid.cellHeight
                width: grid.cellWidth
                height: grid.cellHeight - 1
                radius: 5
                color: marked ? chooser.tint(0.16) : cellArea.containsMouse ? chooser.tint(0.09) : "transparent"
                Accessible.role: Accessible.Button
                Accessible.name: modelData.title
                Accessible.onPressAction: chooser.pickShape(modelData.shape)

                PeqShapeGlyph {
                    anchors.centerIn: parent
                    width: 18
                    height: 12
                    shape: cell.modelData.shape
                    color: cell.marked ? chooser.markColor : Theme.textMuted
                }
                MouseArea {
                    id: cellArea
                    anchors.fill: parent
                    hoverEnabled: true
                    onContainsMouseChanged: {
                        if (containsMouse) chooser.hoveredShape = cell.modelData.shape
                        else if (chooser.hoveredShape === cell.modelData.shape) chooser.hoveredShape = -1
                    }
                    onClicked: chooser.pickShape(cell.modelData.shape)
                }
                HelpTip {
                    text: cell.modelData.title
                    visible: cellArea.containsMouse
                }
            }
        }
    }

    // Step two: the picked shape's two orders; only the band's own shape
    // shows its order
    Row {
        visible: chooser.pendingShape >= 0
        spacing: 6
        anchors.horizontalCenter: parent.horizontalCenter
        y: Math.round((chooser.bodyTop + chooser.height - 4) / 2 - 10)
        Repeater {
            model: [1, 2]
            delegate: Rectangle {
                id: slope
                required property int modelData
                readonly property bool marked: chooser.ownShape === chooser.pendingShape && chooser.ownOrder === modelData
                objectName: chooser.namePrefix + ".order." + modelData
                width: 42
                height: 20
                radius: 5
                color: marked ? chooser.tint(0.16) : chooser.tint(slopeArea.containsMouse ? 0.14 : 0.07)
                Accessible.role: Accessible.Button
                Accessible.name: label.text
                Accessible.onPressAction: chooser.picked(chooser.pendingShape, modelData)
                Text {
                    id: label
                    anchors.centerIn: parent
                    text: chooser.pendingShape >= 0 ? chooser.editor.orderLabel(chooser.pendingShape, slope.modelData) : ""
                    font: Theme.fontSmall
                    color: slope.marked ? chooser.markColor : Theme.text
                }
                MouseArea {
                    id: slopeArea
                    anchors.fill: parent
                    hoverEnabled: true
                    onClicked: chooser.picked(chooser.pendingShape, slope.modelData)
                }
                HelpTip {
                    text: chooser.pendingShape >= 0 ? chooser.editor.orderHelp(chooser.pendingShape, slope.modelData) : ""
                    visible: slopeArea.containsMouse
                }
            }
        }
    }
}
