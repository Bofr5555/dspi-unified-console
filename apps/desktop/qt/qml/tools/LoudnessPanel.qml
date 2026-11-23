import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- Loudness compensation (the Mac's LoudnessView, 780×400, at
// least 740×390): the header with the switch; the ISO 226:2003 compensation
// curve as it is 40 dB below the reference level on the left; the reference
// level, the intensity and, on firmware with the output mask (wire V19), the
// outputs on the right.
// -- WP LAY1a -- A panel (Panel.qml): PanelTable's row for
// tools.loudness gives its window's title and sizes.
Panel {
    id: panel
    objectName: "LoudnessPanel"
    // -- WP QTPOP --
    fitBodies: [body]

    readonly property bool writable: EffectsController.writable

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        EffectHeader {
            objectName: "LoudnessWindow.header"
            touchPath: "loud."   // -- WP AIVIS --
            Layout.fillWidth: true
            iconName: "volume-2"
            title: qsTr("Loudness Compensation")
            subtitle: qsTr("ISO 226:2003 Fletcher-Munson")
            on: EffectsController.loudnessOn
            switchEnabled: panel.writable
            onToggled: on => EffectsController.setEnabled("loudness", on)
        }
        EffectRule {}

        EffectColumns {
            id: body
            shown: panel.shown
            Layout.fillWidth: true
            Layout.fillHeight: true

            leftItems: ColumnLayout {
                Layout.fillWidth: true
                spacing: 6
                SectionHeading { text: qsTr("Compensation Curve") }
                EffectGraph {
                    objectName: "LoudnessWindow.graph"
                    Layout.fillWidth: true
                    // As tall as the column beside it, at least 160
                    Layout.preferredHeight: Math.max(160, body.rightHeight - 24)
                    title: qsTr("Compensation Curve")
                    active: EffectsController.loudnessOn
                    readonly property var graphData: EffectsController.loudnessGraph(
                        isNaN(refRow.liveValue) ? EffectsController.loudnessRef : refRow.liveValue,
                        isNaN(intensityRow.liveValue) ? EffectsController.loudnessIntensity : intensityRow.liveValue)
                    curves: [{ points: graphData.curve, color: Theme.accent }]
                    gridDb: graphData.gridDb
                    gridY: graphData.gridY
                    badge: qsTr("Curve at -40dB")
                }
            }
            rightItems: [
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 14
                    SectionHeading { text: qsTr("Parameters") }
                    EffectParameterRow {
                        id: refRow
                        objectName: "LoudnessWindow.ref"
                        Layout.fillWidth: true
                        enabled: panel.writable
                        title: qsTr("Reference SPL")
                        unit: qsTr("dB")
                        value: EffectsController.loudnessRef
                        from: 40
                        to: 100
                        resetValue: 83
                        caption: qsTr("SPL at 1 kHz when USB volume is 0 dB. Lower = more compensation per dB of volume reduction.")
                        onLive: v => EffectsController.setLoudnessRef(v)
                        onCommit: v => EffectsController.setLoudnessRef(v)
                    }
                    EffectRule {}
                    EffectParameterRow {
                        id: intensityRow
                        objectName: "LoudnessWindow.intensity"
                        Layout.fillWidth: true
                        enabled: panel.writable
                        title: qsTr("Intensity")
                        unit: qsTr("%")
                        value: EffectsController.loudnessIntensity
                        from: 0
                        to: 200
                        resetValue: 100
                        caption: qsTr("Scales the ISO 226 compensation. 100% = standard curve. 0% = bypassed. >100% = exaggerated.")
                        onLive: v => EffectsController.setLoudnessIntensity(v)
                        onCommit: v => EffectsController.setLoudnessIntensity(v)
                    }
                },
                EffectRule { visible: EffectsController.loudnessMaskSupported },
                ColumnLayout {
                    objectName: "LoudnessWindow.outputs"
                    visible: EffectsController.loudnessMaskSupported
                    Layout.fillWidth: true
                    spacing: 10
                    EffectSection {
                        text: qsTr("Outputs")
                        EffectMenuButton {
                            objectName: "LoudnessWindow.outputs.presets"
                            enabled: panel.writable
                            text: qsTr("Presets")
                            AppMenuItem {
                                objectName: "LoudnessWindow.outputs.all"
                                text: qsTr("All outputs")
                                onTriggered: EffectsController.setLoudnessMask(0xFFFF)
                            }
                            AppMenuItem {
                                objectName: "LoudnessWindow.outputs.first"
                                text: qsTr("Slot 1 only (Headphones)")
                                onTriggered: EffectsController.setLoudnessMask(0x0003)
                            }
                            AppMenuItem {
                                objectName: "LoudnessWindow.outputs.none"
                                text: qsTr("None")
                                onTriggered: EffectsController.setLoudnessMask(0x0000)
                            }
                        }
                    }
                    Text {
                        Layout.fillWidth: true
                        text: qsTr("Compensate only the outputs feeding your low-level listening chain. Keep bass-managed pairs (mains + sub) together so the crossover stays coherent.")
                        font: Theme.fontCaption
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                    RowLayout {
                        Layout.fillWidth: true
                        spacing: 6
                        Repeater {
                            model: EffectsController.numOutputs
                            EffectChip {
                                required property int index
                                objectName: "LoudnessWindow.output." + index
                                enabled: panel.writable
                                on: (EffectsController.loudnessMask & (1 << index)) !== 0
                                text: String(index + 1)
                                toolTip: { EffectsController.revision; return EffectsController.outputName(index) }
                                onClicked: EffectsController.toggleLoudnessOutput(index)
                            }
                        }
                    }
                }
            ]
        }
    }
}
