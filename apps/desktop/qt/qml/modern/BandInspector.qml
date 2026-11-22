import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY4 -- The selected band in the inspector: every shape laid out flat
// (Classic's two-step shape chooser), its slope, frequency, gain and Q, and on,
// invert and reset. A band that is off takes a shape here. The values follow a
// drag on the graph.
ColumnLayout {
    id: band
    property int channel: -1
    property int index: -1
    property bool includeLinkwitz: false
    // How many bands the graph has selected
    property int selected: 0
    readonly property var params: ChannelController.revision < 0 || channel < 0 || index < 0 ? null : ChannelController.band(channel, index, false)
    readonly property int type: params ? params.type : 0
    readonly property bool active: type !== 0
    readonly property bool linkwitz: active && type === ChannelController.linkwitzType
    readonly property var live: PeqSelection.revision < 0 || index < 0 || !fromGraph ? null : PeqSelection.live(index)
    readonly property bool fromGraph: PeqEditorController.editedChannel === channel
    readonly property var groups: ChannelController.revision < 0 ? [] : ChannelController.typeMenu(includeLinkwitz).filter(g => g.variants[0].type !== 0)
    readonly property var group: groups.find(g => g.variants.some(v => v.type === type)) || null
    readonly property bool writable: ChannelController.writable

    function setValue(field, v) { ChannelController.setBandValue(channel, index, false, field, v) }
    // The shape of `g`, at the order the band has where it can
    function choose(g) {
        const at = group ? group.variants.findIndex(v => v.type === type) : -1
        const first = g.variants.length === 1 ? 0 : at >= 0 && group.variants.length > 1 ? at : g.variants.length - 1
        ChannelController.setBandType(channel, index, false, g.variants[first].type)
    }
    // Enter on the graph, or a double-click on a dot: Freq 0, Gain 1, Q 2
    function typeValues(field) {
        const rows = [freqRow, gainRow, qRow]
        const row = rows[field] && rows[field].visible ? rows[field] : freqRow
        if (row.visible) row.take()
    }

    spacing: 6

    Text {
        objectName: "BandInspector.none"
        visible: band.index < 0
        Layout.fillWidth: true
        text: qsTr("No band selected. Click a dot or a row in the table, or double-click the graph to add one.")
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
    Text {
        objectName: "BandInspector.off"
        visible: band.index >= 0 && !band.active
        Layout.fillWidth: true
        text: qsTr("This band is off. Pick a shape to use it.")
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
    Text {
        objectName: "BandInspector.several"
        visible: band.selected > 1
        Layout.fillWidth: true
        text: qsTr("%1 bands are selected. This one is shown; on the graph they move together.").arg(band.selected)
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }

    // ---- The shapes ----
    GridLayout {
        visible: band.index >= 0
        Layout.fillWidth: true
        columns: 4
        columnSpacing: 4
        rowSpacing: 4
        Repeater {
            model: band.groups
            AbstractButton {
                id: shape
                required property var modelData
                readonly property bool marked: band.group !== null && band.group.name === modelData.name
                readonly property int glyph: PeqEditorController.shapeOfType(modelData.variants[0].type)
                objectName: "BandInspector.shape." + modelData.variants[modelData.variants.length - 1].type
                Layout.fillWidth: true
                Layout.preferredWidth: 1
                implicitHeight: 40
                enabled: band.writable
                hoverEnabled: true
                focusPolicy: Qt.TabFocus
                onClicked: band.choose(modelData)
                contentItem: Item {
                    PeqShapeGlyph {
                        visible: shape.glyph >= 0
                        anchors { horizontalCenter: parent.horizontalCenter; top: parent.top; topMargin: 2 }
                        width: 24
                        height: 14
                        shape: shape.glyph >= 0 ? shape.glyph : 0
                        ink: shape.marked ? Theme.accentText : Theme.textMuted
                    }
                    Text {
                        visible: shape.glyph < 0
                        anchors { horizontalCenter: parent.horizontalCenter; top: parent.top; topMargin: 1 }
                        text: "LT"
                        font: Theme.fontSmall
                        color: shape.marked ? Theme.accentText : Theme.textMuted
                    }
                    Text {
                        anchors { left: parent.left; right: parent.right; bottom: parent.bottom }
                        text: shape.modelData.name
                        font: Theme.fontCaption
                        color: shape.marked ? Theme.accentText : Theme.textMuted
                        horizontalAlignment: Text.AlignHCenter
                        elide: Text.ElideRight
                    }
                }
                background: Rectangle {
                    radius: Theme.radiusSmall
                    color: shape.hovered ? Theme.controlHover : Theme.control
                    border.width: shape.visualFocus ? 2 : shape.marked ? 1 : 0
                    border.color: shape.visualFocus ? Theme.focusRing : Theme.accent
                }
                HelpTip {
                    text: shape.modelData.name
                    visible: shape.hovered
                }
                Accessible.role: Accessible.RadioButton
                Accessible.name: modelData.name
                Accessible.checkable: true
                Accessible.checked: marked
                Accessible.onPressAction: shape.clicked()
            }
        }
    }
    ToolSegments {
        objectName: "BandInspector.slope"
        visible: band.active && band.group !== null && band.group.variants.length > 1
        Layout.fillWidth: true
        enabled: band.writable
        label: qsTr("Slope")
        model: band.group ? band.group.variants.map(v => v.label) : []
        current: band.group ? band.group.variants.findIndex(v => v.type === band.type) : -1
        onChosen: i => ChannelController.setBandType(band.channel, band.index, false, band.group.variants[i].type)
    }

    // ---- The values ----
    component ValueRow: RowLayout {
        id: row
        property string title
        property string note: ""
        property alias unit: field.unit
        property alias value: field.value
        property alias liveValue: field.liveValue
        property alias scrollStep: field.scrollStep
        property alias minValue: field.minValue
        property alias maxDecimals: field.maxDecimals
        property alias stripTrailingZeros: field.stripTrailingZeros
        signal committed(real value)
        function take() { field.take() }
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 0
            Text {
                Layout.fillWidth: true
                text: row.title
                font: Theme.fontBody
                color: Theme.text
            }
            Text {
                visible: row.note.length > 0
                text: row.note
                font: Theme.fontCaption
                color: Theme.textMuted
            }
        }
        ChannelValueField {
            id: field
            function take() {
                const input = children[0]
                input.forceActiveFocus(Qt.TabFocusReason)
            }
            objectName: row.objectName
            Layout.fillWidth: false
            label: qsTr("Band %1: %2").arg(band.index + 1).arg(row.title)
            enabled: band.writable
            fieldWidth: 84
            onCommitted: v => row.committed(v)
        }
    }
    ValueRow {
        id: freqRow
        objectName: "BandInspector.freq"
        visible: band.active && !band.linkwitz
        title: qsTr("Frequency")
        unit: qsTr("Hz")
        scrollStep: 10
        minValue: 10
        value: band.params ? band.params.freq : 0
        liveValue: band.live ? band.live.freq : NaN
        onCommitted: v => band.setValue("freq", v)
    }
    ValueRow {
        id: gainRow
        objectName: "BandInspector.gain"
        visible: band.active && !band.linkwitz && ChannelController.usesGain(band.type)
        title: qsTr("Gain")
        unit: qsTr("dB")
        maxDecimals: 3
        value: band.params ? band.params.gain : 0
        liveValue: band.live ? band.live.gain : NaN
        onCommitted: v => band.setValue("gain", v)
    }
    ValueRow {
        id: qRow
        objectName: "BandInspector.q"
        readonly property real shownQ: band.live ? band.live.q : band.params ? band.params.q : 1
        visible: band.active && !band.linkwitz && ChannelController.usesQ(band.type)
        title: qsTr("Q")
        //: %1 is a number of octaves
        note: band.group && band.group.variants.length === 1 && shownQ > 0
              ? qsTr("Bandwidth %1 oct").arg((2 / Math.LN2 * Math.asinh(1 / (2 * shownQ))).toFixed(2)) : ""
        maxDecimals: 3
        stripTrailingZeros: true
        minValue: 0.1
        value: band.params ? band.params.q : 1
        liveValue: band.live ? band.live.q : NaN
        onCommitted: v => band.setValue("q", v)
    }
    Text {
        visible: band.linkwitz
        Layout.fillWidth: true
        text: qsTr("Its values are under Linkwitz Transform below.")
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }

    RowLayout {
        visible: band.active
        Layout.fillWidth: true
        AppSwitch {
            objectName: "BandInspector.on"
            visible: ChannelController.bypassSupported
            enabled: band.writable
            text: qsTr("Band on")
            checked: band.params !== null && !band.params.bypass
            onToggled: {
                ChannelController.setBandBypass(band.channel, band.index, false, !checked)
                checked = Qt.binding(() => band.params !== null && !band.params.bypass)
            }
        }
        Item { Layout.fillWidth: true }
        AppButton {
            objectName: "BandInspector.invert"
            visible: !band.linkwitz && ChannelController.usesGain(band.type)
            enabled: band.writable
            text: qsTr("Invert")
            toolTip: qsTr("Invert Gain")
            onClicked: band.setValue("gain", -band.params.gain)
        }
        AppButton {
            objectName: "BandInspector.reset"
            enabled: band.writable
            text: qsTr("Reset")
            toolTip: qsTr("Switch this band off and clear its values")
            onClicked: ChannelController.clearBand(band.channel, band.index, false)
        }
    }
}
