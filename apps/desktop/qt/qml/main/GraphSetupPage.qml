import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP3 -- The graph options' second page (the Mac's GraphSetupPage): the
// scale, the grid and labels, and the curves, adjustable while looking at the
// graph they change. Each control writes GraphController's preference at once.
ColumnLayout {
    id: page
    // Object names start with this (the gear's name)
    property string prefix: "GraphHost.options"
    // The pop-out's own follow switch, which means nothing in the main window
    property bool inPopOutWindow: false
    signal back()

    // Fixed so the slider rows line up whatever their labels and values say
    readonly property int labelWidth: 80
    readonly property int valueWidth: 46

    spacing: 0

    // A level with its sign, "+6" or "-12"
    function signedDb(v) {
        const r = Math.round(v)
        return r > 0 ? "+" + r : String(r)
    }

    // ---- Title bar: Back and the page's name ----
    Item {
        implicitHeight: 32
        Layout.fillWidth: true
        LinkButton {
            objectName: page.prefix + ".back"
            anchors { verticalCenter: parent.verticalCenter; left: parent.left; leftMargin: 12 }
            text: qsTr("Back")
            iconName: "chevron-left"
            onActivated: page.back()
        }
        Text {
            anchors.centerIn: parent
            text: qsTr("Graph Setup")
            color: Theme.text
            font { pixelSize: Theme.fontBody.pixelSize - 1; weight: Font.DemiBold }
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
    }
    Rectangle {
        color: Theme.separator
        implicitHeight: 1
        Layout.fillWidth: true
    }

    // ---- Scale ----
    Header {
        text: qsTr("SCALE")
        LinkButton {
            objectName: page.prefix + ".resetScale"
            text: qsTr("Reset")
            toolTip: qsTr("Restore the default frequency and dB range")
            onActivated: GraphController.resetScale()
        }
    }
    RowLayout {
        spacing: 6
        Layout.fillWidth: true
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        RowLabel {
            text: qsTr("Frequency")
            // Its own width, so the two pickers have room
            Layout.preferredWidth: implicitWidth
        }
        FreqPicker {
            objectName: page.prefix + ".minFreq"
            label: qsTr("Lowest frequency")
            model: [
                { text: qsTr("10 Hz"), hz: 10 },
                { text: qsTr("15 Hz"), hz: 15 },
                { text: qsTr("20 Hz"), hz: 20 },
                { text: qsTr("50 Hz"), hz: 50 },
                { text: qsTr("100 Hz"), hz: 100 }
            ]
            currentIndex: indexOfValue(GraphController.minFreq)
            onActivated: GraphController.minFreq = currentValue
        }
        Text {
            text: qsTr("to")
            color: Theme.textMuted
            font: Theme.fontSmall
        }
        FreqPicker {
            objectName: page.prefix + ".maxFreq"
            label: qsTr("Highest frequency")
            model: [
                { text: qsTr("5 kHz"), hz: 5000 },
                { text: qsTr("10 kHz"), hz: 10000 },
                { text: qsTr("20 kHz"), hz: 20000 }
            ]
            currentIndex: indexOfValue(GraphController.maxFreq)
            onActivated: GraphController.maxFreq = currentValue
        }
    }
    SliderRow {
        objectName: page.prefix + ".range"
        Layout.topMargin: 8
        text: qsTr("Range")
        slider { from: 10; to: 100; stepSize: 1; resetValue: 50; value: GraphController.dbRange }
        shown: qsTr("%1 dB").arg(Math.round(GraphController.dbRange))
        onChanged: v => GraphController.dbRange = Math.round(v)
    }
    SliderRow {
        objectName: page.prefix + ".center"
        Layout.topMargin: 8
        text: qsTr("Center")
        slider { from: -40; to: 20; stepSize: 1; resetValue: 0; value: GraphController.dbCenter }
        shown: qsTr("%1 dB").arg(page.signedDb(GraphController.dbCenter))
        onChanged: v => GraphController.dbCenter = Math.round(v)
    }

    // ---- Grid and labels ----
    Header { text: qsTr("GRID & LABELS") }
    ToggleRow {
        objectName: page.prefix + ".freqGrid"
        text: qsTr("Frequency Grid")
        checked: GraphController.showFrequencyGrid
        onToggled: GraphController.showFrequencyGrid = checked
    }
    ToggleRow {
        objectName: page.prefix + ".freqLabels"
        text: qsTr("Frequency Labels")
        checked: GraphController.showFrequencyLabels
        onToggled: GraphController.showFrequencyLabels = checked
    }
    ToggleRow {
        objectName: page.prefix + ".dbGrid"
        text: qsTr("dB Grid")
        checked: GraphController.showDBGrid
        onToggled: GraphController.showDBGrid = checked
    }
    ToggleRow {
        objectName: page.prefix + ".dbLabels"
        text: qsTr("dB Labels")
        checked: GraphController.showDBLabels
        onToggled: GraphController.showDBLabels = checked
    }
    ToggleRow {
        objectName: page.prefix + ".freqReadout"
        text: qsTr("Frequency Readout")
        checked: GraphController.showFrequencyReadout
        onToggled: GraphController.showFrequencyReadout = checked
    }
    ToggleRow {
        objectName: page.prefix + ".levelReadout"
        text: qsTr("Gain Readout")
        checked: GraphController.showLevelReadout
        onToggled: GraphController.showLevelReadout = checked
    }
    SliderRow {
        objectName: page.prefix + ".gridOpacity"
        Layout.topMargin: 2
        Layout.bottomMargin: 2
        enabled: GraphController.showFrequencyGrid || GraphController.showDBGrid
        text: qsTr("Grid Opacity")
        slider { from: 0; to: 2; resetValue: 0.5; value: GraphController.graphGridOpacity }
        shown: qsTr("%1%").arg(Math.round(GraphController.graphGridOpacity * 100))
        onChanged: v => GraphController.graphGridOpacity = v
    }

    // ---- Curves ----
    Header { text: qsTr("CURVES") }
    SliderRow {
        objectName: page.prefix + ".lineWidth"
        Layout.bottomMargin: 2
        text: qsTr("Line Width")
        slider { from: 1; to: 4; stepSize: 0.5; resetValue: 1.5; value: GraphController.graphLineWidth }
        shown: qsTr("%1 pt").arg(GraphController.graphLineWidth.toFixed(1))
        onChanged: v => GraphController.graphLineWidth = v
    }
    ToggleRow {
        objectName: page.prefix + ".glow"
        text: qsTr("Glow")
        checked: GraphController.showGraphGlow
        onToggled: GraphController.showGraphGlow = checked
    }
    ToggleRow {
        objectName: page.prefix + ".phase"
        text: qsTr("Phase Response")
        checked: GraphController.showPhase
        onToggled: GraphController.showPhase = checked
    }
    ToggleRow {
        objectName: page.prefix + ".unwrap"
        enabled: GraphController.showPhase
        text: qsTr("Unwrap Phase")
        checked: GraphController.phaseUnwrapped
        onToggled: GraphController.phaseUnwrapped = checked
    }
    ToggleRow {
        objectName: page.prefix + ".follow"
        visible: page.inPopOutWindow
        text: qsTr("Follow Channel Selection")
        checked: GraphController.popoutFollowsSelection
        onToggled: GraphController.popoutFollowsSelection = checked
    }
    Item { implicitHeight: 8 }

    // ---- The page's pieces ----

    // A section's small capitals, with an optional control on the right
    component Header: RowLayout {
        property alias text: caption.text
        default property alias trailing: tail.data
        Layout.fillWidth: true
        Layout.topMargin: 12
        Layout.bottomMargin: 6
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        Text {
            id: caption
            color: Theme.textMuted
            font { pixelSize: Theme.fontCaption.pixelSize; weight: Font.Bold }
            Layout.fillWidth: true
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Row { id: tail }
    }

    component RowLabel: Text {
        color: enabled ? Theme.text : Theme.textDisabled
        font: Theme.fontBody
        elide: Text.ElideRight
        Layout.preferredWidth: page.labelWidth
        Layout.maximumWidth: page.labelWidth
    }

    component FreqPicker: AppComboBox {
        textRole: "text"
        valueRole: "hz"
        Layout.fillWidth: true
        Layout.minimumWidth: 60
        Layout.preferredWidth: 74
    }

    component ToggleRow: AppSwitch {
        implicitHeight: 26
        Layout.fillWidth: true
        Layout.leftMargin: 10
        Layout.rightMargin: 10
    }

    // A labelled slider with its value shown; live while dragged. Its slider
    // is set through `slider { ... }`.
    component SliderRow: RowLayout {
        id: sliderRow
        property alias text: rowLabel.text
        property alias slider: control
        property string shown
        signal changed(real v)

        spacing: 6
        Layout.fillWidth: true
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        RowLabel { id: rowLabel }
        CustomSlider {
            id: control
            label: rowLabel.text
            implicitWidth: 100
            snapMode: stepSize > 0 ? Slider.SnapAlways : Slider.NoSnap
            Layout.fillWidth: true
            onValueMoved: v => sliderRow.changed(v)
            onCommitted: v => sliderRow.changed(v)
        }
        Text {
            text: sliderRow.shown
            color: Theme.textMuted
            font: Theme.fontMono
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: page.valueWidth
        }
    }

    // A borderless text button in the accent colour, an icon before it if given
    component LinkButton: Item {
        id: link
        property string iconName
        property string text
        property string toolTip
        signal activated()

        activeFocusOnTab: true
        implicitWidth: content.implicitWidth
        implicitHeight: Math.max(content.implicitHeight, 18)
        Accessible.role: Accessible.Button
        Accessible.name: text
        Accessible.onPressAction: activated()
        Keys.onReturnPressed: activated()
        Keys.onSpacePressed: activated()

        RowLayout {
            id: content
            anchors.fill: parent
            spacing: 3
            Icon {
                visible: link.iconName !== ""
                name: link.iconName
                color: Theme.accentText
                size: 12
            }
            Text {
                text: link.text
                color: Theme.accentText
                font: link.iconName !== "" ? Theme.fontBody : Theme.fontCaption
            }
        }
        // The keyboard focus ring
        Rectangle {
            anchors { fill: parent; margins: -2 }
            visible: link.activeFocus
            color: "transparent"
            radius: Theme.radiusSmall
            border { width: 2; color: Theme.focusRing }
        }
        HoverHandler {
            id: linkHover
            cursorShape: Qt.PointingHandCursor
        }
        TapHandler { onTapped: link.activated() }
        HelpTip {
            visible: linkHover.hovered && link.toolTip !== ""
            text: link.toolTip
        }
    }
}
