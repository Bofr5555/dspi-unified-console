import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- Psychoacoustic bass (the Mac's PsychoacousticBassView, 780×500,
// at least 740×496): the header with the switch; on the left a sketch of what
// it does (the original band below the cutoff, the harmonics from the cutoff
// to four times it) with the starting points, over the harmonics controls;
// on the right the outputs and the shaping. Firmware before wire V23 has none
// of it and gets a note instead.
// -- WP DL1 -- Modern: the intro row, the sketch in a GraphFrame, the
// outputs as option chips, "Apply preset" and "Presets" secondary buttons.
// -- WP LAY1a -- A panel (Panel.qml): PanelTable's row for
// tools.psybass gives its window's title and sizes.
Panel {
    id: panel
    objectName: "PsybassPanel"
    // -- WP QTPOP --
    fitBodies: [body]

    readonly property bool writable: EffectsController.writable && EffectsController.psybassSupported
    readonly property bool unsupported: EffectsController.connected && !EffectsController.psybassSupported

    // A dashed upright line at `at` (the cutoff markers), with its label at the top
    component Marker: Item {
        id: marker
        property real at: 0
        property real strength: 0.4
        property string label
        anchors.fill: parent
        opacity: strength
        Column {
            x: marker.at
            y: 14
            spacing: 3
            Repeater {
                model: Math.max(0, Math.ceil((marker.height - 14) / 6))
                Rectangle {
                    width: 1
                    height: 3
                    color: Theme.text
                }
            }
        }
        Text {
            x: marker.at - width / 2
            y: 2
            text: marker.label
            font.family: Theme.fontMono.family
            font.pixelSize: 9
            font.weight: Font.DemiBold
            color: Theme.text
        }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        EffectHeader {
            objectName: "PsybassWindow.header"
            touchPath: "bass."   // -- WP AIVIS --
            Layout.fillWidth: true
            iconName: "audio-lines"
            title: qsTr("Psychoacoustic Bass")
            subtitle: qsTr("Phantom fundamental bass enhancement")
            on: EffectsController.psybassOn
            switchEnabled: panel.writable
            onToggled: on => EffectsController.setEnabled("psybass", on)
        }
        EffectRule {}

        ColumnLayout {
            objectName: "PsybassWindow.unsupported"
            visible: panel.unsupported
            Layout.fillWidth: true
            Layout.fillHeight: true
            Layout.margins: 24
            spacing: 10
            Item { Layout.fillHeight: true }
            Icon {
                Layout.alignment: Qt.AlignHCenter
                name: "triangle-alert"
                size: 28
                color: Theme.textMuted
            }
            Text {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("Requires firmware with wire format V23 or newer.")
                font: Theme.fontBody
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            Text {
                Layout.fillWidth: true
                horizontalAlignment: Text.AlignHCenter
                text: qsTr("Update the DSPi firmware to use Psychoacoustic Bass.")
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            Item { Layout.fillHeight: true }
        }

        EffectColumns {
            id: body
            shown: panel.shown
            objectName: "PsybassWindow.body"
            visible: !panel.unsupported
            Layout.fillWidth: true
            Layout.fillHeight: true

            leftItems: [
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    EffectSection {
                        text: qsTr("Spectrum")
                        EffectMenuButton {
                            id: startingPoints
                            objectName: "PsybassWindow.startingPoints"
                            enabled: panel.writable
                            text: qsTr("Apply preset")
                            Instantiator {
                                model: EffectsController.psybassStartingPoints
                                delegate: AppMenuItem {
                                    required property int index
                                    required property var modelData
                                    objectName: "PsybassWindow.startingPoint." + index
                                    text: qsTr("%1 - %2").arg(modelData.name).arg(modelData.detail)
                                    onTriggered: EffectsController.applyPsybassStartingPoint(index)
                                }
                                onObjectAdded: (index, item) => startingPoints.menu.insertItem(index, item)
                                onObjectRemoved: (index, item) => startingPoints.menu.removeItem(item)
                            }
                        }
                    }
                    EffectGraph {
                        id: sketch
                        objectName: "PsybassWindow.graph"
                        Layout.fillWidth: true
                        Layout.preferredHeight: LayoutController.modern ? 200 : 160   // -- WP DL1 --
                        title: qsTr("Spectrum")
                        active: EffectsController.psybassOn
                        bottomInset: 6
                        legend: [
                            { color: Theme.accent, label: qsTr("Original"), bar: true },
                            { color: Theme.warning, label: qsTr("Harmonics"), bar: true }
                        ]
                        readonly property var bands: EffectsController.psybassSketch(
                            isNaN(cutoffRow.liveValue) ? EffectsController.psybassCutoff : cutoffRow.liveValue,
                            isNaN(harmonicsRow.liveValue) ? EffectsController.psybassHarmonics : harmonicsRow.liveValue,
                            isNaN(originalRow.liveValue) ? EffectsController.psybassOriginal : originalRow.liveValue)

                        Item {
                            visible: sketch.active
                            width: sketch.plotWidth
                            height: sketch.plotHeight
                            Rectangle {
                                objectName: "PsybassWindow.graph.original"
                                x: 0
                                width: sketch.bands.fc * parent.width
                                height: sketch.bands.original * parent.height
                                y: parent.height - height
                                color: Theme.accent
                                opacity: 0.35
                            }
                            Rectangle {
                                objectName: "PsybassWindow.graph.harmonics"
                                x: sketch.bands.fc * parent.width
                                width: Math.max(0, (sketch.bands.fc4 - sketch.bands.fc) * parent.width)
                                height: sketch.bands.harmonics * parent.height
                                y: parent.height - height
                                color: Theme.warning
                                opacity: 0.55
                            }
                            Marker {
                                at: sketch.bands.fc * width
                                label: "fc"
                            }
                            Marker {
                                at: sketch.bands.fc4 * width
                                strength: 0.25
                                label: "4fc"
                            }
                        }
                    }
                },
                EffectRule {},
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 14
                    SectionHeading { text: qsTr("Harmonics") }
                    EffectParameterRow {
                        id: cutoffRow
                        objectName: "PsybassWindow.cutoff"
                        Layout.fillWidth: true
                        enabled: panel.writable
                        title: qsTr("Cutoff Frequency")
                        unit: qsTr("Hz")
                        value: EffectsController.psybassCutoff
                        from: 30
                        to: 300
                        decimals: 0
                        scrollStep: 1
                        resetValue: 80
                        caption: qsTr("The speaker's low-frequency limit. Content below this feeds the harmonic generator; generated harmonics span roughly this to 4x.")
                        onLive: v => EffectsController.setPsybass("cutoff", v)
                        onCommit: v => EffectsController.setPsybass("cutoff", v)
                    }
                    EffectRule {}
                    EffectParameterRow {
                        id: harmonicsRow
                        objectName: "PsybassWindow.harmonics"
                        Layout.fillWidth: true
                        enabled: panel.writable
                        title: qsTr("Harmonics")
                        unit: qsTr("dB")
                        value: EffectsController.psybassHarmonics
                        from: -24
                        to: 12
                        scrollStep: 0.5
                        resetValue: 0
                        caption: qsTr("Level of the synthesized harmonics. The primary amount-of-effect control. Higher = more perceived bass.")
                        onLive: v => EffectsController.setPsybass("harmonics", v)
                        onCommit: v => EffectsController.setPsybass("harmonics", v)
                    }
                }
            ]
            rightItems: [
                ColumnLayout {
                    objectName: "PsybassWindow.outputs"
                    Layout.fillWidth: true
                    spacing: 10
                    EffectSection {
                        text: qsTr("Outputs")
                        EffectMenuButton {
                            objectName: "PsybassWindow.outputs.presets"
                            enabled: panel.writable
                            text: qsTr("Presets")
                            AppMenuItem {
                                objectName: "PsybassWindow.outputs.all"
                                text: qsTr("All outputs")
                                onTriggered: EffectsController.setPsybassMask(EffectsController.allOutputsMask())
                            }
                            AppMenuItem {
                                objectName: "PsybassWindow.outputs.noSub"
                                text: qsTr("Exclude sub (recommended)")
                                onTriggered: EffectsController.setPsybassMask(EffectsController.outputsWithoutSubMask())
                            }
                            AppMenuItem {
                                objectName: "PsybassWindow.outputs.none"
                                text: qsTr("None")
                                onTriggered: EffectsController.setPsybassMask(0x0000)
                            }
                        }
                    }
                    Text {
                        Layout.fillWidth: true
                        text: qsTr("Enhance only the small-speaker outputs. Mask off the sub and any full-range outputs - synthesizing harmonics on a channel that can reproduce real bass is counterproductive.")
                        font: Theme.fontCaption
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                    EffectChipRow {   // -- WP DL1 --
                        Layout.fillWidth: true
                        Repeater {
                            model: EffectsController.numOutputs
                            EffectChip {
                                required property int index
                                objectName: "PsybassWindow.output." + index
                                enabled: panel.writable
                                on: (EffectsController.psybassMask & (1 << index)) !== 0
                                text: String(index + 1)
                                toolTip: { EffectsController.revision; return EffectsController.outputName(index) }
                                onClicked: EffectsController.togglePsybassOutput(index)
                            }
                        }
                    }
                },
                EffectRule {},
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 14
                    SectionHeading { text: qsTr("Shaping") }
                    EffectParameterRow {
                        objectName: "PsybassWindow.drive"
                        Layout.fillWidth: true
                        enabled: panel.writable
                        title: qsTr("Drive")
                        unit: qsTr("dB")
                        value: EffectsController.psybassDrive
                        from: 0
                        to: 18
                        scrollStep: 0.5
                        resetValue: 6
                        caption: qsTr("Pre-gain into the odd-harmonic soft clipper. Higher makes the effect audible on quieter passages. Mostly affects aggressive character.")
                        onLive: v => EffectsController.setPsybass("drive", v)
                        onCommit: v => EffectsController.setPsybass("drive", v)
                    }
                    EffectRule {}
                    EffectParameterRow {
                        objectName: "PsybassWindow.character"
                        Layout.fillWidth: true
                        enabled: panel.writable
                        title: qsTr("Character")
                        unit: qsTr("%")
                        value: EffectsController.psybassCharacter
                        from: 0
                        to: 100
                        decimals: 0
                        scrollStep: 1
                        resetValue: 50
                        ends: [qsTr("Warm"), qsTr("Aggressive")]
                        onLive: v => EffectsController.setPsybass("character", v)
                        onCommit: v => EffectsController.setPsybass("character", v)
                    }
                    EffectRule {}
                    EffectParameterRow {
                        id: originalRow
                        objectName: "PsybassWindow.original"
                        Layout.fillWidth: true
                        enabled: panel.writable
                        title: qsTr("Original Bass")
                        unit: qsTr("dB")
                        value: EffectsController.psybassOriginal
                        from: -60
                        to: 0
                        scrollStep: 1
                        resetValue: 0
                        caption: qsTr("Level of the un-reproducible fundamental below the cutoff. Lower attenuates it, freeing driver excursion and headroom. -60 dB is full removal. Speaker protection.")
                        onLive: v => EffectsController.setPsybass("original", v)
                        onCommit: v => EffectsController.setPsybass("original", v)
                    }
                }
            ]
        }
    }
}
