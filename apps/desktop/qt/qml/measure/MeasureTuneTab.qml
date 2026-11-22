import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P5QUI -- The auto-tune (PLAN-P5 section 1.4; the web's P5WAT wizard):
// the layout with its alignment groups and programme feeds; the proposal of
// the Results tab's measurement with every write in words, what it was held
// to, what is withdrawn and why; the mandatory summed check; the guarded
// apply with its countdown; the verify; Keep or Revert; the A/B compare.
//
// Keep after a verify that did not pass: the core refuses it once
// (`measure.tune.keep_unverified`) and this tab asks again, in a question of
// its own; only "Keep Anyway", pressed by the person, keeps the tune.
ScrollView {
    id: tab
    clip: true
    contentWidth: availableWidth

    readonly property var tc: TuneController
    readonly property var mc: MeasureController
    readonly property var roles: [
        { id: "full", text: qsTr("Full range") }, { id: "sub", text: qsTr("Subwoofer") },
        { id: "low", text: qsTr("Low (woofer)") }, { id: "mid", text: qsTr("Mid") },
        { id: "high", text: qsTr("High (tweeter)") }
    ]
    function indexOf(list, id) {
        for (let i = 0; i < list.length; ++i) if (list[i].id === id) return i
        return 0
    }
    readonly property bool idle: !mc.running && !tc.busy
    readonly property bool beforeApply: !tc.applied && tc.stage !== "kept"

    Connections {
        target: TuneController
        function onKeepConfirmNeededChanged() {
            if (TuneController.keepConfirmNeeded) keepQuestion.open()
        }
    }

    ColumnLayout {
        width: tab.availableWidth
        spacing: Theme.spacing

        Text {
            objectName: "MeasureTune.stage"
            Layout.fillWidth: true
            text: tc.stageText
            font: Theme.fontTitle
            // -- WP REVERTW -- a revert not fully successful is an error
            color: tc.stage === "revertIncomplete" ? Theme.dangerText : Theme.text
            wrapMode: Text.WordWrap
        }
        Text {
            objectName: "MeasureTune.error"
            visible: text.length > 0
            Layout.fillWidth: true
            text: tc.error
            font: Theme.fontBody
            color: Theme.dangerText
            wrapMode: Text.WordWrap
        }
        Text {
            visible: text.length > 0
            Layout.fillWidth: true
            text: tc.issueText
            font: Theme.fontBody
            color: Theme.dangerText
            wrapMode: Text.WordWrap
        }

        // 1. The layout
        Card {
            title: qsTr("1. Layout")
            Layout.fillWidth: true
            visible: tab.beforeApply
            Text {
                Layout.fillWidth: true
                text: mc.hasResult ? qsTr("From \"%1\".").arg(mc.resultName || qsTr("the measurement"))
                                   : qsTr("No measurement yet: measure on the Measure tab, or open one on Results.")
                font: Theme.fontBody
                color: mc.hasResult ? Theme.text : Theme.warning
                wrapMode: Text.WordWrap
            }
            Repeater {
                model: tc.layout
                RowLayout {
                    required property var modelData
                    spacing: Theme.spacingSmall
                    AppCheckBox {
                        objectName: "MeasureTune.output." + modelData.output
                        text: modelData.label
                        Layout.preferredWidth: 200
                        enabled: tab.idle
                        checked: modelData.on
                        onToggled: tc.setOutputOn(modelData.output, checked)
                    }
                    AppComboBox {
                        label: qsTr("Way")
                        Layout.preferredWidth: 150
                        enabled: tab.idle && modelData.on
                        model: tab.roles
                        textRole: "text"
                        currentIndex: tab.indexOf(tab.roles, modelData.role)
                        onActivated: index => tc.setOutputRole(modelData.output, tab.roles[index].id)
                    }
                    BorderedField {
                        label: qsTr("Speaker")
                        Layout.preferredWidth: 60
                        enabled: tab.idle && modelData.on
                        text: modelData.speaker
                        onCommitted: t => tc.setOutputSpeaker(modelData.output, t)
                    }
                    AppSpinBox {
                        label: qsTr("Group")
                        implicitWidth: 90
                        enabled: tab.idle && modelData.on
                        realFrom: 0
                        realTo: 15
                        realValue: modelData.group
                        onCommitted: value => tc.setOutputGroup(modelData.output, Math.round(value))
                    }
                    Text {
                        Layout.fillWidth: true
                        text: modelData.feed
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        elide: Text.ElideRight
                    }
                }
            }
            Text {
                visible: text.length > 0
                Layout.fillWidth: true
                text: tc.layoutNote
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.WordWrap
            }
            Flow {
                Layout.fillWidth: true
                spacing: Theme.spacing
                AppCheckBox { text: qsTr("Keep the crossover, align only"); checked: tc.keepCrossover; enabled: tab.idle; onToggled: tc.keepCrossover = checked }
                AppCheckBox { text: qsTr("Keep the delays"); checked: tc.keepDelays; enabled: tab.idle; onToggled: tc.keepDelays = checked }
                AppCheckBox { text: qsTr("Keep the levels"); checked: tc.keepLevels; enabled: tab.idle; onToggled: tc.keepLevels = checked }
                AppCheckBox { text: qsTr("No EQ fit"); checked: tc.keepEq; enabled: tab.idle; onToggled: tc.keepEq = checked }
            }
            RowLayout {
                spacing: Theme.spacingSmall
                AppButton {
                    text: qsTr("Propose from the Device")
                    enabled: tab.idle
                    onClicked: tc.reloadLayout()
                }
                AppButton {
                    objectName: "MeasureTune.propose"
                    text: qsTr("Propose a Tune")
                    primary: true
                    enabled: tab.idle && mc.hasResult
                    onClicked: tc.propose()
                }
            }
        }

        // 2. The proposal
        Card {
            title: qsTr("2. Proposal")
            Layout.fillWidth: true
            visible: tc.hasProposal
            Text {
                visible: !tc.timingReliable
                Layout.fillWidth: true
                text: qsTr("Timing unreliable: the measurement's timing is not trusted, so no delay, polarity or crossover change is offered.")
                font: Theme.fontBody
                color: Theme.warning
                wrapMode: Text.WordWrap
            }
            Repeater {
                model: tc.warnings
                Text {
                    required property string modelData
                    Layout.fillWidth: true
                    text: qsTr("Warning: %1").arg(modelData)
                    font: Theme.fontBody
                    color: Theme.warning
                    wrapMode: Text.WordWrap
                }
            }
            Text {
                text: tc.writes.length ? qsTr("Changes (%1):").arg(tc.writes.length) : qsTr("No change proposed.")
                font.pixelSize: Theme.fontBody.pixelSize
                font.weight: Font.DemiBold
                color: Theme.text
            }
            Repeater {
                objectName: "MeasureTune.writes"
                model: tc.writes
                Text {
                    required property var modelData
                    Layout.fillWidth: true
                    text: "• " + modelData.words + (modelData.timing ? qsTr(" (a timing change: after the summed check)") : "")
                    font: Theme.fontBody
                    color: Theme.text
                    wrapMode: Text.WordWrap
                }
            }
            Text {
                visible: tc.held.length > 0
                text: qsTr("Held to:")
                font.pixelSize: Theme.fontBody.pixelSize
                font.weight: Font.DemiBold
                color: Theme.text
            }
            Repeater {
                model: tc.held
                Text {
                    required property string modelData
                    Layout.fillWidth: true
                    text: "• " + modelData
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.WordWrap
                }
            }
            Text {
                visible: tc.withdrawn.length > 0
                text: qsTr("Withdrawn:")
                font.pixelSize: Theme.fontBody.pixelSize
                font.weight: Font.DemiBold
                color: Theme.text
            }
            Repeater {
                objectName: "MeasureTune.withdrawn"
                model: tc.withdrawn
                Text {
                    required property string modelData
                    Layout.fillWidth: true
                    text: "• " + modelData
                    font: Theme.fontBody
                    color: Theme.warning
                    wrapMode: Text.WordWrap
                }
            }
            Repeater {
                model: tc.notes
                Text {
                    required property string modelData
                    Layout.fillWidth: true
                    text: modelData
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.WordWrap
                }
            }
            MeasureGraph {
                objectName: "MeasureTune.graph"
                title: qsTr("Before and predicted")
                Layout.fillWidth: true
                Layout.preferredHeight: 260
                graph: tc.graph
            }
            Text {
                objectName: "MeasureTune.sumCheck"
                visible: text.length > 0
                Layout.fillWidth: true
                text: tc.sumCheckText
                font: Theme.fontBody
                color: tc.sumCheck === "failed" ? Theme.warning : tc.sumCheck === "passed" ? Theme.success : Theme.text
                wrapMode: Text.WordWrap
            }
            RowLayout {
                visible: tab.beforeApply
                spacing: Theme.spacingSmall
                AppButton {
                    objectName: "MeasureTune.sumCheckRun"
                    visible: tc.needsSumCheck
                    text: qsTr("Run the Summed Check")
                    primary: true
                    enabled: tab.idle
                    onClicked: tc.sumCheckRun()
                }
                AppButton {
                    objectName: "MeasureTune.apply"
                    text: qsTr("Apply with a Way Back…")
                    primary: !tc.needsSumCheck
                    enabled: tab.idle && tc.writes.length > 0
                    onClicked: applyQuestion.open()
                }
                AppButton {
                    text: qsTr("Forget It")
                    enabled: tab.idle
                    onClicked: tc.stop()
                }
            }
        }

        // 3. Applied: the guard, the verify, Keep or Revert
        Card {
            title: qsTr("3. Applied")
            Layout.fillWidth: true
            visible: tc.applied || tc.stage === "kept" || tc.stage === "reverted" || tc.stage === "revertIncomplete" // -- WP REVERTW --
            Text {
                objectName: "MeasureTune.guard"
                visible: tc.applied
                Layout.fillWidth: true
                text: tc.guardText
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.WordWrap
            }
            Text {
                visible: tc.running
                text: tc.runText
                font: Theme.fontBody
                color: Theme.text
            }
            ProgressBar {
                visible: tc.running
                Layout.fillWidth: true
                from: 0
                to: 100
                value: tc.progress
            }
            Text {
                objectName: "MeasureTune.verify"
                visible: text.length > 0
                Layout.fillWidth: true
                text: tc.verifyText
                font: Theme.fontBody
                color: tc.verifyPassed ? Theme.success : Theme.warning
                wrapMode: Text.WordWrap
            }
            RowLayout {
                visible: tc.applied
                spacing: Theme.spacingSmall
                AppButton {
                    objectName: "MeasureTune.verifyRun"
                    text: qsTr("Verify")
                    enabled: !tc.running && !mc.running && !tc.busy
                    onClicked: tc.verify()
                }
                AppButton {
                    objectName: "MeasureTune.keep"
                    text: qsTr("Keep")
                    primary: tc.verifyPassed
                    enabled: !tc.busy && !tc.keepConfirmNeeded
                    onClicked: tc.keep()
                }
                AppButton {
                    objectName: "MeasureTune.revert"
                    text: qsTr("Revert")
                    destructive: true
                    enabled: !tc.busy
                    onClicked: tc.revert()
                }
            }
            Text {
                objectName: "MeasureTune.kept"
                visible: tc.stage === "kept" && text.length > 0
                Layout.fillWidth: true
                text: tc.keptText
                font: Theme.fontBody
                color: Theme.success
                wrapMode: Text.WordWrap
            }
            RowLayout {
                visible: tc.stage === "kept"
                spacing: Theme.spacingSmall
                AppButton {
                    objectName: "MeasureTune.abStart"
                    visible: !tc.abActive
                    text: qsTr("Compare Before and After (A/B)")
                    enabled: !tc.busy
                    onClicked: tc.abStart()
                }
                AppButton {
                    objectName: "MeasureTune.abToggle"
                    visible: tc.abActive
                    text: qsTr("Switch A/B")
                    enabled: !tc.busy
                    onClicked: tc.abToggle()
                }
                AppButton {
                    visible: tc.abActive
                    text: qsTr("Keep the Tune (A)")
                    enabled: !tc.busy
                    onClicked: tc.abKeep("a")
                }
                AppButton {
                    visible: tc.abActive
                    text: qsTr("Go Back (B)")
                    enabled: !tc.busy
                    onClicked: tc.abKeep("b")
                }
                Text {
                    text: tc.abText
                    font: Theme.fontBody
                    color: Theme.text
                }
            }
        }
    }

    QuestionDialog {
        id: applyQuestion
        objectName: "MeasureTune.applyQuestion"
        ofDevice: true   // -- WP MDQT2c --
        parent: Overlay.overlay
        title: qsTr("Apply the auto-tune?")
        text: qsTr("Every change above is written as one undo step, after a snapshot \"Before auto-tune\" is filed and the checks are run again. It goes back by itself unless you Keep it.")
        acceptText: qsTr("Apply")
        onAnswered: answer => { if (answer === "accept") TuneController.apply(0) }
    }

    // The second Keep after a verify that did not pass: the person's own
    // answer, never pressed for them (the core takes the second Keep from a
    // second after the first; TuneController.confirmKeep waits for that)
    QuestionDialog {
        id: keepQuestion
        objectName: "MeasureTune.keepQuestion"
        ofDevice: true   // -- WP MDQT2c --
        parent: Overlay.overlay
        title: qsTr("Keep a tune the verify did not confirm?")
        text: TuneController.keepConfirmText
        acceptText: qsTr("Keep Anyway")
        rejectText: qsTr("Not Now")
        destructive: true
        onAnswered: answer => {
            if (answer === "accept") TuneController.confirmKeep()
            else TuneController.declineKeep()
        }
    }
}
