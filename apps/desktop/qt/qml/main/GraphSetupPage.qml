import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP3 -- The graph options' second page (the Mac's GraphSetupPage): the
// scale, the grid and labels, and the curves, adjustable while looking at the
// graph they change. Each control writes GraphController's preference at once.
ColumnLayout {
    id: page
    // The pop-out's own follow switch, which means nothing in the main window
    property bool inPopOutWindow: false
    property string prefix: "GraphHost.options"
    signal back()

    spacing: 0

    // Fixed so the slider rows line up whatever their labels and values say
    readonly property int labelWidth: 80
    readonly property int valueWidth: 46

    // ---- Title bar: Back and the page's name ----
    Item {
        Layout.fillWidth: true
        implicitHeight: 32
        Text {
            anchors.centerIn: parent
            text: qsTr("Graph Setup")
            font.pixelSize: Theme.fontBody.pixelSize - 1
            font.weight: Font.DemiBold
            color: Theme.text
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        LinkButton {
            objectName: page.prefix + ".back"
            anchors.left: parent.left
            anchors.leftMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            iconName: "chevron-left"
            text: qsTr("Back")
            onActivated: page.back()
        }
    }
    Rectangle {
        Layout.fillWidth: true
        implicitHeight: 1
        color: Theme.separator
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
        Layout.fillWidth: true
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        spacing: 6
        RowLabel {
            text: qsTr("Frequency")
            // Its own width, so the two pickers have room
            Layout.preferredWidth: implicitWidth
        }
        AppComboBox {
            objectName: page.prefix + ".minFreq"
            label: qsTr("Lowest frequency")
            Layout.fillWidth: true
            Layout.minimumWidth: 60
            Layout.preferredWidth: 74
            textRole: "text"
            valueRole: "hz"
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
            font: Theme.fontSmall
            color: Theme.textMuted
        }
        AppComboBox {
            objectName: page.prefix + ".maxFreq"
            label: qsTr("Highest frequency")
            Layout.fillWidth: true
            Layout.minimumWidth: 60
            Layout.preferredWidth: 74
            textRole: "text"
            valueRole: "hz"
            model: [
                { text: qsTr("5 kHz"), hz: 5000 },
                { text: qsTr("10 kHz"), hz: 10000 },
                { text: qsTr("20 kHz"), hz: 20000 }
            ]
            currentIndex: indexOfValue(GraphController.maxFreq)
            onActivated: GraphController.maxFreq = currentValue
        }
        Item { Layout.fillWidth: true }
    }
    SliderRow {
        objectName: page.prefix + ".range"
        Layout.topMargin: 8
        text: qsTr("Range")
        value: GraphController.dbRange
        from: 10
        to: 100
        stepSize: 1
        resetValue: 50
        shown: qsTr("%1 dB").arg(Math.round(GraphController.dbRange))
        onChanged: v => GraphController.dbRange = Math.round(v)
    }
    SliderRow {
        objectName: page.prefix + ".center"
        Layout.topMargin: 8
        text: qsTr("Center")
        value: GraphController.dbCenter
        from: -40
        to: 20
        stepSize: 1
        resetValue: 0
        shown: qsTr("%1 dB").arg((GraphController.dbCenter > 0 ? "+" : "") + Math.round(GraphController.dbCenter))
        onChanged: v => GraphController.dbCenter = Math.round(v)
    }

    // ---- Grid and labels ----
    Header { text: qsTr("GRID & LABELS") }
    ToggleRow {
        objectName: page.prefix + ".freqGrid"
        text: qsTr("Frequency Grid")
        checked: GraphController.showFreqGrid
        onToggled: GraphController.showFreqGrid = checked
    }
    ToggleRow {
        objectName: page.prefix + ".freqLabels"
        text: qsTr("Frequency Labels")
        checked: GraphController.showFreqLabels
        onToggled: GraphController.showFreqLabels = checked
    }
    ToggleRow {
        objectName: page.prefix + ".dbGrid"
        text: qsTr("dB Grid")
        checked: GraphController.showDbGrid
        onToggled: GraphController.showDbGrid = checked
    }
    ToggleRow {
        objectName: page.prefix + ".dbLabels"
        text: qsTr("dB Labels")
        checked: GraphController.showDbLabels
        onToggled: GraphController.showDbLabels = checked
    }
    ToggleRow {
        objectName: page.prefix + ".freqReadout"
        text: qsTr("Frequency Readout")
        checked: GraphController.freqReadout
        onToggled: GraphController.freqReadout = checked
    }
    ToggleRow {
        objectName: page.prefix + ".levelReadout"
        text: qsTr("Gain Readout")
        checked: GraphController.levelReadout
        onToggled: GraphController.levelReadout = checked
    }
    SliderRow {
        objectName: page.prefix + ".gridOpacity"
        Layout.topMargin: 2
        Layout.bottomMargin: 2
        text: qsTr("Grid Opacity")
        enabled: GraphController.showFreqGrid || GraphController.showDbGrid
        value: GraphController.gridOpacity
        from: 0
        to: 2
        resetValue: 0.5
        shown: qsTr("%1%").arg(Math.round(GraphController.gridOpacity * 100))
        onChanged: v => GraphController.gridOpacity = v
    }

    // ---- Curves ----
    Header { text: qsTr("CURVES") }
    SliderRow {
        objectName: page.prefix + ".lineWidth"
        Layout.bottomMargin: 2
        text: qsTr("Line Width")
        value: GraphController.lineWidth
        from: 1
        to: 4
        stepSize: 0.5
        resetValue: 1.5
        shown: qsTr("%1 pt").arg(GraphController.lineWidth.toFixed(1))
        onChanged: v => GraphController.lineWidth = v
    }
    ToggleRow {
        objectName: page.prefix + ".glow"
        text: qsTr("Glow")
        checked: GraphController.showGlow
        onToggled: GraphController.showGlow = checked
    }
    ToggleRow {
        objectName: page.prefix + ".phase"
        text: qsTr("Phase Response")
        checked: GraphController.showPhase
        onToggled: GraphController.showPhase = checked
    }
    ToggleRow {
        objectName: page.prefix + ".unwrap"
        text: qsTr("Unwrap Phase")
        enabled: GraphController.showPhase
        checked: GraphController.phaseUnwrapped
        onToggled: GraphController.phaseUnwrapped = checked
    }
    ToggleRow {
        objectName: page.prefix + ".follow"
        visible: page.inPopOutWindow
        text: qsTr("Follow Channel Selection")
        checked: GraphController.popOutFollows
        onToggled: GraphController.popOutFollows = checked
    }
    Item { implicitHeight: 8 }

    // ---- The page's pieces ----

    component Header: RowLayout {
        property alias text: title.text
        default property alias trailing: tail.data
        Layout.fillWidth: true
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        Layout.topMargin: 12
        Layout.bottomMargin: 6
        Text {
            id: title
            font.pixelSize: Theme.fontCaption.pixelSize
            font.weight: Font.Bold
            color: Theme.textMuted
            Layout.fillWidth: true
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Row { id: tail }
    }

    component RowLabel: Text {
        Layout.preferredWidth: page.labelWidth
        Layout.maximumWidth: page.labelWidth
        font: Theme.fontBody
        color: enabled ? Theme.text : Theme.textDisabled
        elide: Text.ElideRight
    }

    component ToggleRow: AppSwitch {
        Layout.fillWidth: true
        Layout.leftMargin: 10
        Layout.rightMargin: 10
        implicitHeight: 26
    }

    // A labelled slider with its value; live while dragged
    component SliderRow: RowLayout {
        id: sliderRow
        property alias text: rowLabel.text
        property alias value: slider.value
        property alias from: slider.from
        property alias to: slider.to
        property alias stepSize: slider.stepSize
        property alias resetValue: slider.resetValue
        property string shown
        signal changed(real v)

        Layout.fillWidth: true
        Layout.leftMargin: 12
        Layout.rightMargin: 12
        spacing: 6
        RowLabel { id: rowLabel }
        CustomSlider {
            id: slider
            label: rowLabel.text
            Layout.fillWidth: true
            implicitWidth: 100
            snapMode: stepSize > 0 ? Slider.SnapAlways : Slider.NoSnap
            onValueMoved: v => sliderRow.changed(v)
            onCommitted: v => sliderRow.changed(v)
        }
        Text {
            text: sliderRow.shown
            font: Theme.fontMono
            color: Theme.textMuted
            horizontalAlignment: Text.AlignRight
            Layout.preferredWidth: page.valueWidth
        }
    }

    // A borderless accent-coloured text button, optionally with an icon
    component LinkButton: Item {
        id: link
        property string text
        property string iconName
        property string toolTip
        signal activated()

        implicitWidth: linkRow.implicitWidth
        implicitHeight: Math.max(linkRow.implicitHeight, 18)
        activeFocusOnTab: true
        Accessible.role: Accessible.Button
        Accessible.name: text
        Accessible.onPressAction: activated()
        Keys.onReturnPressed: activated()
        Keys.onSpacePressed: activated()

        Row {
            id: linkRow
            anchors.verticalCenter: parent.verticalCenter
            spacing: 3
            Icon {
                visible: link.iconName !== ""
                name: link.iconName
                size: 12
                color: Theme.accentText
                anchors.verticalCenter: parent.verticalCenter
            }
            Text {
                text: link.text
                font: link.iconName !== "" ? Theme.fontBody : Theme.fontCaption
                color: Theme.accentText
                anchors.verticalCenter: parent.verticalCenter
            }
        }
        Rectangle {
            anchors.fill: parent
            anchors.margins: -2
            color: "transparent"
            radius: Theme.radiusSmall
            border.width: link.activeFocus ? 2 : 0
            border.color: Theme.focusRing
        }
        HoverHandler { id: linkHover; cursorShape: Qt.PointingHandCursor }
        TapHandler { onTapped: link.activated() }
        HelpTip {
            visible: linkHover.hovered && link.toolTip !== ""
            text: link.toolTip
        }
    }
}
