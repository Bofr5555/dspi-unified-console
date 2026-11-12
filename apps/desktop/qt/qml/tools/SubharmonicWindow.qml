import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- The Subharmonic Synthesizer (the Mac's SubharmonicSynthView,
// 780×620, at least 740×612): the header with Solo and the switch; on the left
// the band graph with the starting points and the headroom it costs, over the
// band levels; on the right selectivity and the sub ceiling (wire V30), the LF
// boost and the outputs with their sub meters and the pair link. Firmware
// before wire V29 gets a note instead. Closing the window switches Solo off.
ToolWindow {
    id: window
    objectName: "SubharmonicWindow"
    windowId: "subharmonic"
    defaultWidth: 780
    defaultHeight: 620
    minimumWidth: 740
    minimumHeight: 612
    title: qsTr("Subharmonic Synthesizer")

    readonly property bool writable: ToolsController.writable && ToolsController.subharmSupported
    readonly property bool unsupported: ToolsController.connected && !ToolsController.subharmSupported
    readonly property bool extended: ToolsController.subharmExtended
    readonly property bool on: value("enabled") !== 0
    readonly property int mask: value("mask")
    readonly property int selectMode: value("select")

    function value(key) { return ToolsController.revision < 0 ? 0 : ToolsController.subharm(key) }
    function liveOr(row, key) { return isNaN(row.liveValue) ? value(key) : row.liveValue }

    onVisibleChanged: ToolsController.windowShown("subharmonic", visible)

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        ToolHeader {
            objectName: "SubharmonicWindow.header"
            Layout.fillWidth: true
            iconName: "waves-arrow-down"
            title: qsTr("Subharmonic Synthesizer")
            subtitle: qsTr("Generates a subharmonic at half the frequency of the source's bass")
            on: window.on
            switchEnabled: window.writable
            onToggled: on => ToolsController.setSubharm("enabled", on ? 1 : 0)

            // Monitoring solo: it latches, and closing the window clears it
            AppButton {
                id: solo
                objectName: "SubharmonicWindow.solo"
                visible: window.extended
                readonly property bool soloed: ToolsController.subharmSolo
                text: qsTr("SOLO")
                primary: soloed
                enabled: window.writable && window.on
                opacity: window.on ? 1 : 0.4
                toolTip: soloed ? qsTr("The selected outputs are carrying the synthesized sub only - the program signal is muted on them. Closing this window switches it off.")
                                : qsTr("Mute the program signal on the selected outputs so the synthesized sub can be heard or measured on its own. Never saved to a preset.")
                onClicked: ToolsController.setSubharmSolo(!soloed)
                Accessible.role: Accessible.CheckBox
                Accessible.checkable: true
                Accessible.checked: soloed
            }
        }
        EffectRule {}

        ToolUnsupported {
            objectName: "SubharmonicWindow.unsupported"
            visible: window.unsupported
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: 24
            text: qsTr("Requires firmware with wire format V29 or newer.")
            detail: qsTr("Update the DSPi firmware to use the Subharmonic Synthesizer.")
        }

        EffectColumns {
            objectName: "SubharmonicWindow.body"
            visible: !window.unsupported
            Layout.fillWidth: true
            Layout.fillHeight: true

            leftItems: [
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    EffectSection {
                        text: qsTr("Bands")
                        EffectMenuButton {
                            id: startingPoints
                            objectName: "SubharmonicWindow.startingPoints"
                            enabled: window.writable
                            text: qsTr("Apply preset")
                            Instantiator {
                                model: ToolsController.subharmStartingPoints
                                delegate: AppMenuItem {
                                    required property int index
                                    required property var modelData
                                    objectName: "SubharmonicWindow.startingPoint." + index
                                    text: qsTr("%1 - %2").arg(modelData.name).arg(modelData.detail)
                                    onTriggered: ToolsController.applySubharmStartingPoint(index)
                                }
                                onObjectAdded: (index, item) => startingPoints.menu.insertItem(index, item)
                                onObjectRemoved: (index, item) => startingPoints.menu.removeItem(item)
                            }
                        }
                    }
                    SubharmonicGraph {
                        objectName: "SubharmonicWindow.graph"
                        Layout.fillWidth: true
                        Layout.preferredHeight: 188
                        active: window.on
                        graph: ToolsController.subharmGraph(window.liveOr(lowRow, "low"), window.liveOr(highRow, "high"),
                                                            window.extended ? window.liveOr(topRow, "top") : -30,
                                                            window.liveOr(boostRow, "boost"),
                                                            window.extended ? window.liveOr(ceilingRow, "ceiling") : 0)
                    }
                    // The worst-case gain of this setting, the graph's caption
                    RowLayout {
                        objectName: "SubharmonicWindow.headroom"
                        Layout.fillWidth: true
                        Layout.topMargin: 4
                        readonly property real db: ToolsController.subharmHeadroom
                        SectionHeading {
                            Layout.fillWidth: true
                            text: qsTr("Headroom cost")
                        }
                        Text {
                            objectName: "SubharmonicWindow.headroom.value"
                            text: parent.db > 0 ? qsTr("+%1 dB").arg(parent.db.toFixed(1)) : qsTr("none")
                            font: Theme.fontSmall
                            color: parent.db > 0 ? Theme.warning : Theme.textMuted
                        }
                        HoverHandler { id: headroomHover }
                        HelpTip {
                            visible: headroomHover.hovered
                            text: parent.db > 0
                                  ? qsTr("This setting can add up to %1 dB. Lower the preamp on the inputs feeding the selected outputs by that much, or a loud passage will clip.").arg(parent.db.toFixed(1))
                                  : qsTr("This setting cannot push the signal past full scale.")
                        }
                    }
                },
                EffectRule {},
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 14
                    SectionHeading { text: qsTr("Levels") }
                    ToolParameterRow {
                        id: lowRow
                        objectName: "SubharmonicWindow.low"
                        Layout.fillWidth: true
                        enabled: window.writable
                        title: qsTr("24 - 36 Hz")
                        subtitle: qsTr("Derived from %1").arg(qsTr("48 - 72 Hz"))
                        unit: qsTr("dB")
                        value: window.value("low")
                        from: -30
                        to: 12
                        scrollStep: 0.5
                        resetValue: 0
                        offAt: -1
                        ends: [qsTr("Off"), qsTr("+12 dB")]
                        help: qsTr("Level of the sub synthesized from program content between 48 and 72 Hz. At 0 dB it comes out 1.4 dB below the bass that produced it, which is the divider's own gain.")
                        onLive: v => ToolsController.setSubharm("low", v)
                        onCommit: v => ToolsController.setSubharm("low", v)
                    }
                    EffectRule {}
                    ToolParameterRow {
                        id: highRow
                        objectName: "SubharmonicWindow.high"
                        Layout.fillWidth: true
                        enabled: window.writable
                        title: qsTr("36 - 56 Hz")
                        subtitle: qsTr("Derived from %1").arg(qsTr("72 - 112 Hz"))
                        unit: qsTr("dB")
                        value: window.value("high")
                        from: -30
                        to: 12
                        scrollStep: 0.5
                        resetValue: 0
                        offAt: -1
                        ends: [qsTr("Off"), qsTr("+12 dB")]
                        help: qsTr("Level of the sub synthesized from program content between 72 and 112 Hz. This band has its own divider, so a bass note here and a kick in the band below are tracked independently.")
                        onLive: v => ToolsController.setSubharm("high", v)
                        onCommit: v => ToolsController.setSubharm("high", v)
                    }
                    EffectRule { visible: window.extended }
                    ToolParameterRow {
                        id: topRow
                        objectName: "SubharmonicWindow.top"
                        visible: window.extended
                        Layout.fillWidth: true
                        enabled: window.writable
                        title: qsTr("56 - 80 Hz")
                        subtitle: qsTr("Derived from %1").arg(qsTr("112 - 160 Hz"))
                        unit: qsTr("dB")
                        value: window.value("top")
                        from: -30
                        to: 12
                        scrollStep: 0.5
                        resetValue: -30
                        offAt: -1
                        ends: [qsTr("Off"), qsTr("+12 dB")]
                        help: qsTr("Level of the sub synthesized from program content between 112 and 160 Hz. It ships off: this band reaches up into the range where a divided sub starts to compete with the program's own fundamentals. Turn it up for a subwoofer that cannot reach the lowest octave.")
                        onLive: v => ToolsController.setSubharm("top", v)
                        onCommit: v => ToolsController.setSubharm("top", v)
                    }
                }
            ]
            rightItems: [
                ColumnLayout {
                    objectName: "SubharmonicWindow.selectivity"
                    visible: window.extended
                    Layout.fillWidth: true
                    spacing: 14
                    SectionHeading { text: qsTr("Selectivity") }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 6
                        ToolSegments {
                            objectName: "SubharmonicWindow.mode"
                            Layout.fillWidth: true
                            enabled: window.writable
                            label: qsTr("Selectivity")
                            model: [qsTr("All material"), qsTr("Percussive"), qsTr("Sustained")]
                            current: window.selectMode
                            onChosen: index => ToolsController.setSubharm("select", index)
                        }
                        Text {
                            objectName: "SubharmonicWindow.mode.summary"
                            Layout.fillWidth: true
                            text: window.selectMode === 1 ? qsTr("A short sub burst after each attack - extends kicks, not the bass line.")
                                : window.selectMode === 2 ? qsTr("The sub opens once a band has been ringing - extends bass notes, not kicks.")
                                : qsTr("Every band signal is treated alike.")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                            wrapMode: Text.Wrap
                            HoverHandler { id: modeHover }
                            HelpTip {
                                visible: modeHover.hovered
                                text: window.selectMode === 1
                                      ? qsTr("A short sub burst after each attack, so a kick can be extended without extending the bass line under it. The most robust of the three: a held note gets nothing between kicks.")
                                      : window.selectMode === 2
                                        ? qsTr("The sub opens only after a band has been ringing, so bass notes are extended and kicks are not. A kick landing in the same band ducks the held note's sub, which pumps on a four-on-the-floor line - shorten the hold or lower the depth to soften it.")
                                        : qsTr("Every band signal is treated alike. Choose percussive or sustained to weight the synthesized sub toward one kind of bass material; the gate decides per band from time behaviour, so it cannot separate two sources sounding at once in the same band.")
                            }
                        }
                    }
                    // Depth and hold do nothing while every band signal is treated alike
                    ToolParameterRow {
                        objectName: "SubharmonicWindow.depth"
                        visible: window.selectMode !== 0
                        Layout.fillWidth: true
                        enabled: window.writable
                        title: qsTr("Depth")
                        unit: qsTr("%")
                        value: window.value("depth")
                        from: 0
                        to: 100
                        decimals: 0
                        scrollStep: 5
                        resetValue: 100
                        help: qsTr("How far the material this mode does not favour is gated down. At 0% the selectivity is inaudible whatever the mode is set to; 100% is full gating.")
                        onLive: v => ToolsController.setSubharm("depth", v)
                        onCommit: v => ToolsController.setSubharm("depth", v)
                    }
                    ToolParameterRow {
                        objectName: "SubharmonicWindow.hold"
                        visible: window.selectMode !== 0
                        Layout.fillWidth: true
                        enabled: window.writable
                        title: qsTr("Hold")
                        unit: qsTr("ms")
                        value: window.value("hold")
                        from: 50
                        to: 400
                        decimals: 0
                        scrollStep: 10
                        resetValue: 150
                        help: window.selectMode === 1 ? qsTr("The length of the sub burst after each attack.")
                                                      : qsTr("How long a band must ring before its sub opens. Every note's first hold period has no sub, so staccato bass lines get little.")
                        onLive: v => ToolsController.setSubharm("hold", v)
                        onCommit: v => ToolsController.setSubharm("hold", v)
                    }
                },
                EffectRule { visible: window.extended },
                ColumnLayout {
                    visible: window.extended
                    Layout.fillWidth: true
                    spacing: 14
                    SectionHeading { text: qsTr("Sub ceiling") }
                    ToolParameterRow {
                        id: ceilingRow
                        objectName: "SubharmonicWindow.ceiling"
                        Layout.fillWidth: true
                        enabled: window.writable
                        title: qsTr("Threshold")
                        unit: qsTr("dB")
                        value: window.value("ceiling")
                        from: -40
                        to: 0
                        decimals: 0
                        scrollStep: 1
                        resetValue: 0
                        offAt: 1
                        ends: [qsTr("-40 dBFS"), qsTr("Off")]
                        help: qsTr("A soft limit on the synthesized sub just before it is mixed back in, capping how far it can push a driver without touching the program signal. It is an absolute level, so a ceiling at full scale limits nothing and means the stage is off. With it on, the headroom cost is only the ceiling's worth. A loud onset overshoots it by a few dB for the first few milliseconds while the limiter's 3 ms attack catches up.")
                        onLive: v => ToolsController.setSubharm("ceiling", v)
                        onCommit: v => ToolsController.setSubharm("ceiling", v)
                    }
                },
                EffectRule { visible: window.extended },
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 14
                    SectionHeading { text: qsTr("LF boost") }
                    ToolParameterRow {
                        id: boostRow
                        objectName: "SubharmonicWindow.boost"
                        Layout.fillWidth: true
                        enabled: window.writable
                        title: qsTr("70 Hz bell")
                        unit: qsTr("dB")
                        value: window.value("boost")
                        from: 0
                        to: 6
                        scrollStep: 0.5
                        resetValue: 0
                        ends: [qsTr("Off"), qsTr("+6 dB")]
                        help: qsTr("A gentle bell at 70 Hz, Q 0.9, applied to the whole output after the subs are summed. It fills the gap between the synthesized sub and the program's own mid-bass. Meant to stay gentle, as on the dbx.")
                        onLive: v => ToolsController.setSubharm("boost", v)
                        onCommit: v => ToolsController.setSubharm("boost", v)
                    }
                },
                EffectRule {},
                ColumnLayout {
                    objectName: "SubharmonicWindow.outputs"
                    Layout.fillWidth: true
                    spacing: 10
                    EffectSection {
                        text: qsTr("Outputs")
                        EffectMenuButton {
                            objectName: "SubharmonicWindow.outputs.presets"
                            enabled: window.writable
                            text: qsTr("Presets")
                            AppMenuItem {
                                objectName: "SubharmonicWindow.outputs.sub"
                                text: qsTr("Sub only (recommended)")
                                onTriggered: ToolsController.setSubharm("mask", ToolsController.subOnlyMask())
                            }
                            AppMenuItem {
                                objectName: "SubharmonicWindow.outputs.all"
                                text: qsTr("All outputs")
                                onTriggered: ToolsController.setSubharm("mask", ToolsController.allOutputsMask())
                            }
                            AppMenuItem {
                                objectName: "SubharmonicWindow.outputs.none"
                                text: qsTr("None")
                                onTriggered: ToolsController.setSubharm("mask", 0)
                            }
                        }
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 6
                        HoverHandler { id: chipsHover }
                        HelpTip {
                            visible: chipsHover.hovered
                            text: qsTr("Select the outputs that can actually play 24 to 80 Hz. Subharm runs before the crossover, so a satellite with a highpass loses the sub again - mask it off and save the CPU instead.")
                        }
                        Repeater {
                            model: ToolsController.numOutputs
                            ColumnLayout {
                                id: chipColumn
                                required property int index
                                readonly property bool on: (window.mask & (1 << index)) !== 0
                                Layout.fillWidth: true
                                spacing: 3
                                EffectChip {
                                    objectName: "SubharmonicWindow.output." + chipColumn.index
                                    enabled: window.writable
                                    on: chipColumn.on
                                    text: String(chipColumn.index + 1)
                                    toolTip: { ToolsController.revision; return ToolsController.outputName(chipColumn.index) }
                                    onClicked: ToolsController.toggleSubharmOutput(chipColumn.index)
                                }
                                // The synthesized sub alone, not the output
                                Meter {
                                    objectName: "SubharmonicWindow.meter." + chipColumn.index
                                    visible: window.extended
                                    Layout.fillWidth: true
                                    Layout.preferredHeight: 3
                                    opacity: chipColumn.on ? 1 : 0.25
                                    color: Theme.accent
                                    trackColor: Theme.control
                                    level: chipColumn.on && chipColumn.index < ToolsController.subharmMeter.length
                                           ? ToolsController.subharmMeter[chipColumn.index] : 0
                                }
                            }
                        }
                    }
                    RowLayout {
                        visible: window.extended
                        Layout.fillWidth: true
                        Layout.topMargin: 4
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2
                            Text {
                                text: qsTr("Link output pairs")
                                font: Theme.fontBody
                                color: window.writable ? Theme.text : Theme.textDisabled
                            }
                            Text {
                                Layout.fillWidth: true
                                text: qsTr("One sub per pair, from its mono sum.")
                                font: Theme.fontCaption
                                color: Theme.textMuted
                                wrapMode: Text.Wrap
                            }
                            HoverHandler { id: linkHover }
                            HelpTip {
                                visible: linkHover.hovered
                                text: qsTr("Synthesize one sub per output pair from its mono sum and feed it to both channels, as the dbx does. Bass is near-mono in most material, and two independent dividers can land on opposite polarities, which cancels a centred note's sub between the speakers.")
                            }
                        }
                        AppSwitch {
                            objectName: "SubharmonicWindow.linkPairs"
                            text: ""
                            enabled: window.writable
                            checked: window.value("link") !== 0
                            onToggled: ToolsController.setSubharm("link", checked ? 1 : 0)
                            Accessible.name: qsTr("Link output pairs")
                        }
                    }
                }
            ]
        }
    }
}
