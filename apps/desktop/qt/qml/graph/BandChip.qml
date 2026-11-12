import QtQuick
import UConsole

// -- WP QP4 -- The card beside a band's dot (the Mac's PeqBandHUD): a header
// with the shape's glyph in the band's colour and its two-letter code (click
// for the shape page) and the bypass button, then one row per value: label,
// number, unit, in fixed columns so the decimals line up. Drag a number up or
// down, scroll it, or double-click it to type ("2k", "A4"; Tab moves on).
// Tall and narrow on purpose: bands crowd along the frequency axis.
//
// The editor places and sizes it (PeqOverlay.chipX/Y/Width/Height) and says
// what it shows (PeqOverlay.chip); it fades out when the editor lets it go.
Rectangle {
    id: chip
    required property PeqOverlay editor
    // What it shows, kept while it fades out
    property var info: ({})
    readonly property bool shown: editor.chipBand >= 0
    readonly property color ink: info.bypass ? Theme.textMuted : (info.color || Theme.text)
    property int editField: -1

    // The Mac's metrics
    readonly property real pad: 8
    readonly property real unitX: width - pad - 19
    readonly property real numberX: unitX - 3 - 44

    objectName: "PeqChip"
    x: editor.chipX
    y: editor.chipY
    width: editor.chipWidth
    height: editor.chipHeight
    radius: Theme.radius
    color: Theme.menu
    border.width: 1
    border.color: Theme.border
    opacity: shown ? 1 : 0
    visible: opacity > 0
    Behavior on opacity { NumberAnimation { duration: chip.shown ? 120 : 150 } }

    Accessible.role: Accessible.Grouping
    Accessible.name: qsTr("Band %1").arg((info.band || 0) + 1)

    onShownChanged: if (!shown) endEditing()
    Connections {
        target: chip.editor
        function onViewChanged() { if (chip.editor.chipBand >= 0) chip.info = chip.editor.chip }
        function onEditRequested(field) { chip.beginEditing(field) }
    }
    Component.onCompleted: info = editor.chip

    function rowOf(field) {
        const rows = info.rows || []
        for (let i = 0; i < rows.length; ++i) if (rows[i].field === field) return i
        return -1
    }
    // The next value that can be typed, after `field` in row order
    function nextField(field, backwards) {
        const rows = (info.rows || []).filter(r => r.adjustable)
        if (rows.length === 0) return -1
        const at = rows.findIndex(r => r.field === field)
        const step = backwards ? -1 : 1
        return rows[((at < 0 ? (backwards ? 0 : -1) : at) + step + rows.length) % rows.length].field
    }
    function beginEditing(field) {
        const i = rowOf(field)
        if (i < 0 || !(info.rows[i].adjustable)) {
            const next = nextField(field, false)
            if (next < 0 || next === field) return
            field = next
        }
        editField = field
        editor.chipEditing(true)
    }
    function endEditing() {
        if (editField < 0) return
        editField = -1
        editor.chipEditing(false)
        editor.forceActiveFocus()
    }

    // The panel swallows clicks, which would otherwise make a band behind it
    MouseArea {
        anchors.fill: parent
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        onContainsMouseChanged: chip.editor.chipHovered(containsMouse)
    }

    // ---- Values page ----
    Item {
        anchors.fill: parent
        visible: !chip.editor.shapePage

        // The shape: glyph, code, chevron; the Linkwitz Transform's is inert
        Rectangle {
            id: shapeButton
            objectName: "PeqChip.shape"
            x: 4
            y: 4
            width: 60
            height: 17
            radius: 5
            color: shapeArea.containsMouse && chip.info.reshapable ? chip.tint(0.09) : "transparent"
            Accessible.role: Accessible.Button
            Accessible.name: chip.info.shapeTip || ""
            Accessible.onPressAction: chip.editor.chipToggleShapes()
            Row {
                anchors.verticalCenter: parent.verticalCenter
                x: 3
                spacing: 3
                PeqShapeGlyph {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: chip.info.reshapable === true
                    width: 18
                    height: 12
                    shape: chip.info.shape >= 0 ? chip.info.shape : 0
                    color: chip.ink
                }
                Text {
                    anchors.verticalCenter: parent.verticalCenter
                    text: chip.info.code || ""
                    font: Theme.fontSmall
                    color: chip.info.bypass || !chip.info.reshapable ? Theme.textMuted : Theme.text
                }
                Icon {
                    anchors.verticalCenter: parent.verticalCenter
                    visible: chip.info.reshapable === true
                    name: "chevron-down"
                    size: 8
                    color: Theme.textMuted
                }
            }
            MouseArea {
                id: shapeArea
                anchors.fill: parent
                hoverEnabled: true
                enabled: chip.info.reshapable === true
                onClicked: chip.editor.chipToggleShapes()
            }
            HelpTip {
                text: chip.info.shapeTip || ""
                visible: shapeArea.containsMouse
            }
        }

        IconButton {
            objectName: "PeqChip.bypass"
            visible: chip.info.bypassSupported === true
            x: chip.width - 22
            y: 4
            width: 18
            height: 17
            iconName: "power"
            iconSize: 10
            iconColor: chip.info.bypass ? Theme.textMuted : chip.ink
            toolTip: chip.info.powerTip || ""
            onClicked: chip.editor.chipBypass()
        }

        Rectangle {
            x: chip.pad
            y: 24
            width: chip.width - 2 * chip.pad
            height: 1
            color: Theme.separator
        }

        // By count, so a value that changes keeps its row (and a field being typed in)
        Repeater {
            model: (chip.info.rows || []).length
            delegate: Item {
                id: line
                required property int index
                readonly property var modelData: (chip.info.rows || [])[index] || ({})
                readonly property bool typing: chip.editField === modelData.field
                objectName: "PeqChip.row." + modelData.field
                // What the row shows, for tests and the accessible name
                readonly property string shownName: modelData.number + (modelData.unit ? " " + modelData.unit : "")
                x: 0
                y: 28 + index * 15
                width: chip.width
                height: 15

                Text {
                    x: chip.pad
                    y: 1
                    width: chip.numberX - chip.pad
                    text: line.modelData.label
                    font: Theme.fontCaption
                    color: Theme.textMuted
                    elide: Text.ElideNone
                    clip: true
                }
                Text {
                    visible: !line.modelData.inline
                    x: chip.unitX
                    y: 1
                    width: 19
                    text: line.modelData.unit
                    font: Theme.fontCaption
                    color: Theme.textMuted
                    clip: true
                }
                Text {
                    visible: !line.typing
                    x: chip.numberX
                    width: line.modelData.inline ? chip.width - chip.pad - chip.numberX : 44
                    height: 15
                    horizontalAlignment: Text.AlignRight
                    verticalAlignment: Text.AlignVCenter
                    text: line.modelData.inline ? line.modelData.number + " " + line.modelData.unit : line.modelData.number
                    font: Theme.fontMono
                    color: chip.info.bypass || !line.modelData.adjustable ? Theme.textMuted : Theme.text
                    Accessible.role: Accessible.StaticText
                    Accessible.name: line.modelData.label + " " + line.shownName
                }
                // Drag up or down, scroll, or double-click to type
                MouseArea {
                    objectName: "PeqChip.value." + line.modelData.field
                    x: chip.numberX
                    width: 44
                    height: 15
                    visible: !line.typing && line.modelData.adjustable
                    cursorShape: Qt.SizeVerCursor
                    preventStealing: true
                    property real lastY: 0
                    property real dragged: 0
                    onPressed: mouse => {
                        lastY = mapToGlobal(mouse.x, mouse.y).y
                        dragged = 0
                        chip.editor.chipAdjust(line.modelData.field, 0, 0)
                    }
                    onPositionChanged: mouse => {
                        // Screen coordinates: the chip moves with its dot
                        const y = mapToGlobal(mouse.x, mouse.y).y
                        dragged += (lastY - y) * ((mouse.modifiers & Qt.ShiftModifier) ? 0.12 : 1)
                        lastY = y
                        chip.editor.chipAdjust(line.modelData.field, dragged, 1)
                    }
                    onReleased: chip.editor.chipAdjust(line.modelData.field, dragged, 2)
                    onDoubleClicked: chip.beginEditing(line.modelData.field)
                    onWheel: wheel => {
                        // A graph wheel gesture, and Ctrl-wheel (the band's
                        // gain), go on to the graph
                        if ((wheel.modifiers & Qt.ControlModifier) || chip.editor.chipForwardsScroll()) {
                            wheel.accepted = false
                            return
                        }
                        const precise = wheel.pixelDelta.y !== 0 || wheel.pixelDelta.x !== 0
                        const raw = precise ? (wheel.pixelDelta.y || wheel.pixelDelta.x)
                                            : (wheel.angleDelta.y || wheel.angleDelta.x) / 120 * 8
                        chip.editor.chipScroll(line.modelData.field, raw, (wheel.modifiers & Qt.ShiftModifier) !== 0)
                    }
                }
                Rectangle {
                    visible: line.typing
                    x: chip.numberX - 2
                    width: 48
                    height: 15
                    radius: 3
                    color: Theme.control
                    border.width: 1
                    border.color: Theme.focusRing
                    TextInput {
                        id: input
                        objectName: "PeqChip.input." + line.modelData.field
                        anchors.fill: parent
                        anchors.rightMargin: 2
                        horizontalAlignment: Text.AlignRight
                        verticalAlignment: Text.AlignVCenter
                        font: Theme.fontMono
                        color: Theme.text
                        selectByMouse: true
                        clip: true
                        property bool done: false
                        Accessible.role: Accessible.EditableText
                        Accessible.name: line.modelData.label
                        function start() {
                            done = false
                            text = chip.editor.chipEditText(line.modelData.field)
                            forceActiveFocus()
                            selectAll()
                        }
                        onVisibleChanged: if (visible) start()
                        Component.onCompleted: if (line.typing) start()
                        // Enter or leaving the field takes the value; a value
                        // that does not parse is dropped
                        function finish(next) {
                            if (done) return
                            done = true
                            const accepted = chip.editor.chipText(line.modelData.field, text)
                            if (!accepted) chip.editor.refused()
                            if (next !== undefined && next >= 0) chip.beginEditing(next)
                            else chip.endEditing()
                        }
                        Keys.onReturnPressed: finish()
                        Keys.onEnterPressed: finish()
                        Keys.onTabPressed: finish(chip.nextField(line.modelData.field, false))
                        Keys.onBacktabPressed: finish(chip.nextField(line.modelData.field, true))
                        Keys.onEscapePressed: {
                            done = true
                            chip.endEditing()
                        }
                        onActiveFocusChanged: if (!activeFocus && visible) finish()
                    }
                }
            }
        }
    }

    // ---- Shape page, in place and at the same size ----
    ShapeChooser {
        id: shapes
        anchors.fill: parent
        visible: chip.editor.shapePage
        editor: chip.editor
        namePrefix: "PeqChip.shapes"
        ownShape: chip.info.reshapable ? chip.info.shape : -1
        ownOrder: chip.info.order || 0
        markColor: chip.ink
        backHelp: qsTr("Back to values")
        onVisibleChanged: reset()
        onPicked: (shape, order) => chip.editor.chipPick(shape, order)
        onBack: chip.editor.chipToggleShapes()
    }

    function tint(alpha) { return Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, alpha) }
}
