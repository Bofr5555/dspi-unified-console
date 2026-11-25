import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP UA2 -- (A2, QT-MEASUREBAR) Measure's action bar: the job under way in
// one line above every tab, with its actions, so they never sit below the
// fold. In order of what matters most:
// - "trial": an applied auto-tune goes back by itself unless kept: the
//   countdown, Verify (or the verify's progress and Stop), Revert, Keep
//   (primary, unless a verify ran and did not confirm the tune);
// - "run": what plays or listens (a measurement, the summed check, the live
//   view, the passive RTA, an agent's run) with its step, its progress and
//   Stop; Next Position while a walked run waits for the microphone;
// - "proposal" (the Auto-tune tab only): Forget, Apply…, and the summed check
//   when the proposal needs it, the next step primary.
// The buttons keep the names the tabs gave them (MeasureTune.keep, …).
Rectangle {
    id: bar
    objectName: "Measure.actionBar"
    // The Auto-tune tab shows: a proposal's actions are offered
    property bool onTune: false

    readonly property var mc: MeasureController
    readonly property var tc: TuneController
    readonly property bool modern: LayoutController.modern
    readonly property bool idle: !mc.running && !tc.busy
    readonly property bool proposalOpen: tc.hasProposal && !tc.applied && tc.stage !== "kept" && tc.stage !== "sumCheck"
    readonly property string job: tc.applied ? "trial" : mc.running ? "run" : onTune && proposalOpen ? "proposal" : ""
    // A run's progress: the tune's runs, else a measurement's; -1 none (the live view, the passive RTA)
    readonly property int progress: tc.running ? tc.progress : mc.measuring ? mc.progress : -1
    readonly property string on: mc.runningOn

    function clock(s) {
        const n = Math.max(0, s)
        return Math.floor(n / 60) + ":" + String(n % 60).padStart(2, "0")
    }

    visible: job.length > 0
    implicitHeight: Math.max(Theme.controlHeight + 2 * 6, row.implicitHeight + 2 * 6)
    radius: modern ? Theme.radiusControl : Theme.radius
    color: modern ? Theme.bannerFill("info") : Theme.selection
    // Modern's status strip: an edge, no outline (DESIGN-LANGUAGE §14)
    Rectangle {
        visible: bar.modern
        width: 3
        height: parent.height
        radius: 1.5
        color: Theme.bannerEdge("info")
    }

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.leftMargin: Theme.spacing
        anchors.rightMargin: 6
        anchors.topMargin: 6
        anchors.bottomMargin: 6
        spacing: Theme.spacingSmall

        Icon {
            visible: bar.modern
            Layout.alignment: Qt.AlignVCenter
            name: bar.job === "trial" ? "timer" : bar.job === "proposal" ? "sliders-horizontal" : "audio-lines"
            color: Theme.bannerEdge("info")
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 4
            Text {
                objectName: "Measure.runningText"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: bar.job === "trial"
                      ? (tc.guardRemainingS > 0
                         //: The action bar while an applied auto-tune waits: %1 the device's name, %2 the time left ("3:02")
                         ? (bar.on.length ? qsTr("Trying the auto-tune on %1 · goes back by itself in %2").arg(bar.on).arg(bar.clock(tc.guardRemainingS))
                                          //: The action bar while an applied auto-tune waits: %1 the time left ("3:02")
                                          : qsTr("Trying the auto-tune · goes back by itself in %1").arg(bar.clock(tc.guardRemainingS)))
                         : tc.guardText)
                      : bar.job === "proposal"
                      ? (tc.writes.length ? qsTr("Proposed: %n change(s), nothing written yet.", "", tc.writes.length)
                                          : qsTr("Proposed: no change."))
                      : bar.on.length ? qsTr("Running on %1. %2").arg(bar.on).arg(mc.runningText) : mc.runningText
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.WordWrap
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }
            // The step under way: the tune's run, a measurement's phase
            Text {
                objectName: "Measure.actionStep"
                visible: text.length > 0
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: tc.running ? (tc.stageText + " " + tc.runText)
                      : bar.job === "run" && mc.measuring ? mc.phaseText
                      // The verify's verdict, in short (the Auto-tune tab has it whole)
                      : bar.job === "trial" && tc.verifyDone ? (tc.verifyPassed ? qsTr("The verify confirmed the tune.")
                                                                                 : qsTr("The verify did not confirm the tune."))
                      : ""
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.WordWrap
            }
            Rectangle {
                objectName: "Measure.progress"
                visible: bar.progress >= 0 && (bar.job === "run" || tc.running)
                Layout.fillWidth: true
                implicitHeight: 4
                radius: 2
                color: Theme.control
                Rectangle {
                    width: parent.width * Math.min(100, Math.max(0, bar.progress)) / 100
                    height: parent.height
                    radius: 2
                    color: Theme.accent
                }
                Accessible.role: Accessible.ProgressBar
                Accessible.name: qsTr("Progress")
                Accessible.description: qsTr("%1 %").arg(Math.max(0, bar.progress))
            }
        }

        // ---- What plays or listens: its Stop (also the verify's) ----
        RowLayout {
            objectName: "Measure.stopBar"
            visible: bar.job === "run" || (bar.job === "trial" && tc.running)
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "MeasureSetup.next"
                visible: bar.job === "run" && mc.waitingPosition
                text: qsTr("Next Position")
                primary: true
                onClicked: mc.nextPosition()
            }
            AppButton {
                objectName: "Measure.stop"
                text: qsTr("Stop")
                destructive: !bar.modern
                onClicked: mc.stop()
            }
        }

        // ---- A proposal: Forget, Apply…, the summed check ----
        RowLayout {
            visible: bar.job === "proposal"
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "MeasureTune.forget"
                text: qsTr("Forget It")
                enabled: bar.idle
                onClicked: tc.stop()
            }
            AppButton {
                objectName: "MeasureTune.apply"
                text: qsTr("Apply…")
                primary: !tc.needsSumCheck
                enabled: bar.idle && tc.writes.length > 0
                onClicked: applyQuestion.open()
            }
            AppButton {
                objectName: "MeasureTune.sumCheckRun"
                visible: tc.needsSumCheck
                text: qsTr("Run the Summed Check")
                primary: true
                enabled: bar.idle
                onClicked: tc.sumCheckRun()
            }
        }

        // ---- The trial: Verify, Revert, Keep ----
        RowLayout {
            visible: bar.job === "trial"
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "MeasureTune.verifyRun"
                visible: !tc.running
                text: qsTr("Verify")
                enabled: !tc.running && !mc.running && !tc.busy
                onClicked: tc.verify()
            }
            AppButton {
                objectName: "MeasureTune.revert"
                text: qsTr("Revert")
                enabled: !tc.busy
                onClicked: tc.revert()
            }
            AppButton {
                objectName: "MeasureTune.keep"
                text: qsTr("Keep")
                // The next step, unless a verify ran and did not confirm the tune
                primary: !tc.verifyDone || tc.verifyPassed
                enabled: !tc.busy && !tc.keepConfirmNeeded
                onClicked: tc.keep()
            }
        }
    }

    QuestionDialog {
        id: applyQuestion
        objectName: "MeasureTune.applyQuestion"
        ofDevice: true   // -- WP MDQT2c --
        parent: Overlay.overlay
        title: qsTr("Apply the auto-tune?")
        text: qsTr("Every change is written as one undo step, after a snapshot \"Before auto-tune\" is filed and the checks are run again. It goes back by itself unless you Keep it.")
        acceptText: qsTr("Apply")
        onAnswered: answer => { if (answer === "accept") TuneController.apply(0) }
    }
}
