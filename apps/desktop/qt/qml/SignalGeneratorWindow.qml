import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import QtQuick.Window 2.15
import "components"

// Signal Generator: the device's onboard test signals, after the macOS
// Console. Pick a signal and outputs (click again to invert polarity), set
// the level, parameters and timing, then Start. While it plays, edits apply
// live. Closing the window leaves a running signal playing.
AppWindow {
    id: win
    title: qsTr("Signal Generator")
    visible: false
    width: 780
    height: 600 + titlebarHeight
    minimumWidth: 720
    minimumHeight: 460 + titlebarHeight

    onVisibleChanged: siggen.watching = visible

    readonly property var d: siggen.draft
    readonly property var info: (siggen.supported, siggen.typeInfo(d.type))
    readonly property int timing: info.timing
    readonly property bool walking: d.type === 14 || (d.flags & 4) !== 0
    readonly property bool textFocused: activeFocusItem !== null && activeFocusItem.selectedText !== undefined
    property int rev: 0
    Connections {
        target: bridge
        function onStateChanged() { win.rev++ }
        function onOutputLabelsChanged() { win.rev++ }   // -- BUG1 --
    }

    // Display names and help, by firmware type id (as on macOS)
    readonly property var types: [
        { tile: qsTr("Sine"), name: qsTr("Sine"), blurb: qsTr("Pure tone, THD approx -139 dB"), labels: [qsTr("Frequency")] },
        { tile: qsTr("Square"), name: qsTr("Square wave"), blurb: qsTr("Band-limited (polyBLEP) square"), labels: [qsTr("Frequency")] },
        { tile: qsTr("White"), name: qsTr("White noise"), blurb: qsTr("Uniform white noise"), labels: [] },
        { tile: qsTr("Pink"), name: qsTr("Pink noise"), blurb: qsTr("-3 dB/oct, level-safe normalized"), labels: [] },
        { tile: qsTr("Log Swp"), name: qsTr("Log sweep"), blurb: qsTr("Exponential sweep for room measurement"), labels: [qsTr("Start", "sweep start frequency"), qsTr("End")] },
        { tile: qsTr("Lin Swp"), name: qsTr("Linear sweep"), blurb: qsTr("Linear frequency sweep"), labels: [qsTr("Start", "sweep start frequency"), qsTr("End")] },
        { tile: qsTr("Step Swp"), name: qsTr("Stepped sweep"), blurb: qsTr("Discrete tones stepping up the band"), labels: [qsTr("Start", "sweep start frequency"), qsTr("End"), qsTr("Steps/octave"), qsTr("Dwell")] },
        { tile: qsTr("Impulse"), name: qsTr("Impulse"), blurb: qsTr("Single-sample unit impulses"), labels: [qsTr("Period")] },
        { tile: qsTr("Clicks"), name: qsTr("Alternating clicks"), blurb: qsTr("Clicks with alternating polarity"), labels: [qsTr("Period")] },
        { tile: qsTr("Polarity"), name: qsTr("Polarity pulse"), blurb: qsTr("Positive half-sine lobe per period"), labels: [qsTr("Pulse width"), qsTr("Period")] },
        { tile: qsTr("Burst"), name: qsTr("Tone burst"), blurb: qsTr("Sine bursts with raised-cosine edges"), labels: [qsTr("Frequency"), qsTr("On cycles"), qsTr("Off cycles"), qsTr("Edge cycles")] },
        { tile: qsTr("2-Tone"), name: qsTr("Tone pair"), blurb: qsTr("IMD test pair (SMPTE / CCIF)"), labels: [qsTr("Tone 1"), qsTr("Tone 2"), qsTr("Ratio A1/A2")] },
        { tile: qsTr("Multi"), name: qsTr("Multitone"), blurb: qsTr("Log-spaced tones, Schroeder phases"), labels: [qsTr("Tones"), qsTr("Low"), qsTr("High")] },
        { tile: qsTr("ISP"), name: qsTr("ISP test"), blurb: qsTr("Inter-sample-peak over patterns"), labels: [qsTr("Pattern")] },
        { tile: qsTr("Chan ID"), name: qsTr("Channel ID"), blurb: qsTr("Counted pentatonic blips per channel"), labels: [qsTr("Blip length")] }
    ]
    function typeName(t) { return (types[t] || types[0]).name }

    readonly property var stateNames: [qsTr("Idle"), qsTr("Fading in"), qsTr("Running"), qsTr("Gap"), qsTr("Fading out")]
    readonly property var stateColors: isMacOS ? [MacColors.secondaryLabel, MacColors.green, MacColors.green, MacColors.yellow, MacColors.orange] : [Qt.rgba(1, 1, 1, 0.4), "#32d74b", "#32d74b", "#ffd60a", "#ff9f0a"]

    readonly property int outputCount: siggen.outputChannels > 0 ? siggen.outputChannels : bridge.numOutputChannels
    function outputName(o) { var n = bridge.channelLabel(o + 2); return n !== "" ? n : qsTr("Out %1").arg(o + 1) }
    function popcount(m) { var n = 0; while (m) { n += m & 1; m >>= 1 } return n }

    readonly property string blocker: !bridge.connected ? qsTr("No device connected")
        : !siggen.supported ? qsTr("Firmware has no signal generator")
        : d.channelMask === 0 ? qsTr("Select at least one output")
        : timing === 1 && d.durationMs === 0 ? qsTr("Sweep length must be greater than 0") : ""

    function fmtTime(ms) {
        var s = ms / 1000
        if (s < 60) return qsTr("%1 s").arg(s.toFixed(1))
        var m = Math.floor(s / 60)
        var r = s - m * 60
        return m + ":" + (r < 10 ? "0" : "") + r.toFixed(1)
    }

    function startOrStop() {
        if (siggen.running) { siggen.stop(); return }
        if (blocker !== "") return
        if (!siggen.start()) rejectedDialog.open()
    }
    Shortcut { sequence: "Space"; enabled: !win.textFocused && win.active; onActivated: win.startOrStop() }

    component SectionLabel: Text {
        font.pixelSize: 11
        font.weight: Font.Bold
        font.letterSpacing: 0.4
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
    }
    component Caption: Text {
        width: parent ? parent.width : 0
        wrapMode: Text.WordWrap
        font.pixelSize: 11
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
    }
    component LinkText: Text {
        signal clicked()
        font.pixelSize: 11
        color: isMacOS ? MacColors.accent : linkMouse.containsMouse ? "white" : "#3a96ff"
        ClickArea { id: linkMouse; anchors.fill: parent; anchors.margins: -3; hoverEnabled: true; cursorShape: Qt.PointingHandCursor
                    Accessible.role: Accessible.Link; Accessible.name: parent.text; onActivated: parent.clicked() }
    }
    // Label left, value field right
    // (plain properties: aliases in inline components misbehave in Qt 5.15)
    component FieldRow: Item {
        id: fr
        property string label: ""
        property real value: 0
        property string suffix: ""
        property int decimals: 1
        property real minValue: 0
        property real maxValue: 1e9
        property real wheelStep: 1
        property string caption: ""
        signal edited(real v)
        width: parent ? parent.width : 0
        height: caption !== "" ? 44 : 28
        Text { y: (26 - height) / 2; text: parent.label; font.pixelSize: 12; font.weight: Font.Medium; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9) }
        Text {
            visible: parent.caption !== ""
            y: 26
            text: parent.caption
            font.pixelSize: 11
            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
        }
        ValueField {
            anchors.right: parent.right
            y: 0
            height: 26
            fieldWidth: 66
            accessibleName: fr.label
            value: fr.value
            suffix: fr.suffix
            decimals: fr.decimals
            minValue: fr.minValue
            maxValue: fr.maxValue
            wheelStep: fr.wheelStep
            onValueEdited: fr.edited(newValue)
        }
    }
    // Title and caption left, switch right
    component OptionRow: Item {
        id: opt
        property string title: ""
        property string caption: ""
        property bool checked: false
        signal toggled(bool on)
        width: parent ? parent.width : 0
        height: Math.max(30, optText.height + 6)
        opacity: isMacOS ? 1 : enabled ? 1 : 0.45
        Column {
            id: optText
            anchors.left: parent.left
            anchors.right: optSwitch.left
            anchors.rightMargin: 10
            spacing: 2
            Text { text: opt.title; font.pixelSize: 12; font.weight: Font.Medium; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9) }
            Text { width: parent.width; text: opt.caption; wrapMode: Text.WordWrap; font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5) }
        }
        ToggleSwitch {
            id: optSwitch
            anchors.right: parent.right
            y: 0
            Accessible.name: opt.title
            checked: opt.checked
            // Shows the draft again after the click (a refused change, such
            // as a cancelled RAW confirmation, flips back)
            onToggled: { opt.toggled(checked); checked = Qt.binding(function () { return opt.checked }) }
        }
    }

    // ── Header ──
    ToolHeader {
        id: header
        width: parent.width
        icon: "signal"
        title: qsTr("Signal Generator")
        subtitle: qsTr("Onboard test and measurement signals")
        showSwitch: false
        accessories: [
            Rectangle {
                id: pill
                readonly property color tint: win.stateColors[siggen.state] || win.stateColors[0]
                width: pillRow.width + 20
                height: 24
                radius: 12
                color: isMacOS ? MacColors.opacity(MacColors.controlBackground, 0.6) : Qt.rgba(1, 1, 1, 0.06)
                border.color: isMacOS ? MacColors.opacity(tint, siggen.running ? 0.5 : 0.2) : Qt.rgba(tint.r, tint.g, tint.b, siggen.running ? 0.5 : 0.2)
                Row {
                    id: pillRow
                    anchors.centerIn: parent
                    spacing: 6
                    Rectangle {
                        id: stateDot
                        width: 8; height: 8; radius: 4
                        anchors.verticalCenter: parent.verticalCenter
                        color: pill.tint
                        SequentialAnimation on scale {
                            running: siggen.running && win.visible
                            loops: Animation.Infinite
                            onRunningChanged: if (!running) stateDot.scale = 1
                            NumberAnimation { to: 0.75; duration: 700; easing.type: Easing.InOutQuad }
                            NumberAnimation { to: 1.0; duration: 700; easing.type: Easing.InOutQuad }
                        }
                    }
                    Text {
                        text: win.stateNames[siggen.state] || qsTr("Idle")
                        font.pixelSize: 11
                        font.weight: Font.DemiBold
                        color: isMacOS ? (siggen.running ? MacColors.label : MacColors.secondaryLabel) : Qt.rgba(1, 1, 1, siggen.running ? 0.9 : 0.55)
                    }
                }
            }
        ]
    }
    Rectangle { id: headerRule; anchors.top: header.bottom; width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.08) }

    // ── Firmware without a generator ──
    Column {
        visible: bridge.connected && !siggen.supported
        anchors.centerIn: body
        spacing: 8
        width: 360
        Icon { anchors.horizontalCenter: parent.horizontalCenter; name: "signal"; size: 32; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.35) }
        Text { anchors.horizontalCenter: parent.horizontalCenter; text: qsTr("Signal generator not available"); font.pixelSize: 13; font.weight: Font.DemiBold; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.85) }
        Text {
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.WordWrap
            text: qsTr("The connected firmware does not include the onboard test signal generator. Update the DSPi firmware to use this tool.")
            font.pixelSize: 11
            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
        }
    }

    // ── Body ──
    Item {
        id: body
        anchors.top: headerRule.bottom
        anchors.bottom: transportRule.top
        width: parent.width
        visible: !bridge.connected || siggen.supported
        enabled: bridge.connected && siggen.supported

        // Left: signal, outputs, level (scrolls in a short window)
        Flickable {
            id: leftFlick
            width: leftColumn.width + 32
            height: parent.height
            contentHeight: leftColumn.height + 28
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar {}

        Column {
            id: leftColumn
            x: 16
            y: 14
            width: 360
            spacing: 16

            Column {
                width: parent.width
                spacing: 8
                SectionLabel { text: qsTr("SIGNAL") }
                Grid {
                    columns: 5
                    spacing: 6
                    Repeater {
                        model: win.types
                        Rectangle {
                            readonly property bool selected: win.d.type === index
                            width: (leftColumn.width - 24) / 5
                            height: 42
                            radius: 7
                            color: isMacOS ? (selected ? MacColors.opacity(MacColors.accent, 0.16) : MacColors.opacity(MacColors.controlBackground, 0.6)) : selected ? Qt.rgba(0.04, 0.49, 1, 0.16) : tileMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.09) : Qt.rgba(1, 1, 1, 0.05)
                            border.color: isMacOS ? (selected ? MacColors.opacity(MacColors.accent, 0.7) : MacColors.opacity(MacColors.gray, 0.2)) : selected ? Qt.rgba(0.04, 0.49, 1, 0.7) : Qt.rgba(1, 1, 1, 0.08)
                            Column {
                                anchors.centerIn: parent
                                spacing: 3
                                SiggenGlyph {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    width: 46; height: 20
                                    type: index
                                    color: isMacOS ? (parent.parent.selected ? MacColors.accent : MacColors.secondaryLabel) : parent.parent.selected ? "#3a96ff" : Qt.rgba(1, 1, 1, 0.6)
                                }
                                Text {
                                    anchors.horizontalCenter: parent.horizontalCenter
                                    text: modelData.tile
                                    font.pixelSize: 10
                                    font.weight: parent.parent.selected ? Font.DemiBold : Font.Normal
                                    color: isMacOS ? (parent.parent.selected ? MacColors.accent : MacColors.secondaryLabel) : Qt.rgba(1, 1, 1, parent.parent.selected ? 0.95 : 0.7)
                                }
                            }
                            ClickArea {
                                id: tileMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                focusRadius: 9
                                Accessible.role: Accessible.RadioButton
                                Accessible.name: modelData.name
                                Accessible.description: modelData.blurb
                                Accessible.checkable: true
                                Accessible.checked: parent.selected
                                onActivated: siggen.selectType(index)
                                ToolTip.visible: containsMouse
                                ToolTip.delay: 600
                                ToolTip.text: modelData.name + ": " + modelData.blurb
                            }
                        }
                    }
                }
                Caption { text: win.types[win.d.type].blurb }
            }

            Column {
                width: parent.width
                spacing: 8
                RowLayout {
                    width: parent.width
                    SectionLabel { text: qsTr("OUTPUTS") }
                    Item { Layout.fillWidth: true }
                    LinkText { text: qsTr("All"); onClicked: siggen.selectAllOutputs() }
                    LinkText { text: qsTr("None"); onClicked: siggen.clearOutputs() }
                }
                Grid {
                    columns: 3
                    spacing: 6
                    Repeater {
                        model: win.outputCount
                        Rectangle {
                            readonly property bool on: (win.d.channelMask >> index & 1) === 1
                            readonly property bool inverted: (win.d.invertMask >> index & 1) === 1
                            readonly property bool walkActive: siggen.running && siggen.activeChannel === index
                            readonly property bool silent: (win.rev, !bridge.outputEnabled(index))
                            width: (leftColumn.width - 12) / 3
                            height: 26
                            radius: 6
                            opacity: silent ? 0.4 : 1
                            color: isMacOS ? (on ? MacColors.opacity(MacColors.accent, 0.18) : MacColors.opacity(MacColors.controlBackground, 0.6)) : on ? Qt.rgba(0.04, 0.49, 1, 0.18) : chipMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.09) : Qt.rgba(1, 1, 1, 0.05)
                            border.width: walkActive ? 1.5 : 1
                            border.color: isMacOS ? (walkActive ? MacColors.green : on ? MacColors.opacity(MacColors.accent, 0.6) : MacColors.opacity(MacColors.gray, 0.2)) : walkActive ? "#32d74b" : on ? Qt.rgba(0.04, 0.49, 1, 0.6) : Qt.rgba(1, 1, 1, 0.08)
                            Row {
                                anchors.centerIn: parent
                                spacing: 4
                                Text {
                                    text: win.outputName(index)
                                    font.pixelSize: 11
                                    font.weight: parent.parent.on ? Font.DemiBold : Font.Normal
                                    color: isMacOS ? (parent.parent.on ? MacColors.label : MacColors.secondaryLabel) : Qt.rgba(1, 1, 1, parent.parent.on ? 0.95 : 0.65)
                                    elide: Text.ElideRight
                                    width: Math.min(implicitWidth, (leftColumn.width - 12) / 3 - 26)
                                }
                                Text {
                                    visible: parent.parent.inverted
                                    text: "ø"
                                    font.pixelSize: 12
                                    font.bold: true
                                    color: isMacOS ? MacColors.orange : "#ff9f0a"
                                }
                            }
                            ClickArea {
                                id: chipMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                focusRadius: 8
                                Accessible.name: win.outputName(index)
                                Accessible.description: parent.silent
                                    ? (parent.on ? (parent.inverted ? qsTr("Inverted, disabled in the matrix mixer. Press: on, inverted, off")
                                                                    : qsTr("On, disabled in the matrix mixer. Press: on, inverted, off"))
                                                 : qsTr("Off, disabled in the matrix mixer. Press: on, inverted, off"))
                                    : (parent.on ? (parent.inverted ? qsTr("Inverted. Press: on, inverted, off") : qsTr("On. Press: on, inverted, off"))
                                                 : qsTr("Off. Press: on, inverted, off"))
                                onActivated: siggen.cycleOutput(index)
                                ToolTip.visible: containsMouse
                                ToolTip.delay: 600
                                ToolTip.text: parent.silent ? qsTr("Output disabled in the matrix mixer: selected but silent until enabled")
                                                            : qsTr("Click: on → inverted → off")
                            }
                        }
                    }
                }
                Caption { text: qsTr("Click to select, click again to invert polarity (ø). Dimmed outputs are disabled in the matrix mixer and stay silent.") }
            }

            Column {
                width: parent.width
                spacing: 6
                RowLayout {
                    width: parent.width
                    SectionLabel { text: qsTr("LEVEL") }
                    Item { Layout.fillWidth: true }
                    ValueField {
                        fieldWidth: 60; height: 24; suffix: qsTr("dB"); decimals: 1; wheelStep: 1
                        minValue: -120; maxValue: 0
                        accessibleName: qsTr("Level")
                        value: win.d.levelDb
                        onValueEdited: siggen.setDraftValue("levelDb", newValue)
                    }
                }
                StyledSlider {
                    width: parent.width
                    from: -80; to: 0; stepSize: 0.5
                    Accessible.name: qsTr("Level")
                    value: win.d.levelDb
                    onMoved: siggen.setDraftValue("levelDb", value)
                }
                Caption { text: qsTr("Peak level in dBFS. Output trim, master volume and mute still apply downstream.") }
            }
        }
        }

        Rectangle { id: colRule; x: leftFlick.width; width: 1; height: parent.height; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.08) }

        // Right: parameters, timing, options
        Flickable {
            anchors.left: colRule.right
            anchors.right: parent.right
            anchors.top: parent.top
            anchors.bottom: parent.bottom
            contentHeight: rightColumn.height + 28
            clip: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: ScrollBar {}

            Column {
                id: rightColumn
                x: 16
                y: 14
                width: parent.width - 32
                spacing: 16

                // PARAMETERS
                Column {
                    readonly property var params: win.info.params || []
                    readonly property bool any: params.some(function (p) { return p.semantic !== 0 })
                    visible: any
                    width: parent.width
                    spacing: 4
                    SectionLabel { text: qsTr("PARAMETERS") }
                    Repeater {
                        model: win.d.type === 13 ? [] : parent.params
                        FieldRow {
                            visible: modelData.semantic !== 0
                            height: visible ? 28 : 0
                            readonly property int sem: modelData.semantic
                            label: (win.types[win.d.type].labels[index]) || qsTr("Value")
                            suffix: sem === 1 ? qsTr("Hz") : sem === 2 ? qsTr("ms") : sem === 3 ? qsTr("cyc") : sem === 5 ? "×" : ""
                            decimals: sem === 3 || sem === 4 ? 0 : sem === 5 ? 2 : 1
                            wheelStep: sem === 5 ? 0.1 : 1
                            minValue: modelData.min
                            maxValue: modelData.max > 0 ? modelData.max : 1e9
                            value: win.d.p[index]
                            onEdited: siggen.setParam(index, v)
                        }
                    }
                    // ISP: which over pattern
                    SegmentedControl {
                        visible: win.d.type === 13
                        width: parent.width
                        model: [qsTr("fs/4 · +3.01 dBTP"), qsTr("fs/6 · +1.25 dBTP")]
                        currentIndex: win.d.p[0] >= 0.5 ? 1 : 0
                        onActivated: siggen.setParam(0, index)
                        tips: [qsTr("Alternating +1 +1 -1 -1 samples: the reconstructed peak is 3.01 dB over the samples"),
                               qsTr("A sample pattern at fs/6: the reconstructed peak is 1.25 dB over the samples")]
                    }
                    Row {
                        visible: win.d.type === 11
                        spacing: 10
                        Text { text: qsTr("Presets:"); font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5) }
                        LinkText { text: qsTr("SMPTE 60/7k"); onClicked: { siggen.setParam(0, 60); siggen.setParam(1, 7000); siggen.setParam(2, 4) } }
                        LinkText { text: qsTr("CCIF 19k/20k"); onClicked: { siggen.setParam(0, 19000); siggen.setParam(1, 20000); siggen.setParam(2, 1) } }
                    }
                    Caption {
                        visible: win.d.type === 12 && siggen.multitoneMax > 0
                        text: qsTr("Up to %1 tones on this device.").arg(siggen.multitoneMax)
                    }
                }

                // TIMING
                Column {
                    width: parent.width
                    spacing: 4
                    SectionLabel { text: qsTr("TIMING") }
                    // Sweep
                    FieldRow {
                        visible: win.timing === 1
                        label: qsTr("Sweep length")
                        suffix: qsTr("s"); decimals: 2; wheelStep: 0.5; minValue: 0.01; maxValue: 3600
                        value: win.d.durationMs / 1000
                        onEdited: siggen.setDraftValue("durationMs", Math.round(v * 1000))
                    }
                    FieldRow {
                        visible: win.timing !== 0 || win.walking
                        label: win.timing === 0 ? qsTr("Passes") : qsTr("Repeat")
                        caption: win.timing === 1 ? qsTr("0 = repeat forever")
                               : win.d.type === 14 ? qsTr("Passes over the selected outputs. 0 = forever")
                               : win.timing === 2 ? qsTr("Pattern periods. 0 = repeat forever")
                               : qsTr("Full passes over the outputs. 0 = forever")
                        decimals: 0; wheelStep: 1; minValue: 0; maxValue: 65535
                        value: win.d.repeat
                        onEdited: siggen.setDraftValue("repeat", Math.round(v))
                    }
                    FieldRow {
                        visible: win.timing !== 0
                        label: win.timing === 1 ? qsTr("Gap between sweeps") : qsTr("Extra gap per period")
                        suffix: qsTr("ms"); decimals: 0; wheelStep: 50; minValue: 0; maxValue: 65535
                        value: win.d.gapMs
                        onEdited: siggen.setDraftValue("gapMs", Math.round(v))
                    }
                    // Continuous
                    FieldRow {
                        visible: win.timing === 0
                        label: win.walking ? qsTr("Dwell per channel") : qsTr("Duration")
                        caption: win.walking ? qsTr("0 = 2 s default") : qsTr("0 = play until stopped")
                        suffix: qsTr("s"); decimals: 2; wheelStep: 0.5; minValue: 0; maxValue: 86400
                        value: win.d.durationMs / 1000
                        onEdited: siggen.setDraftValue("durationMs", Math.round(v * 1000))
                    }
                }

                // OPTIONS
                Column {
                    width: parent.width
                    spacing: 8
                    SectionLabel { text: qsTr("OPTIONS") }
                    OptionRow {
                        title: qsTr("Bypass output EQ (RAW)")
                        caption: qsTr("Skips crossover and PEQ on the selected outputs. Trim, master volume, mute and delay still apply.")
                        checked: (win.d.flags & 1) !== 0
                        onToggled: on ? rawDialog.open() : siggen.setFlag(1, false)
                    }
                    OptionRow {
                        visible: win.d.type === 2 || win.d.type === 3
                        title: qsTr("Decorrelate channels")
                        caption: qsTr("Independent noise per output instead of one copied signal.")
                        checked: (win.d.flags & 2) !== 0
                        onToggled: siggen.setFlag(2, on)
                    }
                    OptionRow {
                        title: qsTr("Walk outputs one at a time")
                        caption: win.d.type === 14 ? qsTr("Channel ID always walks: each output plays its channel number as counted blips.")
                                                   : qsTr("Plays the selected outputs sequentially instead of together.")
                        enabled: win.d.type !== 14
                        checked: win.walking
                        onToggled: siggen.setFlag(4, on)
                    }
                }
            }
        }
    }

    // ── Transport ──
    Rectangle { id: transportRule; anchors.bottom: transport.top; width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.08) }
    Item {
        id: transport
        anchors.bottom: parent.bottom
        width: parent.width
        height: 54
        Column {
            anchors.left: parent.left
            anchors.leftMargin: 16
            anchors.right: buttons.left
            anchors.rightMargin: 12
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2
            Text {
                width: parent.width
                elide: Text.ElideRight
                font.pixelSize: 12
                font.weight: Font.DemiBold
                color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9)
                text: siggen.running ? win.stateNames[siggen.state] + " · " + win.typeName(siggen.signalType)
                    : win.blocker !== "" && bridge.connected && siggen.supported ? win.blocker
                    : qsTr("Ready · %1").arg(win.typeName(win.d.type))
            }
            Text {
                width: parent.width
                elide: Text.ElideRight
                font.pixelSize: 11
                color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
                text: {
                    if (siggen.running) {
                        var parts = [win.fmtTime(siggen.elapsedMs)]
                        if (siggen.currentFreq > 0)
                            parts.push(siggen.currentFreq >= 1000 ? qsTr("%1 kHz").arg((siggen.currentFreq / 1000).toFixed(2)) : qsTr("%1 Hz").arg(siggen.currentFreq.toFixed(0)))
                        if (siggen.cyclesDone > 0) parts.push(qsTr("cycle %1").arg(siggen.cyclesDone))
                        if (siggen.activeChannel >= 0) parts.push(win.outputName(siggen.activeChannel))
                        parts.push(qsTr("edits apply live"))
                        return parts.join(" · ")
                    }
                    if (siggen.stopReason === 2) return qsTr("Finished after %1").arg(win.fmtTime(siggen.elapsedMs))
                    if (siggen.stopReason === 1) return qsTr("Stopped")
                    if (siggen.stopReason === 3) return qsTr("Stopped by preset load")
                    var n = win.popcount(win.d.channelMask)
                    return n === 1 ? qsTr("%1 output · peak %2 dBFS").arg(n).arg(win.d.levelDb.toFixed(1))
                                  : qsTr("%1 outputs · peak %2 dBFS").arg(n).arg(win.d.levelDb.toFixed(1))
                }
            }
        }
        Row {
            id: buttons
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            spacing: 8
            // Stop now (no fade)
            Rectangle {
                visible: siggen.running
                width: 30; height: 28; radius: 8
                color: isMacOS ? (nowMouse.pressed ? MacColors.opacity(MacColors.control, 1.6) : MacColors.control) : nowMouse.pressed ? Qt.rgba(1, 1, 1, 0.14) : nowMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.08) : "transparent"
                border.color: isMacOS ? "transparent" : Qt.rgba(1, 1, 1, 0.18)
                Icon { anchors.centerIn: parent; name: "stop"; size: 13; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.8) }
                ClickArea {
                    id: nowMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    cursorShape: Qt.PointingHandCursor
                    focusRadius: 10
                    Accessible.name: qsTr("Stop immediately, no fade")
                    onActivated: siggen.stopNow()
                    ToolTip.visible: containsMouse
                    ToolTip.delay: 600
                    ToolTip.text: qsTr("Stop immediately, no fade")
                }
            }
            // Start / Stop
            Rectangle {
                readonly property bool canStart: win.blocker === ""
                width: mainRow.width + 28
                height: 28
                radius: 8
                opacity: isMacOS ? 1 : siggen.running || canStart ? 1 : 0.4
                color: isMacOS ? (siggen.running ? (mainMouse.pressed ? Qt.darker(MacColors.red, 1.2) : MacColors.red) : !canStart ? MacColors.control : mainMouse.pressed ? MacColors.defaultButtonPressed : MacColors.prominentButton) : siggen.running ? (mainMouse.pressed ? "#b52e35" : "#d9363e")
                                      : (mainMouse.pressed ? "#0868d6" : "#0a7cff")
                Row {
                    id: mainRow
                    anchors.centerIn: parent
                    spacing: 6
                    Icon { name: siggen.running ? "stop" : "play"; size: 13; color: isMacOS ? (siggen.running || parent.parent.canStart ? "white" : MacColors.tertiaryLabel) : "white"; anchors.verticalCenter: parent.verticalCenter }
                    Text { text: siggen.running ? qsTr("Stop") : qsTr("Start"); font.pixelSize: 13; font.weight: Font.DemiBold; color: isMacOS ? (siggen.running || parent.parent.canStart ? "white" : MacColors.tertiaryLabel) : "white"; anchors.verticalCenter: parent.verticalCenter }
                }
                ClickArea {
                    id: mainMouse
                    anchors.fill: parent
                    hoverEnabled: true
                    enabled: siggen.running || parent.canStart
                    cursorShape: Qt.PointingHandCursor
                    focusRadius: 10
                    Accessible.name: siggen.running ? qsTr("Stop") : qsTr("Start")
                    Accessible.description: parent.canStart || siggen.running ? "" : win.blocker
                    onActivated: win.startOrStop()
                }
                ToolTip.visible: mainHover.containsMouse && !parent.canStart && !siggen.running
                ToolTip.delay: 400
                ToolTip.text: win.blocker
                MouseArea { id: mainHover; anchors.fill: parent; hoverEnabled: true; acceptedButtons: Qt.NoButton }
            }
        }
    }

    AppDialog {
        id: rawDialog
        icon: "warning"
        iconTint: "#ff9f0a"
        title: qsTr("Bypass the Output EQ?")
        message: qsTr("The signal skips each selected output's crossover and PEQ. A full-range signal can damage a tweeter or other driver that relies on its crossover for protection.")
        buttons: [
            { key: "cancel", text: qsTr("Cancel") },
            { key: "bypass", text: qsTr("Bypass"), role: "destructive" }
        ]
        onChosen: if (key === "bypass") siggen.setFlag(1, true)
    }

    AppDialog {
        id: rejectedDialog
        icon: "warning"
        iconTint: "#ff9f0a"
        title: qsTr("Signal Refused")
        message: qsTr("The device declined the signal configuration.")
        buttons: [{ key: "ok", text: qsTr("OK"), role: "primary" }]
    }
}
