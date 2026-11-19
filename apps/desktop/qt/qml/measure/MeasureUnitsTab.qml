import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP ATQT -- Tools > Measure…'s "Several units" tab (PLAN-AT 1.4 to 1.6;
// the web's ATWEB step): an alignment group across units of our hub,
// measured with one microphone, each unit's own generator started on its
// sample of the units' shared clock. The placement first (nothing plays
// before the person chose the listening position), the units and their
// outputs, the check (every unit judged by its own connection), the run with
// Stop always in view and the settle wait in words, the result, the proposal,
// the summed check, and for Admin the apply with the hub's guard, the verify,
// Keep (asked twice after a verify that did not pass) and Revert. Control
// measures and sees; it never gets Apply or Revert.
Item {
    id: tab
    objectName: "MeasureUnits"
    readonly property var uc: UnitsMeasureController
    readonly property bool busy: uc.busy.length > 0 || uc.running
    readonly property var placements: [
        { id: "", text: qsTr("Choose…") },
        { id: "listeningPosition", text: qsTr("Listening position (speakers)") },
        { id: "multiPosition", text: qsTr("Several positions") },
        { id: "nearField", text: qsTr("Near-field") },
        { id: "headphones", text: qsTr("Headphones on a coupler") }
    ]
    readonly property var roles: [
        { id: "full", text: qsTr("Full range") }, { id: "sub", text: qsTr("Subwoofer") },
        { id: "low", text: qsTr("Low (woofer)") }, { id: "mid", text: qsTr("Mid") },
        { id: "high", text: qsTr("High (tweeter)") }
    ]
    function indexOf(list, id) {
        for (let i = 0; i < list.length; ++i) if (list[i].id === id) return i
        return 0
    }
    function notice(key) { return NoticeCenter.row(key) }

    // Shown: the hub's family, its units; hidden or the window closed:
    // whatever this tab started is stopped and the units' connections close
    onVisibleChanged: visible ? uc.enter() : uc.leave()
    Component.onCompleted: if (visible) uc.enter()
    Connections {
        target: tab.Window.window
        function onClosing() { UnitsMeasureController.leave() }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: Theme.spacingSmall

        // Stop, always in view while anything plays; the phase in words
        Rectangle {
            objectName: "MeasureUnits.runBar"
            visible: tab.uc.running
            Layout.fillWidth: true
            implicitHeight: runRow.implicitHeight + 12
            radius: Theme.radius
            color: Theme.selection
            Accessible.role: Accessible.StatusBar
            Accessible.name: qsTr("Measuring across units")
            RowLayout {
                id: runRow
                anchors.fill: parent
                anchors.margins: 6
                Text {
                    objectName: "MeasureUnits.phase"
                    Layout.fillWidth: true
                    text: tab.uc.phaseText
                    font: Theme.fontBody
                    color: Theme.text
                    wrapMode: Text.WordWrap
                }
                // -- review AT-D2 -- the demo hub's simulated microphone is the run's
                Text {
                    objectName: "MeasureUnits.simOn"
                    visible: tab.uc.simOn
                    text: tab.uc.simLabel
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
                AppButton {
                    objectName: "MeasureUnits.stop"
                    text: qsTr("Stop")
                    destructive: true
                    onClicked: tab.uc.stop()
                }
            }
        }

        // Not on our hub
        Text {
            objectName: "MeasureUnits.unavailable"
            visible: tab.uc.unavailable.length > 0
            Layout.fillWidth: true
            property string key: tab.uc.unavailableKey
            text: {
                const r = tab.notice(key)
                return r && r.title ? r.title + ". " + tab.uc.unavailable : tab.uc.unavailable
            }
            font: Theme.fontBody
            color: Theme.warning
            wrapMode: Text.WordWrap
            Accessible.role: Accessible.StaticText
        }

        ScrollView {
            id: scroll
            objectName: "MeasureUnits.scroll"
            visible: tab.uc.unavailable.length === 0
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            contentWidth: availableWidth

            ColumnLayout {
                width: scroll.availableWidth
                spacing: Theme.spacing

                Text {
                    Layout.fillWidth: true
                    text: qsTr("Every unit plays its own generator, each start on its sample of the units' shared clock, and one microphone hears them all. The hub stops every unit if anything changes.")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.WordWrap
                }

                Card {
                    title: qsTr("Where is the microphone?")
                    Layout.fillWidth: true
                    AppComboBox {
                        objectName: "MeasureUnits.placement"
                        label: qsTr("Placement across units")
                        Layout.preferredWidth: 300
                        enabled: !tab.busy
                        model: tab.placements
                        textRole: "text"
                        currentIndex: tab.indexOf(tab.placements, tab.uc.placement)
                        onActivated: index => tab.uc.placement = tab.placements[index].id
                    }
                    Text {
                        objectName: "MeasureUnits.placementProblem"
                        property string key: "measure_system_placement"
                        visible: text.length > 0
                        Layout.fillWidth: true
                        text: tab.uc.placementProblem
                        font: Theme.fontSmall
                        color: Theme.warning
                        wrapMode: Text.WordWrap
                    }
                }

                // -- review AT-D2 -- the demo hub's simulated microphone (WP ATDEMO), where the hub has one
                AppCheckBox {
                    objectName: "MeasureUnits.sim"
                    visible: tab.uc.simAvailable
                    Layout.fillWidth: true
                    text: qsTr("Use the simulated microphone (the demo hub's room hears its demo units; nothing plays anywhere)")
                    checked: tab.uc.useSim
                    enabled: !tab.busy
                    Accessible.name: qsTr("Simulated microphone across units")
                    onToggled: tab.uc.useSim = checked
                }

                Card {
                    title: qsTr("Units of the group")
                    Layout.fillWidth: true
                    Text {
                        visible: tab.uc.units.length === 0
                        Layout.fillWidth: true
                        text: qsTr("The hub shares no DSPi with this console.")
                        font: Theme.fontBody
                        color: Theme.textMuted
                    }
                    Repeater {
                        model: tab.uc.units
                        delegate: ColumnLayout {
                            id: unitBox
                            required property var modelData
                            objectName: "MeasureUnits.unit." + modelData.serial
                            Layout.fillWidth: true
                            spacing: 2
                            AppCheckBox {
                                objectName: "MeasureUnits.choose." + unitBox.modelData.serial
                                text: unitBox.modelData.name
                                      + (unitBox.modelData.name !== unitBox.modelData.serial ? " (" + unitBox.modelData.serial + ")" : "")
                                      + (unitBox.modelData.own ? qsTr(" (this DSPi)") : "")
                                      + (!unitBox.modelData.online ? qsTr(" (offline)") : "")
                                checked: unitBox.modelData.chosen
                                enabled: !tab.busy && unitBox.modelData.online
                                Accessible.name: qsTr("Measure %1").arg(unitBox.modelData.serial)
                                onToggled: tab.uc.setUnitChosen(unitBox.modelData.serial, checked)
                            }
                            Text {
                                visible: unitBox.modelData.opening
                                Layout.leftMargin: 24
                                text: qsTr("Opening %1…").arg(unitBox.modelData.serial)
                                font: Theme.fontSmall
                                color: Theme.textMuted
                            }
                            Text {
                                objectName: "MeasureUnits.unitError." + unitBox.modelData.serial
                                visible: unitBox.modelData.chosen && unitBox.modelData.error.length > 0
                                Layout.fillWidth: true
                                Layout.leftMargin: 24
                                text: unitBox.modelData.error
                                font: Theme.fontSmall
                                color: Theme.dangerText
                                wrapMode: Text.WordWrap
                            }
                            Flow {
                                visible: unitBox.modelData.chosen
                                Layout.fillWidth: true
                                Layout.leftMargin: 24
                                spacing: Theme.spacingSmall
                                Repeater {
                                    model: unitBox.modelData.outputs
                                    delegate: RowLayout {
                                        id: outRow
                                        required property var modelData
                                        spacing: 4
                                        AppCheckBox {
                                            objectName: "MeasureUnits.output." + unitBox.modelData.serial + "." + outRow.modelData.output
                                            text: outRow.modelData.label
                                            checked: outRow.modelData.on
                                            enabled: !tab.busy
                                            Accessible.name: qsTr("%1 output %2").arg(unitBox.modelData.serial).arg(outRow.modelData.output + 1)
                                            onToggled: tab.uc.setOutputOn(unitBox.modelData.serial, outRow.modelData.output, checked)
                                        }
                                        AppComboBox {
                                            visible: outRow.modelData.on
                                            label: qsTr("Role of %1 output %2").arg(unitBox.modelData.serial).arg(outRow.modelData.output + 1)
                                            Layout.preferredWidth: 150
                                            enabled: !tab.busy
                                            model: tab.roles
                                            textRole: "text"
                                            currentIndex: tab.indexOf(tab.roles, outRow.modelData.role)
                                            onActivated: index => tab.uc.setOutputRole(unitBox.modelData.serial, outRow.modelData.output, tab.roles[index].id)
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                Card {
                    title: qsTr("The runs")
                    Layout.fillWidth: true
                    Flow {
                        Layout.fillWidth: true
                        spacing: Theme.spacing
                        AppSpinBox {
                            objectName: "MeasureUnits.sweep"
                            label: qsTr("Sweep length across units")
                            enabled: !tab.busy
                            decimals: 1
                            unit: qsTr("s")
                            realFrom: 1
                            realTo: 15
                            realStep: 0.5
                            realValue: tab.uc.sweepMs / 1000
                            onCommitted: value => tab.uc.sweepMs = Math.round(value * 1000)
                        }
                        AppComboBox {
                            objectName: "MeasureUnits.passes"
                            label: qsTr("Passes across units")
                            Layout.preferredWidth: 260
                            enabled: !tab.busy
                            readonly property var ids: [0, 2, 3, 4, 5, 6, 7, 8]
                            model: [qsTr("The plan's (5, or 3 for a long group)"), "2", "3", "4", "5", "6", "7", "8"]
                            currentIndex: Math.max(0, ids.indexOf(tab.uc.passes))
                            onActivated: index => tab.uc.passes = ids[index]
                        }
                        AppComboBox {
                            objectName: "MeasureUnits.anchor"
                            label: qsTr("Anchor output")
                            Layout.preferredWidth: 260
                            enabled: !tab.busy
                            model: tab.uc.anchors
                            textRole: "label"
                            currentIndex: tab.indexOf(tab.uc.anchors, tab.uc.anchor)
                            onActivated: index => tab.uc.anchor = tab.uc.anchors[index].id
                        }
                        BorderedField {
                            objectName: "MeasureUnits.name"
                            label: qsTr("Name of the measurements")
                            Accessible.name: label
                            placeholderText: qsTr("Name (optional)")
                            implicitWidth: 220
                            enabled: !tab.busy
                            text: tab.uc.runName
                            onCommitted: t => tab.uc.runName = t
                        }
                    }
                    RowLayout {
                        spacing: Theme.spacingSmall
                        AppButton {
                            objectName: "MeasureUnits.check"
                            text: qsTr("Check every unit")
                            enabled: !tab.busy && tab.uc.blocker.length === 0
                            toolTip: tab.uc.blocker
                            onClicked: tab.uc.check()
                        }
                        AppButton {
                            objectName: "MeasureUnits.start"
                            text: qsTr("Measure the units")
                            primary: true
                            enabled: !tab.busy && tab.uc.blocker.length === 0
                            toolTip: tab.uc.blocker
                            onClicked: tab.uc.start()
                        }
                        Text {
                            visible: tab.uc.busy.length > 0
                            text: qsTr("%1…").arg(tab.uc.busy)
                            font: Theme.fontSmall
                            color: Theme.textMuted
                        }
                    }
                    Text {
                        objectName: "MeasureUnits.blocker"
                        visible: text.length > 0 && tab.uc.placementProblem.length === 0
                        Layout.fillWidth: true
                        text: tab.uc.blocker
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.WordWrap
                    }
                    Repeater {
                        model: tab.uc.preflightLines
                        delegate: Text {
                            required property var modelData
                            objectName: "MeasureUnits.preflight." + (modelData.key || "ready") + "." + (modelData.serial || "")
                            Layout.fillWidth: true
                            text: modelData.text
                            font: Theme.fontSmall
                            color: modelData.kind === "blocker" ? Theme.dangerText : modelData.kind === "ready" ? Theme.success : Theme.textMuted
                            wrapMode: Text.WordWrap
                        }
                    }
                    Text {
                        objectName: "MeasureUnits.runs"
                        property string key: "measure_system_runs"
                        visible: text.length > 0
                        Layout.fillWidth: true
                        text: tab.uc.runsText
                        font: Theme.fontSmall
                        color: Theme.text
                        wrapMode: Text.WordWrap
                    }
                }

                Text {
                    objectName: "MeasureUnits.error"
                    property string key: tab.uc.errorKey
                    visible: text.length > 0
                    Layout.fillWidth: true
                    text: tab.uc.error
                    font: Theme.fontBody
                    color: Theme.dangerText
                    wrapMode: Text.WordWrap
                    Accessible.role: Accessible.AlertMessage
                }
                Text {
                    objectName: "MeasureUnits.said"
                    visible: text.length > 0 && tab.uc.error.length === 0
                    Layout.fillWidth: true
                    text: tab.uc.said
                    font: Theme.fontBody
                    color: Theme.text
                    wrapMode: Text.WordWrap
                }
                Text {
                    objectName: "MeasureUnits.issue"
                    property string key: tab.uc.issueKey
                    visible: text.length > 0 && !tab.uc.running
                    Layout.fillWidth: true
                    text: tab.uc.issueText
                    font: Theme.fontBody
                    color: Theme.dangerText
                    wrapMode: Text.WordWrap
                    Accessible.role: Accessible.AlertMessage
                    Accessible.name: qsTr("Why it stopped")
                }

                // ---- The result ----
                Card {
                    objectName: "MeasureUnits.result"
                    title: qsTr("Result")
                    visible: tab.uc.hasResult
                    Layout.fillWidth: true
                    Text {
                        objectName: "MeasureUnits.timing"
                        Layout.fillWidth: true
                        text: tab.uc.timingText
                        font: Theme.fontBody
                        color: tab.uc.timingReliable ? Theme.textMuted : Theme.dangerText
                        wrapMode: Text.WordWrap
                    }
                    MeasureGraph {
                        objectName: "MeasureUnits.graph"
                        visible: tab.uc.graph && tab.uc.graph.series !== undefined
                        title: qsTr("Measured responses across units")
                        Layout.fillWidth: true
                        Layout.preferredHeight: 260
                        graph: tab.uc.graph
                    }
                    Repeater {
                        model: tab.uc.arrivals
                        delegate: Text {
                            required property var modelData
                            Layout.fillWidth: true
                            text: qsTr("%1: arrives %2; offered: %3").arg(modelData.label).arg(modelData.arrival).arg(modelData.offered)
                            font: Theme.fontSmall
                            color: Theme.text
                            wrapMode: Text.WordWrap
                        }
                    }
                    Repeater {
                        model: tab.uc.delays
                        delegate: Text {
                            required property var modelData
                            Layout.fillWidth: true
                            text: qsTr("%1 to %2: %3; offered: %4").arg(modelData.from).arg(modelData.to).arg(modelData.delay).arg(modelData.offered)
                            font: Theme.fontSmall
                            color: Theme.text
                            wrapMode: Text.WordWrap
                        }
                    }
                    Repeater {
                        model: tab.uc.reasons
                        delegate: Text {
                            required property string modelData
                            Layout.fillWidth: true
                            text: modelData
                            font: Theme.fontSmall
                            color: Theme.textMuted
                            wrapMode: Text.WordWrap
                        }
                    }
                }

                // ---- The proposal ----
                Card {
                    objectName: "MeasureUnits.proposal"
                    title: qsTr("Proposal")
                    visible: tab.uc.hasProposal
                    Layout.fillWidth: true
                    Text {
                        Layout.fillWidth: true
                        text: qsTr("The measured arrivals go into the hub's installation; the hub's alignment then sets every unit's delays. Nothing else changes.")
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.WordWrap
                    }
                    Repeater {
                        model: tab.uc.proposalArrivals
                        delegate: Text {
                            required property string modelData
                            Layout.fillWidth: true
                            text: modelData
                            font: Theme.fontSmall
                            color: Theme.text
                        }
                    }
                    Repeater {
                        model: tab.uc.withheld
                        delegate: Text {
                            required property string modelData
                            Layout.fillWidth: true
                            text: modelData
                            font: Theme.fontSmall
                            color: Theme.textMuted
                            wrapMode: Text.WordWrap
                        }
                    }
                    Repeater {
                        model: tab.uc.transitions
                        delegate: Text {
                            required property var modelData
                            Layout.fillWidth: true
                            text: modelData.text
                            font: Theme.fontSmall
                            color: modelData.failed ? Theme.dangerText : Theme.text
                            wrapMode: Text.WordWrap
                        }
                    }
                    Text {
                        objectName: "MeasureUnits.proposalNotice"
                        visible: text.length > 0
                        Layout.fillWidth: true
                        text: tab.uc.proposalNotice
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.WordWrap
                    }
                    // The hub's guard of the apply (or, on an older hub, that nothing reverts by itself)
                    Text {
                        objectName: "MeasureUnits.guardNote"
                        visible: text.length > 0 && (tab.uc.stage === "none" || tab.uc.stage === "kept" || tab.uc.stage === "reverted" || tab.uc.stage === "revert_incomplete")
                        Layout.fillWidth: true
                        text: tab.uc.guardNote
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.WordWrap
                    }
                    RowLayout {
                        visible: tab.uc.stage === "none" || tab.uc.stage === "kept" || tab.uc.stage === "reverted" || tab.uc.stage === "revert_incomplete"
                        spacing: Theme.spacingSmall
                        AppButton {
                            objectName: "MeasureUnits.sumCheck"
                            visible: tab.uc.sumCheckNeeded
                            text: qsTr("Run the summed check")
                            enabled: !tab.busy
                            onClicked: tab.uc.sumCheck()
                        }
                        // Admin's only: not offered below it
                        AppButton {
                            objectName: "MeasureUnits.apply"
                            visible: tab.uc.mayApply
                            text: qsTr("Apply the alignment")
                            primary: true
                            enabled: !tab.busy
                            onClicked: tab.uc.apply()
                        }
                        Text {
                            objectName: "MeasureUnits.applyWhy"
                            property string key: tab.uc.applyWhyKey
                            visible: !tab.uc.mayApply && text.length > 0
                            Layout.fillWidth: true
                            text: tab.uc.applyWhy
                            font: Theme.fontSmall
                            color: Theme.textMuted
                            wrapMode: Text.WordWrap
                        }
                    }
                    // Applied: the guard's countdown, the verify, Keep and Revert
                    ColumnLayout {
                        objectName: "MeasureUnits.applied"
                        visible: tab.uc.stage === "applied" || tab.uc.stage === "verified"
                        Layout.fillWidth: true
                        spacing: Theme.spacingSmall
                        Text {
                            objectName: "MeasureUnits.appliedText"
                            Layout.fillWidth: true
                            text: tab.uc.appliedText
                            font: Theme.fontBody
                            color: tab.uc.verifyFailed ? Theme.dangerText : Theme.text
                            wrapMode: Text.WordWrap
                        }
                        Text {
                            objectName: "MeasureUnits.guard"
                            visible: text.length > 0
                            Layout.fillWidth: true
                            text: tab.uc.guardText
                            font: Theme.fontBody
                            color: Theme.warning
                            wrapMode: Text.WordWrap
                        }
                        RowLayout {
                            spacing: Theme.spacingSmall
                            AppButton {
                                objectName: "MeasureUnits.verify"
                                visible: tab.uc.stage === "applied"
                                text: qsTr("Verify")
                                primary: true
                                enabled: !tab.busy
                                onClicked: tab.uc.verify()
                            }
                            AppButton {
                                objectName: "MeasureUnits.keep"
                                visible: tab.uc.stage === "verified"
                                text: qsTr("Keep")
                                primary: true
                                enabled: !tab.busy && !tab.uc.keepQuestion
                                onClicked: tab.uc.keep()
                            }
                            AppButton {
                                objectName: "MeasureUnits.revert"
                                visible: tab.uc.admin
                                text: qsTr("Revert")
                                destructive: true
                                enabled: !tab.busy
                                onClicked: tab.uc.revert()
                            }
                        }
                        // The second Keep's question: only the person's press, a second later, keeps
                        Rectangle {
                            objectName: "MeasureUnits.keepQuestion"
                            visible: tab.uc.keepQuestion
                            Layout.fillWidth: true
                            implicitHeight: questionBox.implicitHeight + 16
                            radius: Theme.radius
                            color: Theme.selection
                            Accessible.role: Accessible.AlertMessage
                            Accessible.name: qsTr("Keep anyway?")
                            ColumnLayout {
                                id: questionBox
                                anchors.fill: parent
                                anchors.margins: 8
                                spacing: Theme.spacingSmall
                                Text {
                                    Layout.fillWidth: true
                                    text: tab.uc.keepQuestionText
                                    font: Theme.fontBody
                                    color: Theme.text
                                    wrapMode: Text.WordWrap
                                }
                                RowLayout {
                                    spacing: Theme.spacingSmall
                                    AppButton {
                                        objectName: "MeasureUnits.keepAnyway"
                                        text: tab.uc.keepReady ? qsTr("Keep anyway") : qsTr("Keep anyway (in %1 s)").arg(tab.uc.keepWaitS)
                                        destructive: true
                                        enabled: tab.uc.keepReady && !tab.busy
                                        onClicked: tab.uc.confirmKeep()
                                    }
                                    AppButton {
                                        objectName: "MeasureUnits.keepNotNow"
                                        text: qsTr("Not now")
                                        onClicked: tab.uc.declineKeep()
                                    }
                                }
                            }
                        }
                    }
                    Text {
                        objectName: "MeasureUnits.outcome"
                        // -- WP REVERTW -- a revert not fully successful: its own stage, in the danger colour
                        visible: tab.uc.stage === "kept" || tab.uc.stage === "reverted" || tab.uc.stage === "revert_incomplete"
                        Layout.fillWidth: true
                        text: tab.uc.appliedText
                        font: Theme.fontBody
                        color: tab.uc.stage === "revert_incomplete" ? Theme.dangerText : Theme.text
                        wrapMode: Text.WordWrap
                    }
                }
            }
        }
    }
}
