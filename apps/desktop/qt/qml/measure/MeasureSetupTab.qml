import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P5QUI -- The Measure tab (the web's setup.tsx and run.tsx): the
// microphone and its calibration, the placement (and coupler), the outputs
// with their roles, speakers and alignment groups, the sweep; the preflight in
// words; then the run with its progress, the ramp's level and the timing
// verdict. The core decides everything; Stop is in the window's bar.
ScrollView {
    id: tab
    signal showResults()
    clip: true
    contentWidth: availableWidth

    readonly property var mc: MeasureController
    readonly property var roles: [
        { id: "full", text: qsTr("Full range") }, { id: "sub", text: qsTr("Subwoofer") },
        { id: "low", text: qsTr("Low (woofer)") }, { id: "mid", text: qsTr("Mid") },
        { id: "high", text: qsTr("High (tweeter)") }
    ]
    readonly property var placements: [
        { id: "listeningPosition", text: qsTr("Listening position"), detail: qsTr("Speakers: one position, with timing (alignment, crossover, EQ).") },
        { id: "multiPosition", text: qsTr("Several positions"), detail: qsTr("Speakers: 3 to 9 positions averaged; position 1 is the timing reference.") },
        { id: "nearField", text: qsTr("Near-field"), detail: qsTr("Speakers: the microphone close to each driver; kept as a labelled near-field measurement.") },
        { id: "headphones", text: qsTr("Headphones on a coupler"), detail: qsTr("On the coupler, not on a head: 1 to 9 reseats averaged.") }
    ]
    readonly property var couplers: [
        { id: "iec60318_4", text: qsTr("IEC 60318-4 ear simulator (711)") },
        { id: "flatPlate", text: qsTr("Flat-plate coupler") },
        { id: "hats", text: qsTr("HATS or another coupler with pinna") }
    ]
    // The microphone choices: the simulated one (the demo), then the inputs
    readonly property var micChoices: {
        const out = []
        if (mc.simAvailable) out.push({ id: "sim", text: mc.simLabel || qsTr("Simulated microphone") })
        out.push({ id: "", text: qsTr("The system's default input") })
        for (const i of mc.inputs) out.push({ id: i.id, text: i.name })
        return out
    }
    function indexOf(list, id) {
        for (let i = 0; i < list.length; ++i) if (list[i].id === id) return i
        return 0
    }
    readonly property bool editable: !mc.running && mc.playBlocker.length === 0
    // -- WP P5FOLLOW -- -1 while no placement is chosen
    readonly property int placementIndex: {
        for (let i = 0; i < placements.length; ++i) if (placements[i].id === mc.placement) return i
        return -1
    }

    ColumnLayout {
        width: tab.availableWidth
        spacing: Theme.spacing

        Card {
            title: qsTr("Microphone")
            Layout.fillWidth: true
            RowLayout {
                spacing: Theme.spacingSmall
                AppComboBox {
                    objectName: "MeasureSetup.mic"
                    label: qsTr("Microphone")
                    Layout.preferredWidth: 300
                    enabled: tab.editable
                    model: tab.micChoices
                    textRole: "text"
                    currentIndex: tab.indexOf(tab.micChoices, mc.micChoice)
                    onActivated: index => mc.micChoice = tab.micChoices[index].id
                }
                AppButton {
                    text: qsTr("Refresh")
                    enabled: tab.editable
                    onClicked: mc.refreshInputs()
                }
            }
            Text {
                Layout.fillWidth: true
                visible: mc.micChoice === "sim"
                text: qsTr("The demo DSPi's simulated room: nothing is played or captured, the microphone is arithmetic in the core.")
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.WordWrap
            }
            RowLayout {
                spacing: Theme.spacingSmall
                Text {
                    objectName: "MeasureSetup.calibration"
                    Layout.fillWidth: true
                    text: mc.micCalibrated ? mc.micSummary : qsTr("Uncalibrated: levels are relative dB.")
                    font: Theme.fontBody
                    color: Theme.text
                    wrapMode: Text.WordWrap
                }
                AppButton {
                    text: qsTr("Import Calibration…")
                    enabled: tab.editable
                    onClicked: mc.chooseMicFile()
                }
                AppButton {
                    visible: mc.micCalibrated
                    text: qsTr("Uncalibrated")
                    enabled: tab.editable
                    onClicked: mc.clearMic()
                }
            }
        }

        Card {
            title: qsTr("Placement")
            Layout.fillWidth: true
            RowLayout {
                spacing: Theme.spacingSmall
                AppComboBox {
                    objectName: "MeasureSetup.placement"
                    label: qsTr("Placement")
                    Layout.preferredWidth: 240
                    enabled: tab.editable
                    model: tab.placements
                    textRole: "text"
                    // -- WP P5FOLLOW -- nothing chosen until the person chooses
                    currentIndex: tab.placementIndex
                    displayText: tab.placementIndex < 0 ? qsTr("Choose the placement") : currentText
                    onActivated: index => mc.placement = tab.placements[index].id
                }
                AppSpinBox {
                    objectName: "MeasureSetup.count"
                    visible: mc.placement === "multiPosition" || mc.placement === "headphones"
                    label: mc.placement === "headphones" ? qsTr("Reseats") : qsTr("Positions")
                    enabled: tab.editable
                    realFrom: mc.placement === "headphones" ? 1 : 3
                    realTo: 9
                    realValue: mc.count
                    onCommitted: value => mc.count = Math.round(value)
                }
                AppComboBox {
                    objectName: "MeasureSetup.coupler"
                    visible: mc.placement === "headphones"
                    label: qsTr("Coupler")
                    Layout.preferredWidth: 260
                    enabled: tab.editable
                    model: tab.couplers
                    textRole: "text"
                    currentIndex: tab.indexOf(tab.couplers, mc.coupler)
                    onActivated: index => mc.coupler = tab.couplers[index].id
                }
            }
            Text {
                Layout.fillWidth: true
                text: tab.placementIndex < 0
                      ? qsTr("Where is the microphone? Speakers or headphones on a coupler: the levels depend on it, and headphones are measured much more quietly.")
                      : tab.placements[tab.placementIndex].detail
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.WordWrap
            }
        }

        Card {
            title: qsTr("Outputs")
            Layout.fillWidth: true
            Text {
                Layout.fillWidth: true
                text: qsTr("The outputs measured together in one walked run are an alignment group: their delays and polarity are compared within the run only.")
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.WordWrap
            }
            Repeater {
                model: mc.setupOutputs
                RowLayout {
                    required property var modelData
                    spacing: Theme.spacingSmall
                    AppCheckBox {
                        objectName: "MeasureSetup.output." + modelData.output
                        text: modelData.label
                        Layout.preferredWidth: 200
                        enabled: tab.editable
                        checked: modelData.on
                        onToggled: mc.setOutputOn(modelData.output, checked)
                    }
                    AppComboBox {
                        label: qsTr("Way")
                        Layout.preferredWidth: 150
                        enabled: tab.editable && modelData.on
                        model: tab.roles
                        textRole: "text"
                        currentIndex: tab.indexOf(tab.roles, modelData.role)
                        onActivated: index => mc.setOutputRole(modelData.output, tab.roles[index].id)
                    }
                    BorderedField {
                        label: qsTr("Speaker")
                        Layout.preferredWidth: 60
                        enabled: tab.editable && modelData.on
                        text: modelData.speaker
                        onCommitted: t => mc.setOutputSpeaker(modelData.output, t)
                    }
                    AppSpinBox {
                        label: qsTr("Group")
                        implicitWidth: 90
                        enabled: tab.editable && modelData.on
                        realFrom: 0
                        realTo: 15
                        realValue: modelData.group
                        onCommitted: value => mc.setOutputGroup(modelData.output, Math.round(value))
                    }
                }
            }
            AppButton {
                text: qsTr("Propose from the Device")
                enabled: tab.editable
                onClicked: mc.proposeLayout()
            }
        }

        Card {
            title: qsTr("Sweep")
            Layout.fillWidth: true
            RowLayout {
                spacing: Theme.spacing
                Text { text: qsTr("Sweep length"); font: Theme.fontBody; color: Theme.text }
                AppSpinBox {
                    objectName: "MeasureSetup.sweep"
                    label: qsTr("Sweep length")
                    enabled: tab.editable
                    decimals: 1
                    realFrom: 1
                    realTo: 15
                    realStep: 0.5
                    unit: qsTr("s")
                    realValue: mc.sweepMs / 1000
                    onCommitted: value => mc.sweepMs = Math.round(value * 1000)
                }
                Text { text: qsTr("Passes"); font: Theme.fontBody; color: Theme.text }
                AppSpinBox {
                    objectName: "MeasureSetup.passes"
                    label: qsTr("Passes")
                    enabled: tab.editable
                    realFrom: 2
                    realTo: 8
                    realValue: mc.passes
                    onCommitted: value => mc.passes = Math.round(value)
                }
                Text { text: qsTr("Name"); font: Theme.fontBody; color: Theme.text }
                BorderedField {
                    objectName: "MeasureSetup.name"
                    label: qsTr("Name")
                    Layout.fillWidth: true
                    enabled: tab.editable
                    placeholderText: qsTr("By the date")
                    text: mc.runName
                    onCommitted: t => mc.runName = t
                }
            }
            // -- WP P5FOLLOW -- (review P5-E) a hint, no minimum
            Text {
                objectName: "MeasureSetup.passesHint"
                visible: mc.passes === 2
                Layout.fillWidth: true
                text: qsTr("Two passes cannot judge timing; use three or more for auto-tune.")
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.WordWrap
            }
            Text {
                Layout.fillWidth: true
                text: qsTr("The level is the core's: it starts quiet and rises in steps until the microphone hears the output well, never past the ceiling.")
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.WordWrap
            }
        }

        RowLayout {
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "MeasureSetup.preflight"
                text: qsTr("Check")
                enabled: tab.editable && mc.setupProblem.length === 0
                onClicked: mc.preflight()
            }
            AppButton {
                objectName: "MeasureSetup.start"
                text: qsTr("Measure")
                primary: true
                enabled: tab.editable && mc.setupProblem.length === 0
                onClicked: mc.start()
            }
            AppButton {
                objectName: "MeasureSetup.next"
                visible: mc.waitingPosition
                text: qsTr("Next Position")
                primary: true
                onClicked: mc.nextPosition()
            }
            Text {
                Layout.fillWidth: true
                text: mc.setupProblem
                font: Theme.fontBody
                color: Theme.warning
                wrapMode: Text.WordWrap
            }
        }

        // The preflight in words
        ColumnLayout {
            objectName: "MeasureSetup.preflightLines"
            visible: mc.preflightDone
            Layout.fillWidth: true
            spacing: 4
            Text {
                text: mc.preflightClear ? qsTr("Clear to measure.") : qsTr("Not clear to measure:")
                font: Theme.fontBody
                color: mc.preflightClear ? Theme.success : Theme.dangerText
            }
            Repeater {
                model: mc.preflightLines
                Text {
                    required property var modelData
                    Layout.fillWidth: true
                    text: (modelData.kind === "blocker" ? "• " : qsTr("Warning: ")) + modelData.text
                    font: Theme.fontBody
                    color: modelData.kind === "blocker" ? Theme.dangerText : Theme.warning
                    wrapMode: Text.WordWrap
                }
            }
        }

        // The run
        Card {
            title: qsTr("Run")
            Layout.fillWidth: true
            Text {
                objectName: "MeasureSetup.phase"
                Layout.fillWidth: true
                text: mc.phaseText
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.WordWrap
                Accessible.role: Accessible.StaticText
            }
            ProgressBar {
                objectName: "MeasureSetup.progress"
                visible: mc.measuring
                Layout.fillWidth: true
                from: 0
                to: 100
                value: mc.progress
            }
            Text {
                objectName: "MeasureSetup.issue"
                visible: text.length > 0 && !mc.measuring
                Layout.fillWidth: true
                text: mc.issueText
                font: Theme.fontBody
                color: mc.phase === "aborted" ? Theme.textMuted : Theme.dangerText
                wrapMode: Text.WordWrap
            }
            Text {
                objectName: "MeasureSetup.timing"
                visible: text.length > 0
                Layout.fillWidth: true
                text: mc.timingText
                font: Theme.fontBody
                color: mc.timingReliable ? Theme.success : Theme.warning
                wrapMode: Text.WordWrap
            }
            AppButton {
                visible: mc.hasResult && mc.resultFrom === "run" && !mc.measuring
                text: qsTr("Show the Results")
                onClicked: tab.showResults()
            }
        }
    }
}
