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
// -- WP UA2 -- (A2) The actions are the panel's action bar's (MeasureActionBar:
// Run the Summed Check, Apply…, Forget It; Verify, Revert, Keep with the
// countdown), always in view. The proposal reads graph, changes, then its
// details folded; nothing above it comes or goes with the stage (the layout
// stays, disabled after Apply; the proposal stays, dimmed, while the summed
// check plays), so the page keeps the reader's place.
//
// Keep after a verify that did not pass: the core refuses it once
// (`measure.tune.keep_unverified`) and this tab asks again, in a question of
// its own; only "Keep Anyway", pressed by the person, keeps the tune.
ScrollView {
    id: tab
    objectName: "MeasureTune.page"   // -- WP UA2 --
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

    // -- WP DL2 -- Modern: the steps as dots over sections (a label, no box,
    // no number), one notice for what is missing, the state of each line in
    // an icon (never orange text), and no proposal while the summed check
    // plays (the one shown may be an earlier one; its result reads it again).
    // Classic keeps its numbered cards.
    readonly property bool modern: LayoutController.modern
    readonly property bool afterApply: tc.applied || tc.stage === "kept" || tc.stage === "reverted" || tc.stage === "revertIncomplete" // -- WP REVERTW --
    readonly property bool checking: tc.stage === "sumCheck"
    readonly property int step: afterApply ? 2 : tc.hasProposal || checking ? 1 : 0
    component Section: Card {
        color: tab.modern ? "transparent" : Theme.panel
        padding: tab.modern ? 0 : Theme.spacing
    }
    component StatusLine: RowLayout {
        id: line
        property string kind: "info"   // "ok", "warning", "error", "info"
        property string text
        property font font: Theme.fontBody
        property color classicColor: Theme.text
        spacing: 6
        Icon {
            visible: tab.modern && line.kind !== "plain"
            Layout.alignment: Qt.AlignTop
            name: line.kind === "ok" ? "circle-check" : line.kind === "error" ? "circle-alert" : line.kind === "warning" ? "triangle-alert" : "info"
            color: line.kind === "ok" ? Theme.success : line.kind === "error" ? Theme.dangerText : line.kind === "warning" ? Theme.warning : Theme.textMuted
        }
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: line.text
            font: line.font
            color: tab.modern ? Theme.text : line.classicColor
            wrapMode: Text.WordWrap
        }
    }

    Connections {
        target: TuneController
        function onKeepConfirmNeededChanged() {
            if (TuneController.keepConfirmNeeded) keepQuestion.open()
        }
    }

    ContentColumn {
        width: tab.modern ? Math.min(tab.availableWidth, maximumWidth) : tab.availableWidth
        spacing: tab.modern ? Theme.spacingSection : Theme.spacing

        StepDots {
            objectName: "MeasureTune.steps"
            visible: tab.modern
            Layout.preferredWidth: Math.min(360, tab.availableWidth)
            steps: [qsTr("Layout"), qsTr("Proposal"), qsTr("Applied")]
            at: tab.step
        }
        // -- WP DL2 -- Modern: the one notice while there is nothing to tune from
        NoticeRow {
            objectName: "MeasureTune.noMeasurement"
            visible: tab.modern && !mc.hasResult && tab.beforeApply && !tc.hasProposal && !tab.checking
            Layout.fillWidth: true
            text: qsTr("No measurement yet: measure on the Measure tab, or open one on Results.")
        }
        Text {
            objectName: "MeasureTune.stage"
            // -- WP DL2 -- Modern: "No tune yet" says what the notice and the dots say
            // -- WP UA2 -- Modern: the action bar says the stage, the Applied
            // section how it ended (nothing above the proposal comes and goes)
            visible: !tab.modern
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: tc.stageText
            font: tab.modern ? Theme.fontBody : Theme.fontTitle
            // -- WP REVERTW -- a revert not fully successful is an error
            color: tc.stage === "revertIncomplete" ? Theme.dangerText : Theme.text
            wrapMode: Text.WordWrap
        }
        Text {
            objectName: tab.modern ? "" : "MeasureTune.error"
            visible: text.length > 0 && !tab.modern
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: tc.error
            font: Theme.fontBody
            color: Theme.dangerText
            wrapMode: Text.WordWrap
        }
        Text {
            visible: text.length > 0 && !tab.modern
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: tc.issueText
            font: Theme.fontBody
            color: Theme.dangerText
            wrapMode: Text.WordWrap
        }
        NoticeRow {
            objectName: tab.modern ? "MeasureTune.error" : ""
            visible: text.length > 0 && tab.modern
            Layout.fillWidth: true
            kind: "error"
            text: tc.error.length ? tc.error : tc.issueText
        }
        // 1. The layout
        Section {
            title: tab.modern ? qsTr("Layout") : qsTr("1. Layout")
            Layout.fillWidth: true
            // -- WP UA2 -- stays after Apply (disabled): the proposal under it keeps its place
            enabled: tab.beforeApply
            Text {
                visible: !tab.modern || mc.hasResult   // -- WP DL2 -- the notice above says it
                Layout.fillWidth: true
                textFormat: Text.PlainText
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
                        textFormat: Text.PlainText
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
                textFormat: Text.PlainText
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
                    // -- WP DL2 -- no disabled primary, and the proposal's action is the next one
                    primary: !tab.modern || (mc.hasResult && !tc.hasProposal)
                    enabled: tab.idle && mc.hasResult
                    onClicked: tc.propose()
                }
            }
        }

        // 2. The proposal
        // -- WP UA2 -- (A2) graph, the changes, then the details folded; shown
        // dimmed while the summed check plays (its result reads it again), never
        // taken away, so the page does not jump; the actions are the bar's
        Section {
            objectName: "MeasureTune.proposal"
            title: tab.modern ? qsTr("Proposal") : qsTr("2. Proposal")
            Layout.fillWidth: true
            visible: tc.hasProposal
            enabled: !tab.checking
            opacity: tab.checking ? 0.5 : 1
            MeasureGraph {
                objectName: "MeasureTune.graph"
                title: qsTr("Before and predicted")
                Layout.fillWidth: true
                Layout.preferredHeight: 260
                graph: tc.graph
            }
            Text {
                textFormat: Text.PlainText
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
                    textFormat: Text.PlainText
                    text: "• " + modelData.words + (modelData.timing ? qsTr(" (a timing change: after the summed check)") : "")
                    font: Theme.fontBody
                    color: Theme.text
                    wrapMode: Text.WordWrap
                }
            }
            StatusLine {
                objectName: "MeasureTune.sumCheck"
                visible: text.length > 0
                Layout.fillWidth: true
                kind: tc.sumCheck === "failed" ? "warning" : tc.sumCheck === "passed" ? "ok" : "info"
                text: tc.sumCheckText
                classicColor: tc.sumCheck === "failed" ? Theme.warning : tc.sumCheck === "passed" ? Theme.success : Theme.text
            }
            StatusLine {
                visible: !tc.timingReliable
                Layout.fillWidth: true
                kind: "warning"
                text: qsTr("Timing unreliable: the measurement's timing is not trusted, so no delay, polarity or crossover change is offered.")
                classicColor: Theme.warning
            }
            Repeater {
                model: tc.warnings
                StatusLine {
                    required property string modelData
                    Layout.fillWidth: true
                    kind: "warning"
                    // -- WP DL2 -- Modern: the icon says it is a warning
                    text: tab.modern ? modelData : qsTr("Warning: %1").arg(modelData)
                    classicColor: Theme.warning
                }
            }
            // -- WP UA2 -- what the tune was held to, what it withdrew and its
            // notes: folded, the counts at the heading's right
            InspectorSection {
                id: details
                objectName: "MeasureTune.details"
                visible: tc.held.length + tc.withdrawn.length + tc.notes.length > 0
                Layout.fillWidth: true
                title: qsTr("Details")
                expanded: false
                summary: [tc.held.length ? qsTr("%n limit(s)", "", tc.held.length) : "",
                          tc.withdrawn.length ? qsTr("%n withdrawn", "", tc.withdrawn.length) : "",
                          tc.notes.length ? qsTr("%n note(s)", "", tc.notes.length) : ""].filter(t => t.length).join(" · ")
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
                        textFormat: Text.PlainText
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
                    StatusLine {
                        required property string modelData
                        Layout.fillWidth: true
                        kind: "warning"
                        text: tab.modern ? modelData : "• " + modelData
                        classicColor: Theme.warning
                    }
                }
                Repeater {
                    model: tc.notes
                    Text {
                        required property string modelData
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        text: modelData
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.WordWrap
                    }
                }
            }
        }

        // 3. Applied: the guard, the verify, Keep or Revert
        Section {
            title: tab.modern ? qsTr("Applied") : qsTr("3. Applied")
            Layout.fillWidth: true
            visible: tab.afterApply
            // -- WP UA2 -- (A2) the countdown, the verify's run and Verify, Keep and
            // Revert are the action bar's; here what the verify found, and in
            // Modern the stage in words (Classic says it at the top)
            StatusLine {
                objectName: "MeasureTune.stageLine"
                visible: tab.modern && tc.stage !== "kept"
                Layout.fillWidth: true
                kind: tc.stage === "revertIncomplete" ? "error" : "info"
                text: tc.stageText
            }
            StatusLine {
                objectName: "MeasureTune.verify"
                visible: text.length > 0
                Layout.fillWidth: true
                kind: tc.verifyPassed ? "ok" : "warning"
                text: tc.verifyText
                classicColor: tc.verifyPassed ? Theme.success : Theme.warning
            }
            StatusLine {
                objectName: "MeasureTune.kept"
                visible: tc.stage === "kept" && text.length > 0
                Layout.fillWidth: true
                kind: "ok"
                text: tc.keptText
                classicColor: Theme.success
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
                    textFormat: Text.PlainText
                    text: tc.abText
                    font: Theme.fontBody
                    color: Theme.text
                }
            }
        }
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
