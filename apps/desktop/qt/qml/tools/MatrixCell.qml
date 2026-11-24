import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP QP6 -- One crosspoint of the matrix (the Mac's MatrixPoint).
// -- WP MX1 -- Redrawn (DESREV's matrix-mockup.html): a cell never changes
// size. Connected, it is a fixed tile tinted with the input's colour, a solid
// 3 px bar of it at the left and the gain inside ("0", "-6", "+1.5"); an
// inverted crosspoint adds a warning-coloured ø. Empty, it is a 4 px dot,
// a "+" tile while hovered. (-- WP REVDFIX -- REVD D7: the PDM column's dots
// are no longer in the warning colour while enabling PDM would collide on
// Core 1: only its power icon is, with the tooltip, and enabling it asks.) A cell whose output is off is dimmed
// (45 %; in Modern no lower than AA contrast allows, Theme.dim) and stays
// editable.
//
// Mouse: a click on an empty cell connects it; on a connected one it selects
// it, and a click on the selected one disconnects it. A double-click types
// the gain in the tile, the wheel steps it (MatrixField), the ø on hover
// inverts, a right-click opens the cell's menu. The keyboard is the grid's
// (MatrixPanel); `view` is the panel.
//
// A delegate of MatrixController.model (roles input, output, connected, gain,
// invert, outputEnabled, conflict).
Item {
    id: cell
    required property int index
    required property int input
    required property int output
    required property bool connected
    required property real gain
    required property bool invert
    required property bool outputEnabled
    required property bool conflict
    property string inputName
    property string outputName
    property Item view: null
    property int tileWidth: 48
    property int tileHeight: 28

    readonly property bool current: MatrixController.currentInput === input && MatrixController.currentOutput === output
    readonly property bool editing: view !== null && view.editInput === input && view.editOutput === output
    readonly property color tint: ChannelPalette.input(input)
    readonly property bool hovered: hover.hovered

    objectName: "Matrix.cell." + input + "." + output
    implicitWidth: 56
    implicitHeight: 40
    opacity: outputEnabled ? 1 : Theme.dim(0.45)   // Modern: no lower than AA allows
    enabled: MatrixController.writable

    // The person types the gain: the field takes the focus, with the key that started it
    onEditingChanged: {
        // Done: the field lets go, else the grid's scope would give it the focus again
        if (!editing) {
            gainField.focus = false
            return
        }
        gainField.forceActiveFocus()
        const seed = view ? view.editSeed : ""
        if (seed.length) {
            gainField.text = seed
            gainField._typed = seed
            gainField._edited = true
            gainField.cursorPosition = seed.length
        }
    }

    HoverHandler {
        id: hover
        onHoveredChanged: if (cell.view) cell.view.hovering(cell.input, cell.output, hovered)
    }

    // ---- Empty -------------------------------------------------------------
    Rectangle {
        anchors.centerIn: parent
        visible: !cell.connected && !(cell.hovered && cell.enabled)
        width: 4
        height: 4
        radius: 2
        color: Theme.border   // -- WP REVDFIX -- D7: not a standing warning
    }
    Rectangle {
        anchors.centerIn: parent
        visible: !cell.connected && cell.hovered && cell.enabled
        width: cell.tileWidth
        height: cell.tileHeight
        radius: Theme.radiusSmall
        color: Theme.controlHover
        Text {
            anchors.centerIn: parent
            text: "+"
            font.pixelSize: 14
            color: Theme.textMuted
        }
    }

    // ---- Connected: the tile -------------------------------------------------
    Rectangle {
        id: tile
        anchors.centerIn: parent
        visible: cell.connected
        width: cell.tileWidth
        height: cell.tileHeight
        radius: Theme.radiusSmall
        color: Qt.tint(Theme.panel, Qt.rgba(cell.tint.r, cell.tint.g, cell.tint.b, 0.22))
        clip: true
        Rectangle {
            width: 3
            height: parent.height
            color: cell.tint
        }
    }

    AbstractButton {
        id: connect
        objectName: "Matrix.cell." + cell.input + "." + cell.output + ".connect"
        anchors.fill: parent
        z: 1   // over the number, which takes the mouse only while typed in
        visible: !cell.editing
        hoverEnabled: false
        focusPolicy: Qt.NoFocus
        onClicked: if (cell.view) cell.view.cellClicked(cell.input, cell.output)
        onDoubleClicked: if (cell.view) cell.view.beginEdit(cell.input, cell.output, "")
        background: null
        contentItem: null
        // On the button, over the number: a right-click opens the cell's menu
        TapHandler {
            acceptedButtons: Qt.RightButton
            onTapped: if (cell.view) cell.view.openCellMenu(cell.input, cell.output, cell)
        }
        Accessible.role: Accessible.CheckBox
        Accessible.name: qsTr("%1 to %2").arg(cell.inputName).arg(cell.outputName)
        Accessible.description: cell.connected ? qsTr("%1 dB").arg(gainField.format(cell.gain)) + (cell.invert ? ", " + qsTr("phase inverted") : "") : ""
        Accessible.checkable: true
        Accessible.checked: cell.connected
        Accessible.onPressAction: connect.clicked()
    }

    MatrixField {
        id: gainField
        objectName: "Matrix.cell." + cell.input + "." + cell.output + ".gain"
        z: cell.editing ? 3 : 0
        x: tile.x + 3
        y: tile.y
        width: tile.width - 3 - (cell.invert || cell.hovered ? invertButton.width : 0)
        height: tile.height
        visible: cell.connected
        kind: "gain"
        value: cell.gain
        showUnit: false
        bare: true
        resetOnRightClick: false
        focusPolicy: Qt.NoFocus
        horizontalAlignment: TextInput.AlignRight
        leftPadding: 2
        rightPadding: 4
        font.pixelSize: cell.tileHeight < 26 ? 11 : 12
        color: Theme.text
        label: qsTr("%1 to %2 gain").arg(cell.inputName).arg(cell.outputName)
        onValueCommitted: v => MatrixController.setGain(cell.input, cell.output, v)
        onAccepted: if (cell.view) Qt.callLater(cell.view.endEdit)
        // The focus and the editing go together: focus given another way (an
        // assistive tool) starts it, and none is kept once it ends
        onActiveFocusChanged: {
            if (!cell.view) return
            if (activeFocus && !cell.editing) cell.view.beginEdit(cell.input, cell.output, "")
            else if (!activeFocus && cell.editing) cell.view.editLeft()
        }
    }

    AbstractButton {
        id: invertButton
        objectName: "Matrix.cell." + cell.input + "." + cell.output + ".invert"
        z: 2
        x: tile.x + tile.width - width
        y: tile.y
        width: 12
        height: tile.height
        visible: cell.connected && !cell.editing
        opacity: cell.invert ? 1 : cell.hovered ? 0.5 : 0
        hoverEnabled: true
        focusPolicy: Qt.NoFocus
        onClicked: MatrixController.setInvert(cell.input, cell.output, !cell.invert)
        background: null
        contentItem: Text {
            text: "ø"
            font.pixelSize: 11
            font.weight: Font.Bold
            color: cell.invert ? Theme.warning : Theme.textMuted
            horizontalAlignment: Text.AlignLeft
            verticalAlignment: Text.AlignVCenter
        }
        HelpTip {
            text: cell.invert ? qsTr("Phase inverted (I)") : qsTr("Invert phase (I)")
            visible: invertButton.hovered
        }
        Accessible.role: Accessible.CheckBox
        Accessible.name: qsTr("%1 to %2 phase invert").arg(cell.inputName).arg(cell.outputName)
        Accessible.checkable: true
        Accessible.checked: cell.invert
        Accessible.onPressAction: invertButton.clicked()
    }

    // The grid's keyboard focus, the one outline (DESIGN-LANGUAGE §5)
    Rectangle {
        anchors.fill: parent
        anchors.margins: 1
        visible: cell.current && cell.view !== null && cell.view.gridFocused && !cell.editing
        radius: Theme.radiusControl
        color: "transparent"
        border.width: Theme.focusRingWidth
        border.color: Theme.focusRing
    }

    HelpTip {
        text: cell.inputName + " → " + cell.outputName + (cell.connected ? ": " + qsTr("%1 dB").arg(gainField.format(cell.gain)) + (cell.invert ? ", " + qsTr("inverted") : "") : "")
        visible: cell.hovered && !cell.editing && !invertButton.hovered
    }

    // -- WP AIVIS -- an agent changed it just now
    AiTouched { objectName: cell.objectName + ".touched"; path: "mix"; indices: [cell.input, cell.output] }
}
