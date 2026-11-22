import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QPI2 -- The band bar docked right under the graph (GraphController.
// PeqDock, the owner's choice over the Mac's PeqBandHUD beside the dot, for
// touch screens): the selected band's channel tag and number, its shape (the
// shape chooser, then its slope), Freq, Gain and Q to type or scroll, and
// bypass, invert and delete. With nothing selected it says how to pick a band.
// The editor is the PeqOverlay of the same graph (GraphPlot.peqEditor); the
// values, the typing rules and the writes are the overlay's (chip*).
Rectangle {
    id: bar
    // The GraphPlot it docks under (set by the slot)
    property var plot: null
    readonly property PeqOverlay editor: plot ? plot.peqEditor : null
    readonly property bool editing: editor !== null && editor.editing
    readonly property int band: editor ? editor.chipBand : -1
    readonly property var info: band >= 0 && editor ? editor.chip : ({})
    readonly property int channel: editor ? editor.editedChannel : -1
    readonly property color ink: info.bypass ? Theme.textMuted : (info.color || Theme.text)

    objectName: plot && plot.popOut ? "GraphWindow.peqBar" : "PeqBar"
    implicitHeight: Theme.controlHeight + 2 * 6
    radius: Theme.radius
    color: Theme.panel
    border { width: 1; color: Theme.separator }
    clip: true

    Accessible.role: Accessible.Grouping
    Accessible.name: band >= 0 ? qsTr("Band %1").arg(band + 1) : qsTr("Band editor")

    // The value row with `field` (PeqEditorController's Freq 0, Gain 1, Width 2), or null
    function rowOf(field) { return (info.rows || []).find(r => r.field === field) || null }
    function channelLabel(ch) {
        const rows = DeviceModel.channels
        const r = rows.rowOfChannel(ch)
        return r >= 0 ? rows.get(r, "label") : ""
    }
    function fieldOf(f) { return f === 0 ? freqSlot.input : f === 1 ? gainSlot.input : qSlot.input }

    Connections {
        target: bar.editor
        // Enter on the graph types the band's values, as on the chip
        function onEditRequested(field) {
            const f = bar.fieldOf(field)
            if (f.enabled) f.forceActiveFocus()
            else freqSlot.input.forceActiveFocus()
        }
    }
    // While the pointer or the keyboard is on the bar the band stays in it
    HoverHandler {
        onHoveredChanged: if (bar.editor) bar.editor.chipHovered(hovered)
    }

    // Freq, Gain, Q: the band's value, or a dash when its shape has none
    component ValueSlot: RowLayout {
        id: slot
        required property int field
        required property string caption
        readonly property var value: bar.rowOf(field)
        readonly property alias input: input
        spacing: 4
        // The value's own name ("Slope", "Order", "f0"), but "Q" for the width
        Text {
            text: !slot.value || (slot.field === 2 && slot.value.adjustable === true) ? slot.caption : slot.value.label
            font: Theme.fontCaption
            color: Theme.textMuted
        }
        // A value the shape fixes ("6 dB/oct", "1st") is text, not a field
        Text {
            objectName: bar.objectName + ".fixed." + slot.field
            visible: slot.value !== null && slot.value.adjustable !== true
            text: slot.value ? (slot.value.inline ? slot.value.number + " " + slot.value.unit : slot.value.number) : ""
            font: Theme.fontMono
            color: Theme.textMuted
        }
        BorderedField {
            id: input
            visible: slot.value === null || slot.value.adjustable === true
            objectName: bar.objectName + ".input." + slot.field
            label: slot.caption
            selectAllOnFocus: true // -- WP QTTWEAK --
            enabled: slot.value !== null && slot.value.adjustable === true && bar.editing
            implicitWidth: 64
            font: Theme.fontMono
            horizontalAlignment: TextInput.AlignRight
            // What it shows while not typed in
            readonly property string shownText: slot.value ? (slot.value.inline ? slot.value.number + " " + slot.value.unit
                                                                                : slot.value.number)
                                                           : "–"
            Binding on text {
                when: !input.activeFocus
                value: input.shownText
                restoreMode: Binding.RestoreNone
            }
            // The text typing began with: leaving the field unchanged writes nothing
            property string typedStart
            onActiveFocusChanged: {
                if (!bar.editor) return
                if (activeFocus) {
                    bar.editor.chipEditing(true)
                    typedStart = bar.editor.chipEditText(slot.field)
                    text = typedStart
                    selectAll()
                } else {
                    bar.editor.chipEditing(false)
                }
            }
            onCommitted: t => {
                if (!bar.editor || t === typedStart || t === shownText) return
                if (!bar.editor.chipText(slot.field, t)) bar.editor.refused()
                typedStart = t
            }
            // Enter takes the value and Escape drops it; both give the keys
            // back to the graph, as the chip did
            onAccepted: if (bar.editor) bar.editor.forceActiveFocus()
            Keys.onEscapePressed: {
                text = typedStart
                if (bar.editor) bar.editor.forceActiveFocus()
            }
            WheelHandler {
                acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
                onWheel: wheel => {
                    if (!input.enabled || !bar.editor) return
                    const precise = wheel.pixelDelta.y !== 0 || wheel.pixelDelta.x !== 0
                    const raw = precise ? (wheel.pixelDelta.y || wheel.pixelDelta.x)
                                        : (wheel.angleDelta.y || wheel.angleDelta.x) / 120 * 8
                    bar.editor.chipScroll(slot.field, raw, (wheel.modifiers & Qt.ShiftModifier) !== 0)
                }
            }
        }
        Text {
            visible: input.visible && slot.value !== null && !slot.value.inline && slot.value.unit.length > 0
            Layout.minimumWidth: 22
            text: slot.value ? slot.value.unit : ""
            font: Theme.fontCaption
            color: Theme.textMuted
        }
    }

    // ---- Nothing selected ----
    Text {
        objectName: bar.objectName + ".placeholder"
        visible: bar.band < 0
        anchors { left: parent.left; right: parent.right; verticalCenter: parent.verticalCenter; margins: Theme.spacing }
        text: bar.editing ? qsTr("No band selected. Click a dot or a row in the band list, or double-click the graph to add one.")
                          : qsTr("Open a channel's PEQ to edit its bands here.")
        font: Theme.fontSmall
        color: Theme.textMuted
        elide: Text.ElideRight
    }

    // ---- The selected band ----
    RowLayout {
        id: row
        visible: bar.band >= 0
        anchors { fill: parent; leftMargin: Theme.spacingSmall; rightMargin: 4 }
        spacing: Theme.spacingSmall

        // The channel's tag and the band's number
        Rectangle {
            objectName: bar.objectName + ".channelTag"
            Layout.alignment: Qt.AlignVCenter
            width: 8
            height: 8
            radius: 4
            color: bar.plot && bar.channel >= 0 && bar.plot.channelColors[bar.channel] !== undefined
                   ? bar.plot.channelColors[bar.channel] : Theme.textMuted
        }
        Text {
            objectName: bar.objectName + ".channel"
            Layout.maximumWidth: 110
            textFormat: Text.PlainText
            text: bar.channelLabel(bar.channel)
            font: Theme.fontSmall
            color: Theme.textMuted
            elide: Text.ElideRight
        }
        Text {
            objectName: bar.objectName + ".band"
            text: qsTr("Band %1").arg(bar.band + 1)
            font: Theme.fontSmall
            color: Theme.text
        }

        // The shape: glyph, code, chevron; opens the shape chooser
        AbstractButton {
            id: shapeButton
            objectName: bar.objectName + ".shape"
            enabled: bar.info.reshapable === true
            implicitWidth: shapeRow.implicitWidth + 12
            implicitHeight: Theme.controlHeight
            focusPolicy: Qt.TabFocus
            hoverEnabled: true
            onClicked: bar.editor.chipToggleShapes()
            Accessible.role: Accessible.Button
            Accessible.name: bar.info.shapeTip || ""
            background: Rectangle {
                radius: Theme.radiusSmall
                color: shapeButton.down ? Theme.controlPressed : shapeButton.hovered ? Theme.controlHover : Theme.control
                border.width: shapeButton.visualFocus ? 2 : 1
                border.color: shapeButton.visualFocus ? Theme.focusRing : Theme.border
            }
            contentItem: Item {
                Row {
                    id: shapeRow
                    anchors.centerIn: parent
                    spacing: 4
                    PeqShapeGlyph {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: bar.info.reshapable === true
                        width: 18
                        height: 12
                        shape: bar.info.shape >= 0 ? bar.info.shape : 0
                        ink: bar.ink
                    }
                    Text {
                        anchors.verticalCenter: parent.verticalCenter
                        text: bar.info.code || ""
                        font: Theme.fontSmall
                        color: shapeButton.enabled && !bar.info.bypass ? Theme.text : Theme.textMuted
                    }
                    Icon {
                        anchors.verticalCenter: parent.verticalCenter
                        visible: shapeButton.enabled
                        name: "chevron-down"
                        size: 8
                        color: Theme.textMuted
                    }
                }
            }
            HelpTip {
                text: bar.info.shapeTip || ""
                visible: shapeButton.hovered
            }
        }

        ValueSlot {
            id: freqSlot
            field: 0
            caption: qsTr("Freq")
        }
        ValueSlot {
            id: gainSlot
            field: 1
            caption: qsTr("Gain")
        }
        ValueSlot {
            id: qSlot
            field: 2
            caption: qsTr("Q")
        }

        Item { Layout.fillWidth: true }

        IconButton {
            objectName: bar.objectName + ".bypass"
            visible: bar.info.bypassSupported === true
            iconName: "power"
            iconSize: 12
            iconColor: bar.info.bypass ? Theme.textMuted : bar.ink
            toolTip: bar.info.powerTip || ""
            enabled: bar.editing
            onClicked: bar.editor.chipBypass()
        }
        IconButton {
            objectName: bar.objectName + ".invert"
            iconName: "arrow-down-up"
            iconSize: 12
            iconColor: Theme.text
            toolTip: bar.info.invertTip || ""
            enabled: bar.editing && bar.info.invertible === true
            onClicked: bar.editor.chipInvert()
        }
        IconButton {
            objectName: bar.objectName + ".delete"
            iconName: "trash"
            iconSize: 12
            iconColor: Theme.dangerText
            toolTip: bar.info.deleteTip || ""
            enabled: bar.editing
            onClicked: bar.editor.chipDelete()
        }
    }

    // ---- The shape chooser, opened from the shape button ----
    readonly property bool shapesWanted: editor !== null && editor.shapePage && band >= 0
    onShapesWantedChanged: shapesWanted ? shapes.open() : shapes.close()
    Popup {
        id: shapes
        objectName: bar.objectName + ".shapesPopup"
        parent: shapeButton
        y: shapeButton.height + 4
        width: 132
        height: 92
        padding: 0
        closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutsideParent
        onClosed: if (bar.editor && bar.editor.shapePage) bar.editor.chipToggleShapes()
        background: Rectangle {
            radius: Theme.radius
            color: Theme.menu
            border { width: 1; color: Theme.border }
        }
        ShapeChooser {
            id: chooser
            anchors.fill: parent
            editor: bar.editor
            namePrefix: bar.objectName + ".shapes"
            ownShape: bar.info.reshapable ? bar.info.shape : -1
            ownOrder: bar.info.order || 0
            markColor: bar.ink
            backHelp: qsTr("Back to values")
            onVisibleChanged: reset()
            onPicked: (shape, order) => bar.editor.chipPick(shape, order)
            onBack: bar.editor.chipToggleShapes()
        }
    }
}
