import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP9 -- The signal generator (the Mac's TestSignalsView, 780×500, at
// least 740×500): the header with the state, then two columns (the signal
// tiles and the outputs on the left; level, parameters, timing and options
// scrolling on the right), and the transport at the bottom. Space starts it
// and stops it with a fade. Closing the window leaves the generator playing.
ToolWindow {
    id: window
    objectName: "SiggenWindow"
    windowId: "signalGenerator"
    defaultWidth: 780
    defaultHeight: 500
    minimumWidth: 740
    minimumHeight: 500
    title: qsTr("Signal Generator")

    readonly property var gen: SiggenController
    readonly property bool enabledControls: SiggenController.controlsEnabled
    readonly property var currentType: SiggenController.types[SiggenController.signalType]

    onVisibleChanged: SiggenController.windowShown = visible
    Component.onCompleted: SiggenController.windowShown = visible

    // Space: Start, or the faded Stop (a focused text field keeps it)
    Shortcut {
        sequence: "Space"
        onActivated: SiggenController.toggle()
    }

    component Heading: Text {
        font: Theme.fontHeading
        color: Theme.textMuted
    }
    component Caption: Text {
        font: Theme.fontCaption
        color: Theme.textMuted
        wrapMode: Text.WordWrap
        Layout.fillWidth: true
    }
    // A timing or parameter row: label (and caption) left, field right
    component FieldRow: RowLayout {
        id: fieldRow
        property string label
        property string caption
        default property alias field: holder.data
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        ColumnLayout {
            spacing: 1
            Layout.fillWidth: true
            Text {
                text: fieldRow.label
                font: Theme.fontBody
                color: Theme.text
            }
            Caption {
                text: fieldRow.caption
                visible: text.length > 0
            }
        }
        Item {
            id: holder
            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
        }
    }
    component OptionRow: RowLayout {
        id: optionRow
        property string label
        property string caption
        property alias checked: toggle.checked
        property alias switchEnabled: toggle.enabled
        property alias toggleName: toggle.objectName
        signal toggled(bool on)
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        ColumnLayout {
            spacing: 1
            Layout.fillWidth: true
            Text {
                text: optionRow.label
                font: Theme.fontBody
                color: Theme.text
            }
            Caption { text: optionRow.caption }
        }
        AppSwitch {
            id: toggle
            Accessible.name: optionRow.label
            onToggled: optionRow.toggled(checked)
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // ---- Header ----
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
            Layout.topMargin: Theme.spacing
            Layout.bottomMargin: Theme.spacing
            spacing: Theme.spacing
            Icon {
                name: "audio-waveform"
                color: Theme.accentText
                size: 22
            }
            ColumnLayout {
                spacing: 0
                Text {
                    text: qsTr("Signal Generator")
                    font: Theme.fontTitle
                    color: Theme.text
                }
                Text {
                    text: qsTr("Onboard test and measurement signals")
                    font: Theme.fontCaption
                    color: Theme.textMuted
                }
            }
            Item { Layout.fillWidth: true }
            // The state pill: a dot in the state's colour, pulsing while it plays
            Rectangle {
                id: pill
                objectName: "SiggenWindow.state"
                readonly property color tint: SiggenController.phase === SiggenController.FadeIn
                                              || SiggenController.phase === SiggenController.Run ? Theme.success
                                            : SiggenController.phase === SiggenController.Gap
                                              || SiggenController.phase === SiggenController.FadeOut ? Theme.warning
                                            : Theme.textMuted
                property string shownName: SiggenController.phaseText
                implicitWidth: pillRow.implicitWidth + 20
                implicitHeight: pillRow.implicitHeight + 10
                radius: height / 2
                color: Theme.control
                border.color: Qt.alpha(tint, SiggenController.playing ? 0.5 : 0.2)
                Accessible.role: Accessible.StaticText
                Accessible.name: shownName
                RowLayout {
                    id: pillRow
                    anchors.centerIn: parent
                    spacing: 6
                    Rectangle {
                        width: 8
                        height: 8
                        radius: 4
                        color: pill.tint
                        SequentialAnimation on scale {
                            running: SiggenController.playing
                            loops: Animation.Infinite
                            alwaysRunToEnd: true
                            NumberAnimation { from: 1.0; to: 0.75; duration: 700; easing.type: Easing.InOutQuad }
                            NumberAnimation { from: 0.75; to: 1.0; duration: 700; easing.type: Easing.InOutQuad }
                        }
                    }
                    Text {
                        text: pill.shownName
                        font: Qt.font({ pixelSize: Theme.fontCaption.pixelSize, weight: Font.DemiBold })
                        color: SiggenController.playing ? Theme.text : Theme.textMuted
                    }
                }
            }
        }
        Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: Theme.separator }

        // ---- Unsupported ----
        ColumnLayout {
            objectName: "SiggenWindow.unsupported"
            visible: SiggenController.connected && !SiggenController.available
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: Theme.spacingSmall
            Item { Layout.fillHeight: true }
            Icon {
                Layout.alignment: Qt.AlignHCenter
                name: "audio-waveform"
                color: Theme.textMuted
                size: 34
            }
            Text {
                Layout.alignment: Qt.AlignHCenter
                text: qsTr("Signal generator not available")
                font: Theme.fontTitle
                color: Theme.text
            }
            Text {
                Layout.fillWidth: true
                Layout.leftMargin: 40
                Layout.rightMargin: 40
                horizontalAlignment: Text.AlignHCenter
                wrapMode: Text.WordWrap
                text: qsTr("The connected firmware does not include the onboard test signal generator. Update the DSPi firmware to use this tool.")
                font: Theme.fontSmall
                color: Theme.textMuted
            }
            Item { Layout.fillHeight: true }
        }

        // ---- The two columns, sharing the width ----
        RowLayout {
            id: columns
            readonly property real half: (width - 1) / 2
            visible: !(SiggenController.connected && !SiggenController.available)
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            // Left: the signal and the outputs
            ColumnLayout {
                Layout.preferredWidth: columns.half - 2 * Theme.spacingLarge
                Layout.maximumWidth: columns.half - 2 * Theme.spacingLarge
                Layout.fillHeight: true
                Layout.leftMargin: Theme.spacingLarge
                Layout.rightMargin: Theme.spacingLarge
                Layout.topMargin: 14
                Layout.bottomMargin: 14
                spacing: Theme.spacingLarge

                ColumnLayout {
                    spacing: 6
                    Layout.fillWidth: true
                    Heading { text: qsTr("SIGNAL") }
                    GridLayout {
                        Layout.fillWidth: true
                        columns: 5
                        rowSpacing: 6
                        columnSpacing: 6
                        Repeater {
                            model: SiggenController.types
                            SiggenTile {
                                required property var modelData
                                objectName: "SiggenWindow.signal." + modelData.id
                                Layout.fillWidth: true
                                type: modelData.id
                                text: modelData.label
                                toolTip: modelData.blurb
                                chosen: SiggenController.signalType === modelData.id
                                enabled: window.enabledControls
                                onClicked: SiggenController.selectType(modelData.id)
                            }
                        }
                    }
                    Caption {
                        objectName: "SiggenWindow.blurb"
                        text: window.currentType ? window.currentType.blurb : ""
                    }
                }

                Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: Theme.separator }

                ColumnLayout {
                    spacing: 6
                    Layout.fillWidth: true
                    RowLayout {
                        Layout.fillWidth: true
                        Heading { text: qsTr("OUTPUTS") }
                        Item { Layout.fillWidth: true }
                        AppButton {
                            objectName: "SiggenWindow.all"
                            text: qsTr("All")
                            enabled: window.enabledControls
                            onClicked: SiggenController.allOutputs()
                        }
                        AppButton {
                            objectName: "SiggenWindow.none"
                            text: qsTr("None")
                            enabled: window.enabledControls
                            onClicked: SiggenController.noOutputs()
                        }
                    }
                    GridLayout {
                        Layout.fillWidth: true
                        columns: 3
                        rowSpacing: 6
                        columnSpacing: 6
                        Repeater {
                            model: SiggenController.outputCount
                            SiggenOutputChip {
                                required property int index
                                objectName: "SiggenWindow.output." + index
                                Layout.fillWidth: true
                                text: { DeviceModel.revision; return SiggenController.outputName(index) }
                                selected: (SiggenController.channelMask & (1 << index)) !== 0
                                inverted: selected && (SiggenController.invertMask & (1 << index)) !== 0
                                walking: SiggenController.playing && SiggenController.walkOutput === index
                                matrixEnabled: { DeviceModel.revision; return SiggenController.outputEnabled(index) }
                                enabled: window.enabledControls
                                onClicked: SiggenController.stepOutput(index)
                            }
                        }
                    }
                    Caption {
                        text: qsTr("Click to select, click again to invert polarity (ø). Dimmed outputs are disabled in the matrix mixer and stay silent.")
                    }
                }
                Item { Layout.fillHeight: true }
            }

            Rectangle { Layout.fillHeight: true; implicitWidth: 1; color: Theme.separator }

            // Right: level, parameters, timing, options
            Flickable {
                id: right
                Layout.preferredWidth: columns.half
                Layout.fillWidth: true
                Layout.fillHeight: true
                contentWidth: width
                contentHeight: rightColumn.implicitHeight + 28
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: AppScrollBar {}

                ColumnLayout {
                    id: rightColumn
                    x: Theme.spacingLarge
                    y: 14
                    width: right.width - 2 * Theme.spacingLarge
                    spacing: Theme.spacingLarge

                    // Level
                    ColumnLayout {
                        spacing: 6
                        Layout.fillWidth: true
                        RowLayout {
                            Layout.fillWidth: true
                            Heading { text: qsTr("LEVEL") }
                            Item { Layout.fillWidth: true }
                            ChannelValueField {
                                objectName: "SiggenWindow.levelField"
                                label: qsTr("Level")
                                unit: qsTr("dB")
                                value: SiggenController.levelDb
                                maxDecimals: 1
                                scrollStep: 1
                                minValue: -120
                                maxValue: 0
                                enabled: window.enabledControls
                                onCommitted: v => SiggenController.setLevel(v, false)
                            }
                        }
                        CustomSlider {
                            objectName: "SiggenWindow.level"
                            label: qsTr("Level")
                            Layout.fillWidth: true
                            from: -80
                            to: 0
                            value: SiggenController.levelDb
                            enabled: window.enabledControls
                            onValueMoved: v => SiggenController.setLevel(v, true)
                            onCommitted: v => SiggenController.setLevel(v, true)
                        }
                        Caption {
                            text: qsTr("Peak level in dBFS. Output trim, master volume and mute still apply downstream.")
                        }
                    }

                    // Parameters (types with none show no section)
                    ColumnLayout {
                        id: paramSection
                        readonly property var descs: window.currentType ? window.currentType.params : []
                        readonly property bool any: descs.some(p => p.semantic !== SiggenController.Unused)
                        visible: any
                        spacing: 6
                        Layout.fillWidth: true
                        Heading { text: qsTr("PARAMETERS") }

                        // The ISP test: its two patterns
                        AppTabBar {
                            objectName: "SiggenWindow.isp"
                            visible: SiggenController.signalType === SiggenController.Isp
                            Layout.fillWidth: true
                            enabled: window.enabledControls
                            currentIndex: Math.round(SiggenController.params[0])
                            ToolTip.visible: hovered
                            ToolTip.text: qsTr("Sample-peak-normalized sequences with a known inter-sample true-peak over. Set the level at or below the headroom the over should fit into.")
                            AppTabButton {
                                text: qsTr("fs/4 · +3.01 dBTP")
                                onClicked: SiggenController.setParameter(0, 0)
                            }
                            AppTabButton {
                                text: qsTr("fs/6 · +1.25 dBTP")
                                onClicked: SiggenController.setParameter(0, 1)
                            }
                        }
                        Repeater {
                            model: SiggenController.signalType === SiggenController.Isp ? [] : paramSection.descs
                            FieldRow {
                                required property var modelData
                                required property int index
                                visible: modelData.semantic !== SiggenController.Unused
                                label: modelData.label
                                ChannelValueField {
                                    readonly property bool whole: modelData.semantic === SiggenController.Count
                                                                  || modelData.semantic === SiggenController.Cycles
                                    objectName: "SiggenWindow.param." + index
                                    label: modelData.label
                                    unit: SiggenController.unitFor(modelData.semantic)
                                    fieldWidth: 66
                                    value: SiggenController.params[index]
                                    maxDecimals: whole ? 0 : modelData.semantic === SiggenController.Ratio ? 2 : 1
                                    stripTrailingZeros: true
                                    scrollStep: modelData.semantic === SiggenController.Ratio ? 0.1 : 1
                                    minValue: modelData.min
                                    maxValue: modelData.max > 0 ? modelData.max : Infinity
                                    enabled: window.enabledControls
                                    onCommitted: v => SiggenController.setParameter(index, v)
                                }
                            }
                        }
                        RowLayout {
                            visible: SiggenController.signalType === SiggenController.TonePair
                            spacing: Theme.spacingSmall
                            Text {
                                text: qsTr("Presets:")
                                font: Theme.fontSmall
                                color: Theme.textMuted
                            }
                            AppButton {
                                objectName: "SiggenWindow.presetSmpte"
                                text: qsTr("SMPTE 60/7k")
                                enabled: window.enabledControls
                                onClicked: SiggenController.applyPreset("smpte")
                            }
                            AppButton {
                                objectName: "SiggenWindow.presetCcif"
                                text: qsTr("CCIF 19k/20k")
                                enabled: window.enabledControls
                                onClicked: SiggenController.applyPreset("ccif")
                            }
                        }
                        Caption {
                            visible: SiggenController.signalType === SiggenController.Multitone && SiggenController.mostTones > 0
                            text: qsTr("Up to %1 tones on this device.").arg(SiggenController.mostTones)
                        }
                    }

                    // Timing, by the type's timing model
                    ColumnLayout {
                        spacing: 6
                        Layout.fillWidth: true
                        readonly property int timing: SiggenController.timing
                        readonly property bool sweep: timing === SiggenController.Sweep
                        readonly property bool pattern: timing === SiggenController.Pattern
                        readonly property bool walk: timing === SiggenController.Continuous && SiggenController.walking
                        readonly property bool plain: timing === SiggenController.Continuous && !SiggenController.walking
                        id: timingSection
                        Heading { text: qsTr("TIMING") }

                        FieldRow {
                            visible: timingSection.sweep || timingSection.walk || timingSection.plain
                            label: timingSection.sweep ? qsTr("Sweep length") : timingSection.walk ? qsTr("Dwell per channel") : qsTr("Duration")
                            caption: timingSection.walk ? qsTr("0 = 2 s default") : timingSection.plain ? qsTr("0 = play until stopped") : ""
                            ChannelValueField {
                                objectName: "SiggenWindow.duration"
                                label: qsTr("Duration")
                                unit: qsTr("s")
                                fieldWidth: 66
                                value: SiggenController.durationSeconds
                                maxDecimals: 2
                                stripTrailingZeros: true
                                scrollStep: 0.5
                                minValue: timingSection.sweep ? 0.01 : 0
                                enabled: window.enabledControls
                                onCommitted: v => SiggenController.setDurationSeconds(v, timingSection.sweep ? 0.01 : 0)
                            }
                        }
                        FieldRow {
                            visible: timingSection.sweep || timingSection.pattern || timingSection.walk
                            label: timingSection.walk ? qsTr("Passes") : qsTr("Repeat")
                            caption: timingSection.sweep ? qsTr("0 = repeat forever")
                                   : timingSection.walk ? qsTr("Full passes over the outputs. 0 = forever")
                                   : SiggenController.signalType === SiggenController.ChannelId
                                     ? qsTr("Passes over the selected outputs. 0 = forever")
                                     : qsTr("Pattern periods. 0 = repeat forever")
                            ChannelValueField {
                                objectName: "SiggenWindow.repeat"
                                label: qsTr("Repeat")
                                fieldWidth: 66
                                value: SiggenController.repeat
                                maxDecimals: 0
                                scrollStep: 1
                                minValue: 0
                                maxValue: 65535
                                enabled: window.enabledControls
                                onCommitted: v => SiggenController.setRepeat(v)
                            }
                        }
                        FieldRow {
                            visible: timingSection.sweep || timingSection.pattern
                            label: timingSection.sweep ? qsTr("Gap between sweeps") : qsTr("Extra gap per period")
                            ChannelValueField {
                                objectName: "SiggenWindow.gap"
                                label: qsTr("Gap")
                                unit: qsTr("ms")
                                fieldWidth: 66
                                value: SiggenController.gapMs
                                maxDecimals: 0
                                scrollStep: 50
                                minValue: 0
                                maxValue: 65535
                                enabled: window.enabledControls
                                onCommitted: v => SiggenController.setGapMs(v)
                            }
                        }
                    }

                    Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: Theme.separator }

                    // Options
                    ColumnLayout {
                        spacing: Theme.spacingSmall
                        Layout.fillWidth: true
                        Heading { text: qsTr("OPTIONS") }
                        OptionRow {
                            toggleName: "SiggenWindow.raw"
                            label: qsTr("Bypass output EQ (RAW)")
                            caption: qsTr("Skips crossover and PEQ on the selected outputs. Trim, master volume, mute and delay still apply.")
                            checked: (SiggenController.flags & 1) !== 0
                            switchEnabled: window.enabledControls
                            onToggled: on => SiggenController.setOption(1, on)
                        }
                        OptionRow {
                            toggleName: "SiggenWindow.decorrelate"
                            visible: SiggenController.signalType === SiggenController.White
                                     || SiggenController.signalType === SiggenController.Pink
                            label: qsTr("Decorrelate channels")
                            caption: qsTr("Independent noise per output instead of one copied signal.")
                            checked: (SiggenController.flags & 2) !== 0
                            switchEnabled: window.enabledControls
                            onToggled: on => SiggenController.setOption(2, on)
                        }
                        OptionRow {
                            readonly property bool channelId: SiggenController.signalType === SiggenController.ChannelId
                            toggleName: "SiggenWindow.walk"
                            label: qsTr("Walk outputs one at a time")
                            caption: channelId ? qsTr("Channel ID always walks: each output plays its channel number as counted blips.")
                                               : qsTr("Plays the selected outputs sequentially instead of together.")
                            checked: channelId || (SiggenController.flags & 4) !== 0
                            switchEnabled: window.enabledControls && !channelId
                            onToggled: on => SiggenController.setOption(4, on)
                        }
                    }
                }
            }
        }

        Rectangle { Layout.fillWidth: true; implicitHeight: 1; color: Theme.separator }

        // ---- Transport ----
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
            Layout.topMargin: 10
            Layout.bottomMargin: 10
            spacing: 10
            ColumnLayout {
                spacing: 1
                Layout.fillWidth: true
                Text {
                    objectName: "SiggenWindow.title"
                    property string shownName: text
                    text: SiggenController.transportTitle
                    font: Qt.font({ pixelSize: Theme.fontSmall.pixelSize, weight: Font.Medium })
                    color: Theme.text
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
                Text {
                    objectName: "SiggenWindow.detail"
                    property string shownName: text
                    text: SiggenController.transportDetail
                    font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontCaption.pixelSize })
                    color: Theme.textMuted
                    elide: Text.ElideRight
                    Layout.fillWidth: true
                }
            }
            IconButton {
                objectName: "SiggenWindow.stopNow"
                visible: SiggenController.playing
                iconName: "square"
                toolTip: qsTr("Stop immediately, no fade")
                enabled: window.enabledControls
                onClicked: SiggenController.stopPlaying(true)
            }
            AppButton {
                objectName: "SiggenWindow.stop"
                visible: SiggenController.playing
                text: qsTr("Stop")
                iconName: "circle-x"
                destructive: true
                enabled: window.enabledControls
                onClicked: SiggenController.stopPlaying(false)
            }
            AppButton {
                objectName: "SiggenWindow.start"
                visible: !SiggenController.playing
                text: qsTr("Start")
                iconName: "play"
                primary: true
                enabled: SiggenController.startBlocker === "" && !DeviceModel.readOnly
                toolTip: SiggenController.startBlocker !== "" ? SiggenController.startBlocker : qsTr("Start the generator")
                onClicked: SiggenController.play()
            }
        }
    }
}
