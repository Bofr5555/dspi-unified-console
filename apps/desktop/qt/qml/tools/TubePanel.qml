import QtCore
import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- The Tube Modeller (the Mac's TubeModellerView, 780×664, at
// least 740 wide). Basic shows the tube on the left, and on the right the
// shelf of tubes, drive and mix, and the outputs. Advanced shows every
// parameter: the transfer curve with the harmonics at full scale, the stage
// and the outputs on the left; the character and the output stage on the
// right. The choice is remembered. Firmware before wire V31 gets a note.
// -- WP LAY1a -- A panel (Panel.qml): PanelTable's row for
// tools.tube gives its window's title and sizes.
Panel {
    id: panel
    objectName: "TubePanel"
    // -- WP QTPOP --
    fitBodies: [basic, advanced]

    property bool advanced: false
    readonly property bool writable: ToolsController.writable && ToolsController.tubeSupported
    readonly property bool unsupported: ToolsController.connected && !ToolsController.tubeSupported
    readonly property bool on: value("enabled") !== 0
    readonly property int type: value("type")
    readonly property int mask: value("mask")
    readonly property int rectifier: value("rectifier")
    readonly property bool xfmr: value("xfmr") !== 0
    readonly property var row: type > 0 && type < ToolsController.tubeTypes.length ? ToolsController.tubeTypes[type] : null

    function value(key) { return ToolsController.revision < 0 ? 0 : ToolsController.tube(key) }

    onShownChanged: ToolsController.windowShown("tube", shown)
    Component.onDestruction: ToolsController.windowShown("tube", false)   // -- WP LAY1a -- a page host unloaded

    Settings {
        category: "tools/tube"
        property alias advanced: panel.advanced
    }

    // One parameter row of the panel: its key, and the graph follows its drag
    component TubeRow: ToolParameterRow {
        property string key
        Layout.fillWidth: true
        enabled: panel.writable
        value: panel.value(key)
        onLive: v => ToolsController.setTube(key, v)
        onCommit: v => ToolsController.setTube(key, v)
    }

    // A harmonic's level against the fundamental, "none" when absent
    component Harmonic: RowLayout {
        id: harmonic
        property string label
        property real db: -120
        spacing: 4
        Text {
            text: harmonic.label
            font: Theme.fontCaption
            color: Theme.textMuted
        }
        Text {
            objectName: harmonic.objectName.length ? harmonic.objectName + ".value" : ""
            Layout.preferredWidth: 42
            text: harmonic.db <= -100 ? qsTr("none") : qsTr("%1 dB").arg(Math.round(harmonic.db))
            font: Theme.fontMono
            color: harmonic.db > -100 ? Theme.text : Theme.textMuted
        }
    }

    // The outputs the stage runs on, with their presets
    component Outputs: ColumnLayout {
        id: outputs
        property string prefix
        Layout.fillWidth: true
        spacing: 10
        EffectSection {
            text: qsTr("Outputs")
            EffectMenuButton {
                objectName: outputs.prefix + ".presets"
                enabled: panel.writable
                text: qsTr("Presets")
                AppMenuItem {
                    objectName: outputs.prefix + ".all"
                    text: qsTr("All outputs")
                    onTriggered: ToolsController.setTube("mask", ToolsController.allOutputsMask())
                }
                AppMenuItem {
                    objectName: outputs.prefix + ".noSub"
                    text: qsTr("Exclude sub")
                    onTriggered: ToolsController.setTube("mask", ToolsController.withoutSubMask())
                }
                AppMenuItem {
                    objectName: outputs.prefix + ".none"
                    text: qsTr("None")
                    onTriggered: ToolsController.setTube("mask", 0)
                }
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: 6
            HoverHandler { id: outputsHover }
            HelpTip {
                visible: outputsHover.hovered
                text: qsTr("Tube runs before the crossover and the per-output EQ, where a real preamp sits: a sub output saturates the full-band program and then low-passes the result.")
            }
            Repeater {
                model: ToolsController.numOutputs
                EffectChip {
                    required property int index
                    objectName: outputs.prefix + "." + index
                    enabled: panel.writable
                    on: (panel.mask & (1 << index)) !== 0
                    text: String(index + 1)
                    toolTip: { ToolsController.revision; return ToolsController.outputName(index) }
                    onClicked: ToolsController.toggleTubeOutput(index)
                }
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        ToolHeader {
            objectName: "TubeWindow.header"
            Layout.fillWidth: true
            iconName: "tube"
            title: qsTr("Tube Modeller")
            subtitle: qsTr("Valve-style harmonic colour, supply sag and a tube amplifier's output stage")
            on: panel.on
            switchEnabled: panel.writable
            onToggled: on => ToolsController.setTube("enabled", on ? 1 : 0)

            ToolSegments {
                objectName: "TubeWindow.mode"
                visible: !panel.unsupported
                label: qsTr("Basic or Advanced")
                model: [qsTr("Basic"), qsTr("Advanced")]
                current: panel.advanced ? 1 : 0
                onChosen: index => panel.advanced = index === 1
            }
        }
        EffectRule {}

        ToolUnsupported {
            objectName: "TubeWindow.unsupported"
            visible: panel.unsupported
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: 24
            text: qsTr("Requires firmware with wire format V31 or newer.")
            detail: qsTr("Update the DSPi firmware to use the Tube Modeller.")
        }

        // ---- Basic: the tube on show, the shelf, drive and mix ----
        EffectColumns {
            id: basic
            shown: panel.shown
            objectName: "TubeWindow.basic"
            visible: !panel.unsupported && !panel.advanced
            Layout.fillWidth: true
            Layout.fillHeight: true

            leftItems: [
                Rectangle {
                    objectName: "TubeWindow.showcase"
                    Layout.fillWidth: true
                    Layout.preferredHeight: Math.max(420, basicRight.implicitHeight)
                    radius: 10
                    color: Theme.control
                    border.width: 1
                    border.color: Theme.separator

                    // A warm pool of light behind the glass while the stage is on
                    Canvas {
                        id: pool
                        anchors.fill: parent
                        opacity: panel.on ? 1 : 0
                        Behavior on opacity { NumberAnimation { duration: panel.on ? 900 : 600; easing.type: Easing.InOutQuad } }
                        readonly property bool dark: Theme.dark
                        onDarkChanged: requestPaint()
                        onWidthChanged: requestPaint()
                        onHeightChanged: requestPaint()
                        onPaint: {
                            const ctx = getContext("2d")
                            ctx.reset()
                            const cx = width / 2, cy = height * 0.42
                            const grad = ctx.createRadialGradient(cx, cy, 0, cx, cy, 190)
                            grad.addColorStop(0, Qt.rgba(Theme.warning.r, Theme.warning.g, Theme.warning.b, 0.10))
                            grad.addColorStop(1, Qt.rgba(Theme.warning.r, Theme.warning.g, Theme.warning.b, 0))
                            ctx.fillStyle = grad
                            ctx.fillRect(0, 0, width, height)
                        }
                    }
                    ColumnLayout {
                        anchors.fill: parent
                        anchors.margins: 12
                        spacing: 12
                        Item { Layout.fillHeight: true }
                        Item {
                            Layout.alignment: Qt.AlignHCenter
                            Layout.preferredWidth: 168
                            Layout.preferredHeight: 280
                            readonly property int family: ToolsController.tubeFamily(panel.type)
                            TubeIllustration {
                                objectName: "TubeWindow.tube"
                                anchors.fill: parent
                                family: parent.family
                                part: TubeIllustration.Tube
                            }
                            TubeIllustration {
                                objectName: "TubeWindow.heater"
                                anchors.fill: parent
                                family: parent.family
                                part: TubeIllustration.Heater
                                // A heater takes a moment to glow and a little less to go dark
                                opacity: panel.on ? 1 : 0
                                Behavior on opacity { NumberAnimation { duration: panel.on ? 900 : 600; easing.type: Easing.InOutQuad } }
                            }
                            TubeIllustration {
                                objectName: "TubeWindow.bloom"
                                anchors.fill: parent
                                family: parent.family
                                part: TubeIllustration.Bloom
                                opacity: panel.on ? ToolsController.tubeGlow : 0
                                Behavior on opacity { NumberAnimation { duration: 100; easing.type: Easing.OutQuad } }
                            }
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 4
                            Text {
                                objectName: "TubeWindow.showcase.name"
                                Layout.alignment: Qt.AlignHCenter
                                text: panel.row ? panel.row.name : qsTr("Custom")
                                font.family: Theme.fontTitle.family
                                font.pixelSize: 17
                                font.weight: Font.DemiBold
                                color: Theme.text
                            }
                            Text {
                                objectName: "TubeWindow.showcase.caption"
                                Layout.fillWidth: true
                                horizontalAlignment: Text.AlignHCenter
                                text: !panel.row ? qsTr("Character set by hand. Pick a tube to load one, or fine-tune it in Advanced.")
                                    : panel.row.pushPull && !panel.xfmr ? qsTr("%1. Meant for use with the output stage, in Advanced.").arg(panel.row.style)
                                    : qsTr("%1.").arg(panel.row.style)
                                font: Theme.fontCaption
                                color: Theme.textMuted
                                wrapMode: Text.Wrap
                            }
                        }
                        Item { Layout.fillHeight: true }
                    }
                }
            ]
            rightItems: [
                ColumnLayout {
                    id: basicRight
                    objectName: "TubeWindow.shelf"
                    Layout.fillWidth: true
                    spacing: 14
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 10
                        SectionHeading { text: qsTr("Tube") }
                        HoverHandler { id: shelfHover }
                        HelpTip {
                            visible: shelfHover.hovered
                            text: qsTr("Loads the character of a real tube: its bias, asymmetry, knee hardness and sag. Drive, mix and everything in Advanced keep their values.")
                        }
                        Repeater {
                            model: [{ title: qsTr("Preamp triodes"), from: 1, to: 8 },
                                    { title: qsTr("Preamp pentodes"), from: 9, to: 10 },
                                    { title: qsTr("Power stages"), from: 11, to: 16 }]
                            ColumnLayout {
                                id: group
                                required property var modelData
                                Layout.fillWidth: true
                                spacing: 5
                                Text {
                                    text: group.modelData.title
                                    font: Theme.fontCaption
                                    color: Theme.textMuted
                                }
                                GridLayout {
                                    Layout.fillWidth: true
                                    columns: 4
                                    columnSpacing: 6
                                    rowSpacing: 6
                                    Repeater {
                                        model: group.modelData.to - group.modelData.from + 1
                                        EffectChip {
                                            required property int index
                                            readonly property int tubeType: group.modelData.from + index
                                            readonly property var tubeRow: ToolsController.tubeTypes[tubeType]
                                            objectName: "TubeWindow.type." + tubeType
                                            Layout.preferredWidth: 1
                                            enabled: panel.writable
                                            on: panel.type === tubeType
                                            text: tubeRow.shortName
                                            toolTip: qsTr("%1: %2").arg(tubeRow.name).arg(tubeRow.style)
                                            onClicked: ToolsController.setTube("type", tubeType)
                                        }
                                    }
                                }
                            }
                        }
                    }
                    EffectRule {}
                    TubeRow {
                        objectName: "TubeWindow.basicDrive"
                        key: "drive"
                        title: qsTr("Drive")
                        unit: qsTr("dB")
                        from: -30
                        to: 24
                        scrollStep: 0.5
                        resetValue: -12
                        ends: [qsTr("Clean"), qsTr("Overdrive")]
                        help: qsTr("How hard the tube is driven. Drive moves the knee, not the level: at the -12 dB default the knee sits 12 dB above full scale and the colour is subtle, the -30 dB floor is close to transparent, and the top of the range is overdrive.")
                    }
                    TubeRow {
                        objectName: "TubeWindow.basicMix"
                        key: "mix"
                        title: qsTr("Mix")
                        unit: qsTr("%")
                        from: 0
                        to: 100
                        decimals: 0
                        scrollStep: 1
                        resetValue: 100
                        ends: [qsTr("Dry"), qsTr("All tube")]
                        help: qsTr("Blends the tube with the untouched signal. Below 100% the original transients stay intact under the colour, which is the easiest way to use heavy drive subtly.")
                    }
                    EffectRule {}
                    Outputs { prefix: "TubeWindow.basicOutput" }
                }
            ]
        }

        // ---- Advanced: every parameter ----
        EffectColumns {
            id: advanced
            shown: panel.shown
            objectName: "TubeWindow.advanced"
            visible: !panel.unsupported && panel.advanced
            Layout.fillWidth: true
            Layout.fillHeight: true

            leftItems: [
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    EffectSection {
                        text: qsTr("Transfer curve")
                        EffectMenuButton {
                            id: tubePoints
                            objectName: "TubeWindow.startingPoints"
                            enabled: panel.writable
                            text: qsTr("Apply preset")
                            Instantiator {
                                model: ToolsController.tubeStartingPoints
                                delegate: AppMenuItem {
                                    required property int index
                                    required property var modelData
                                    objectName: "TubeWindow.startingPoint." + index
                                    text: qsTr("%1 - %2").arg(modelData.name).arg(modelData.detail)
                                    onTriggered: ToolsController.applyTubeStartingPoint(index)
                                }
                                onObjectAdded: (index, item) => tubePoints.menu.insertItem(index, item)
                                onObjectRemoved: (index, item) => tubePoints.menu.removeItem(item)
                            }
                        }
                    }
                    TubeTransferGraph {
                        id: transfer
                        objectName: "TubeWindow.graph"
                        Layout.fillWidth: true
                        Layout.preferredHeight: 188
                        active: panel.on
                        function pick(row, key) { return isNaN(row.liveValue) ? panel.value(key) : row.liveValue }
                        graph: ToolsController.tubeGraph(pick(driveRow, "drive"), pick(biasRow, "bias"), pick(asymRow, "asym"),
                                                         pick(hardnessRow, "hardness"), pick(mixRow, "mix"), pick(trimRow, "trim"))
                    }
                    // The harmonics a full-scale sine comes out with
                    RowLayout {
                        objectName: "TubeWindow.harmonics"
                        Layout.fillWidth: true
                        Layout.topMargin: 4
                        spacing: 16
                        SectionHeading {
                            Layout.fillWidth: true
                            text: qsTr("At full scale")
                        }
                        Harmonic {
                            objectName: "TubeWindow.harmonics.second"
                            label: qsTr("2nd")
                            db: transfer.graph.second
                        }
                        Harmonic {
                            objectName: "TubeWindow.harmonics.third"
                            label: qsTr("3rd")
                            db: transfer.graph.third
                        }
                        HoverHandler { id: harmonicsHover }
                        HelpTip {
                            visible: harmonicsHover.hovered
                            text: qsTr("Level of the second and third harmonic relative to the fundamental, for a full-scale sine through the static curve after mix and trim. Sag lowers the drive on sustained loud passages and the output stage adds its own low-frequency lift, so the running figures sit somewhat lower.")
                        }
                    }
                },
                EffectRule {},
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 14
                    SectionHeading { text: qsTr("Stage") }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 4
                        RowLayout {
                            Layout.fillWidth: true
                            Text {
                                Layout.fillWidth: true
                                text: qsTr("Tube")
                                font: Theme.fontBody
                                color: panel.writable ? Theme.text : Theme.textDisabled
                            }
                            AppComboBox {
                                objectName: "TubeWindow.typePicker"
                                implicitWidth: 190
                                label: qsTr("Tube")
                                enabled: panel.writable
                                model: ToolsController.tubeTypes.map(r => r.name)
                                currentIndex: panel.type
                                onActivated: index => ToolsController.setTube("type", index)
                            }
                        }
                        Text {
                            objectName: "TubeWindow.typeCaption"
                            Layout.fillWidth: true
                            text: !panel.row ? qsTr("Character controls as set, no tube row applied.")
                                : panel.row.pushPull && !panel.xfmr ? qsTr("%1. Meant for use with the output stage on.").arg(panel.row.style)
                                : qsTr("%1.").arg(panel.row.style)
                            font: Theme.fontCaption
                            color: Theme.textMuted
                            wrapMode: Text.Wrap
                        }
                        HoverHandler { id: typeHover }
                        HelpTip {
                            visible: typeHover.hovered
                            text: qsTr("Loads the bias, asymmetry, knee hardness and sag of a real tube. Drive, mix, the rectifier and the output stage are left alone. Editing any of the four character controls switches this to Custom.")
                        }
                    }
                    TubeRow {
                        id: driveRow
                        objectName: "TubeWindow.drive"
                        key: "drive"
                        title: qsTr("Drive")
                        unit: qsTr("dB")
                        from: -30
                        to: 24
                        scrollStep: 0.5
                        resetValue: -12
                        help: qsTr("Gain ahead of the shaper. At 0 dB a full-scale signal just reaches the knee, so this alone sets how hard the stage is driven. The shaper carries matching makeup gain, so clean material keeps its level at every drive; harmonics and sag rise with it.")
                    }
                    TubeRow {
                        id: mixRow
                        objectName: "TubeWindow.mix"
                        key: "mix"
                        title: qsTr("Mix")
                        unit: qsTr("%")
                        from: 0
                        to: 100
                        decimals: 0
                        scrollStep: 1
                        resetValue: 100
                        help: qsTr("Blend of the processed signal with the untouched input. The dry path is sample-aligned with the wet one, so blending never combs.")
                    }
                    TubeRow {
                        id: trimRow
                        objectName: "TubeWindow.trim"
                        key: "trim"
                        title: qsTr("Output Trim")
                        unit: qsTr("dB")
                        from: -12
                        to: 12
                        scrollStep: 0.5
                        resetValue: 0
                        help: qsTr("Level of the processed signal only. A hard-driven, strongly asymmetric setting can push the wet path above full scale; watch the output clip indicators and bring it back here.")
                    }
                },
                EffectRule {},
                Outputs { prefix: "TubeWindow.output" }
            ]
            rightItems: [
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 14
                    EffectSection {
                        text: qsTr("Character")
                        Text {
                            objectName: "TubeWindow.characterFrom"
                            text: panel.row ? qsTr("from %1").arg(panel.row.name) : qsTr("custom")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                        }
                    }
                    TubeRow {
                        id: biasRow
                        objectName: "TubeWindow.bias"
                        key: "bias"
                        title: qsTr("Bias")
                        unit: qsTr("%")
                        from: -100
                        to: 100
                        decimals: 0
                        scrollStep: 1
                        resetValue: 10
                        help: qsTr("Shifts the operating point along the curve. Positive values give the classic warm second harmonic that grows with level; negative values give the same amount with the even products inverted, which only matters when mixed with the dry signal.")
                    }
                    TubeRow {
                        id: asymRow
                        objectName: "TubeWindow.asym"
                        key: "asym"
                        title: qsTr("Asymmetry")
                        unit: qsTr("dB")
                        from: -12
                        to: 12
                        scrollStep: 0.5
                        resetValue: 3
                        help: qsTr("How much later the negative half reaches its knee than the positive half. Adds even-order content at heavy drive. Zero is symmetric, as in a push-pull stage.")
                    }
                    TubeRow {
                        id: hardnessRow
                        objectName: "TubeWindow.hardness"
                        key: "hardness"
                        title: qsTr("Knee Hardness")
                        unit: qsTr("%")
                        from: 0
                        to: 100
                        decimals: 0
                        scrollStep: 1
                        resetValue: 40
                        help: qsTr("Blends from a soft cubic knee (0%) to a harder quintic one (100%). Clean material stays at the same level at every setting; only how abruptly the stage runs out changes.")
                    }
                    TubeRow {
                        objectName: "TubeWindow.sag"
                        key: "sag"
                        title: qsTr("Sag")
                        unit: qsTr("%")
                        from: 0
                        to: 100
                        decimals: 0
                        scrollStep: 1
                        resetValue: 15
                        opacity: panel.rectifier === 0 ? 0.5 : 1
                        help: qsTr("Supply-sag compression: sustained heavy drive pulls the gain down slowly, then recovers. The rectifier below scales the depth and sets the timing.")
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 6
                        Text {
                            text: qsTr("Rectifier")
                            font: Theme.fontBody
                            color: panel.writable ? Theme.text : Theme.textDisabled
                        }
                        ToolSegments {
                            objectName: "TubeWindow.rectifier"
                            Layout.fillWidth: true
                            enabled: panel.writable
                            label: qsTr("Rectifier")
                            model: ToolsController.tubeRectifiers.map(r => r.name)
                            current: panel.rectifier
                            onChosen: index => ToolsController.setTube("rectifier", index)
                        }
                        Text {
                            objectName: "TubeWindow.rectifier.summary"
                            Layout.fillWidth: true
                            readonly property var rect: panel.rectifier > 0 && panel.rectifier < ToolsController.tubeRectifiers.length
                                                        ? ToolsController.tubeRectifiers[panel.rectifier] : null
                            text: rect ? qsTr("Sag depth x%1, %2 ms attack, %3 ms release.").arg(rect.depth.toFixed(1)).arg(Math.round(rect.attack)).arg(Math.round(rect.release))
                                       : qsTr("No sag: the supply holds up however hard the stage is driven.")
                            font: Theme.fontCaption
                            color: Theme.textMuted
                            wrapMode: Text.Wrap
                        }
                        HoverHandler { id: rectHover }
                        HelpTip {
                            visible: rectHover.hovered
                            text: qsTr("The power-supply rectifier sets how deep and how slow the sag is. Solid state switches sag off entirely; the valve rectifiers get progressively softer and slower from GZ34 to 5Y3.")
                        }
                    }
                },
                EffectRule {},
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 14
                    RowLayout {
                        Layout.fillWidth: true
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 2
                            SectionHeading { text: qsTr("Output stage") }
                            Text {
                                Layout.fillWidth: true
                                text: qsTr("A valve amplifier's loose grip on the speaker.")
                                font: Theme.fontCaption
                                color: Theme.textMuted
                                wrapMode: Text.Wrap
                            }
                        }
                        AppSwitch {
                            objectName: "TubeWindow.xfmr"
                            text: ""
                            enabled: panel.writable
                            checked: panel.xfmr
                            onToggled: ToolsController.setTube("xfmr", checked ? 1 : 0)
                            Accessible.name: qsTr("Output stage")
                        }
                        HoverHandler { id: stageHover }
                        HelpTip {
                            visible: stageHover.hovered
                            text: qsTr("A tube amplifier's high source impedance lets the speaker's own impedance curve shape the response: a broad bump at the woofer resonance and a small lift at the top. Nothing here is nonlinear, and the firmware skips the whole stage while it is off.")
                        }
                    }
                    // The firmware leaves the stage out while it is off, so its controls go too
                    TubeRow {
                        id: dampingRow
                        objectName: "TubeWindow.damping"
                        visible: panel.xfmr
                        key: "damping"
                        title: qsTr("Damping Factor")
                        unit: ""
                        from: 1
                        to: 20
                        scrollStep: 0.5
                        resetValue: 2
                        ends: [qsTr("1 (loose)"), qsTr("20 (tight)")]
                        help: qsTr("The speaker's nominal impedance divided by the amplifier's source impedance. A single-ended triode amplifier without feedback sits around 2 to 3; a push-pull pentode amplifier with feedback around 8 to 15. It sets the size of both the bell and the top lift.")
                    }
                    Text {
                        objectName: "TubeWindow.lift"
                        visible: panel.xfmr
                        Layout.fillWidth: true
                        readonly property var lift: ToolsController.transformerLift(isNaN(dampingRow.liveValue) ? panel.value("damping") : dampingRow.liveValue)
                        text: qsTr("+%1 dB at resonance, +%2 dB at the top.").arg(lift.bell.toFixed(1)).arg(lift.top.toFixed(1))
                        font: Theme.fontCaption
                        color: Theme.textMuted
                    }
                    TubeRow {
                        objectName: "TubeWindow.resonance"
                        visible: panel.xfmr
                        key: "resonance"
                        title: qsTr("Speaker Resonance")
                        unit: qsTr("Hz")
                        from: 30
                        to: 150
                        decimals: 0
                        scrollStep: 1
                        resetValue: 95
                        ends: [qsTr("30 Hz"), qsTr("150 Hz")]
                        help: qsTr("Where the loudspeaker resonates in its enclosure, which is where the bell sits. Q is fixed at 0.707, so the bump is broad. 85 Hz suits a typical small to medium woofer; larger drivers sit lower.")
                    }
                }
            ]
        }
    }
}
