import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P5QUI -- The live view (the web's live.tsx; P5LIVE's calls): one
// output's short sweeps, or finite pink noise with the microphone RTA, the
// measured curve with the target (dashed) and the predicted curve (dotted) of
// a pending change; "cut this peak" at a point and "fit this range" over a
// span, each previewed and applied as one undo step (Undo takes it back); the
// RTA's averaging, hold and compare.
ScrollView {
    id: tab
    clip: true
    contentWidth: availableWidth

    readonly property var mc: MeasureController
    readonly property var roles: [
        { id: "full", text: qsTr("Full range") }, { id: "sub", text: qsTr("Subwoofer") },
        { id: "low", text: qsTr("Low (woofer)") }, { id: "mid", text: qsTr("Mid") },
        { id: "high", text: qsTr("High (tweeter)") }
    ]
    readonly property var placements: [
        { id: "listeningPosition", text: qsTr("Listening position") },
        { id: "nearField", text: qsTr("Near-field") },
        { id: "headphones", text: qsTr("Headphones on a coupler") }
    ]
    readonly property var couplers: [
        { id: "iec60318_4", text: qsTr("IEC 60318-4 ear simulator (711)") },
        { id: "flatPlate", text: qsTr("Flat-plate coupler") },
        { id: "hats", text: qsTr("HATS or another coupler with pinna") }
    ]
    readonly property var smoothings: [0, 1, 2, 3, 6, 12, 24, 48]
    readonly property var averagings: [
        { id: "fast", text: qsTr("Fast (125 ms)") }, { id: "slow", text: qsTr("Slow (1 s)") }, { id: "infinite", text: qsTr("Infinite") }
    ]
    function indexOf(list, id) {
        for (let i = 0; i < list.length; ++i) if (list[i].id === id) return i
        return 0
    }

    // The draft
    property int output: mc.outputs.length ? mc.outputs[0].output : 0
    property string role: "full"
    // -- WP ROLEQ -- the output's role from the speaker layout (else the
    // suggestion); a change here is written back to it
    function layoutRole() {
        const r = SpeakersController.roleOf(tab.output)
        if (r.length) tab.role = r
    }
    onOutputChanged: layoutRole()
    Component.onCompleted: { SpeakersController.reload(); layoutRole() }
    Connections {
        target: SpeakersController
        function onStoredChanged() { tab.layoutRole() }
    }
    // -- WP MDQT2c -- another device: its draft starts over (a placement chosen
    // for the last one is never assumed for it)
    Connections {
        target: MeasureController
        function onDeviceSwitched() {
            tab.placementChoice = ""
            tab.couplerChoice = ""
            tab.at = tab.from = tab.to = ""
            tab.output = tab.mc.outputs.length ? tab.mc.outputs[0].output : 0
            tab.layoutRole()
        }
    }
    property string mode: "sweep"
    // -- WP P5FOLLOW -- (coordinator, headphone safety) no default placement:
    // the person's choice here, else the Setup tab's when it is a live one
    property string placementChoice: ""
    readonly property string placement: placementChoice.length ? placementChoice
                                        : (setupIndexOf(mc.placement) >= 0 ? mc.placement : "")
    property string couplerChoice: ""
    readonly property string coupler: couplerChoice.length ? couplerChoice : (mc.coupler.length ? mc.coupler : "iec60318_4")
    function setupIndexOf(id) {
        for (let i = 0; i < placements.length; ++i) if (placements[i].id === id) return i
        return -1
    }
    property int smoothing: 6
    property string averaging: "slow"
    // The frequency, or the range, picked on the graph or typed
    property string at: ""
    property string from: ""
    property string to: ""

    readonly property bool active: mc.liveActive
    readonly property bool editable: !mc.running && mc.playBlocker.length === 0
    readonly property var mark: {
        const a = Number(from), b = Number(to), f = Number(at)
        if (from.length && to.length && b > a) return { f1: a, f2: b }
        if (at.length && f > 0) return { f1: f }
        return null
    }

    // -- WP DL2 -- Modern: no heading that repeats the tab, the state as a
    // chip beside Start Live, why it waits as a notice row, the adjustments
    // as a section; Classic keeps its heading, lines and card
    readonly property bool modern: LayoutController.modern
    component Section: Card {
        color: tab.modern ? "transparent" : Theme.panel
        padding: tab.modern ? 0 : Theme.spacing
    }

    function start() {
        mc.liveStart({ output: tab.output, role: tab.role, mode: tab.mode, placement: tab.placement,
                       coupler: tab.coupler, smoothing: tab.smoothing, averaging: tab.averaging })
    }

    ColumnLayout {
        width: tab.availableWidth
        spacing: Theme.spacing

        // -- WP ROLEQ -- while the speaker layout is not confirmed
        SpeakersBanner {
            objectName: "MeasureLive.speakersBanner"
            Layout.fillWidth: true
        }
        RowLayout {
            spacing: Theme.spacingSmall
            Text {
                visible: !tab.modern
                Layout.fillWidth: true
                text: qsTr("Live response")
                font: Theme.fontTitle
                color: Theme.text
            }
            AppButton {
                objectName: "MeasureLive.start"
                visible: !tab.active
                text: qsTr("Start Live")
                primary: true
                enabled: tab.editable && tab.placement.length > 0
                onClicked: tab.start()
            }
            AppButton {
                objectName: "MeasureLive.stop"
                visible: tab.active
                text: qsTr("Stop Live")
                destructive: !tab.modern
                onClicked: mc.liveStop()
            }
            // -- WP DL2 -- the state as a status chip (a dot and the words)
            StatusChip {
                objectName: "MeasureLive.status"
                visible: tab.modern && mc.liveText.length > 0
                Layout.maximumWidth: 480
                kind: mc.liveIssueText.length ? "warning" : tab.active ? "success" : "idle"
                text: mc.liveText
            }
            Item { visible: tab.modern; Layout.fillWidth: true }
        }
        // -- WP P5FOLLOW -- why Start Live waits
        NoticeRow {
            objectName: tab.modern ? "MeasureLive.placementNeeded" : ""
            visible: tab.modern && !tab.active && tab.placement.length === 0
            Layout.fillWidth: true
            kind: "warning"
            text: qsTr("Choose the placement first: speakers or headphones on a coupler. Headphones are measured much more quietly.")
        }
        Text {
            objectName: tab.modern ? "" : "MeasureLive.placementNeeded"
            visible: !tab.modern && !tab.active && tab.placement.length === 0
            Layout.fillWidth: true
            text: qsTr("Choose the placement first: speakers or headphones on a coupler. Headphones are measured much more quietly.")
            font: Theme.fontBody
            color: Theme.warning
            wrapMode: Text.WordWrap
        }
        Flow {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            AppComboBox {
                objectName: "MeasureLive.output"
                label: qsTr("Live output")
                enabled: tab.editable
                model: mc.outputs
                textRole: "label"
                currentIndex: Math.max(0, mc.outputs.findIndex(o => o.output === tab.output))
                onActivated: index => tab.output = mc.outputs[index].output
            }
            AppComboBox {
                objectName: "MeasureLive.role"
                label: qsTr("Way")
                width: 150
                enabled: tab.editable
                model: tab.roles
                textRole: "text"
                currentIndex: tab.indexOf(tab.roles, tab.role)
                onActivated: index => {
                    tab.role = tab.roles[index].id
                    SpeakersController.writeBack(tab.output, tab.role)   // -- WP ROLEQ --
                }
            }
            AppComboBox {
                label: qsTr("Placement")
                width: 200
                enabled: tab.editable
                model: tab.placements
                textRole: "text"
                objectName: "MeasureLive.placement"
                currentIndex: tab.setupIndexOf(tab.placement)
                displayText: currentIndex < 0 ? qsTr("Choose the placement") : currentText
                onActivated: index => tab.placementChoice = tab.placements[index].id
            }
            AppComboBox {
                visible: tab.placement === "headphones"
                label: qsTr("Coupler")
                width: 240
                enabled: tab.editable
                model: tab.couplers
                textRole: "text"
                currentIndex: tab.indexOf(tab.couplers, tab.coupler)
                onActivated: index => tab.couplerChoice = tab.couplers[index].id
            }
            AppComboBox {
                objectName: "MeasureLive.mode"
                label: qsTr("Plays")
                width: 170
                enabled: tab.editable
                model: [qsTr("Sweeps"), qsTr("Pink noise (RTA)")]
                currentIndex: tab.mode === "pinkRta" ? 1 : 0
                onActivated: index => tab.mode = index === 1 ? "pinkRta" : "sweep"
            }
            AppComboBox {
                label: qsTr("Smoothing")
                width: 130
                enabled: tab.editable
                model: tab.smoothings.map(n => n ? qsTr("1/%1 octave").arg(n) : qsTr("None"))
                currentIndex: Math.max(0, tab.smoothings.indexOf(tab.smoothing))
                onActivated: index => tab.smoothing = tab.smoothings[index]
            }
        }
        RowLayout {
            spacing: Theme.spacingSmall
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: mc.targetName.length ? qsTr("Target: %1").arg(mc.targetName) : qsTr("Target: the placement's default (our own curve)")
                font: Theme.fontSmall
                color: Theme.textMuted
                elide: Text.ElideRight
            }
            AppButton {
                text: qsTr("Import Target…")
                enabled: tab.editable
                onClicked: mc.chooseTargetFile(tab.placement === "headphones" ? "headphones" : "speakers", tab.coupler)
            }
            AppButton {
                visible: mc.targetName.length > 0
                text: qsTr("Default Target")
                enabled: tab.editable
                onClicked: mc.clearTarget()
            }
        }
        RowLayout {
            visible: tab.mode === "pinkRta"
            spacing: Theme.spacingSmall
            AppComboBox {
                label: qsTr("Averaging")
                Layout.preferredWidth: 150
                model: tab.averagings
                textRole: "text"
                currentIndex: tab.indexOf(tab.averagings, tab.averaging)
                onActivated: index => {
                    tab.averaging = tab.averagings[index].id
                    mc.liveAveraging(tab.averaging)
                }
            }
            AppButton {
                objectName: "MeasureLive.hold"
                text: qsTr("Hold")
                enabled: tab.active && mc.rtaReady
                onClicked: mc.liveHold(true)
            }
            AppButton {
                text: qsTr("Clear Hold")
                enabled: mc.rtaHeld
                onClicked: mc.liveHold(false)
            }
            Text {
                visible: tab.active
                text: qsTr("%n frame(s) averaged", "", mc.rtaFrames)
                font: Theme.fontSmall
                color: Theme.textMuted
            }
        }
        Text {
            objectName: "MeasureLive.state"
            visible: !tab.modern   // -- WP DL2 -- Modern: the status chip
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: mc.liveText
            font: Theme.fontBody
            color: Theme.text
        }
        Text {
            visible: text.length > 0 && !tab.modern
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: mc.liveIssueText
            font: Theme.fontBody
            color: Theme.dangerText
            wrapMode: Text.WordWrap
        }
        NoticeRow {
            objectName: "MeasureLive.issue"
            visible: text.length > 0 && tab.modern
            Layout.fillWidth: true
            kind: "error"
            text: mc.liveIssueText
        }
        MeasureGraph {
            objectName: "MeasureLive.graph"
            title: qsTr("Live response")
            Layout.fillWidth: true
            Layout.preferredHeight: 300
            graph: mc.liveGraph
            mark: tab.mark
            interactive: true
            onPicked: hz => {
                tab.at = String(Math.round(hz))
                tab.from = ""
                tab.to = ""
            }
            onSpanned: (a, b) => {
                tab.from = String(Math.round(a))
                tab.to = String(Math.round(b))
                tab.at = ""
            }
        }
        MeasureGraph {
            visible: mc.compareGraph.series !== undefined
            title: qsTr("Compared with the held spectrum")
            Layout.fillWidth: true
            Layout.preferredHeight: 160
            graph: mc.compareGraph
        }

        // The direct adjustments
        Section {
            title: qsTr("Direct adjustments")
            Layout.fillWidth: true
            RowLayout {
                spacing: Theme.spacingSmall
                Text { text: qsTr("Peak at, Hz"); font: Theme.fontBody; color: Theme.text }
                BorderedField {
                    objectName: "MeasureLive.at"
                    label: qsTr("Peak frequency")
                    Layout.preferredWidth: 80
                    text: tab.at
                    onTextEdited: tab.at = text
                }
                AppButton {
                    text: qsTr("Find the Highest Peak")
                    toolTip: qsTr("The highest trusted point from 100 Hz to 5 kHz")
                    enabled: tab.active && mc.livePeakHz > 0
                    onClicked: tab.at = String(Math.round(mc.livePeakHz))
                }
                AppButton {
                    objectName: "MeasureLive.cut"
                    text: qsTr("Cut This Peak")
                    enabled: tab.active && mc.liveMeasured
                    onClicked: mc.cutPeak(Number(tab.at))
                }
            }
            RowLayout {
                spacing: Theme.spacingSmall
                Text { text: qsTr("From, Hz"); font: Theme.fontBody; color: Theme.text }
                BorderedField {
                    label: qsTr("Range from")
                    Layout.preferredWidth: 80
                    text: tab.from
                    onTextEdited: tab.from = text
                }
                Text { text: qsTr("to"); font: Theme.fontBody; color: Theme.text }
                BorderedField {
                    label: qsTr("Range to")
                    Layout.preferredWidth: 80
                    text: tab.to
                    onTextEdited: tab.to = text
                }
                AppButton {
                    objectName: "MeasureLive.fit"
                    text: qsTr("Fit This Range")
                    enabled: tab.active && mc.liveMeasured
                    onClicked: mc.fitRange(Number(tab.from), Number(tab.to))
                }
                Item { Layout.fillWidth: true }
                AppButton {
                    objectName: "MeasureLive.undo"
                    text: qsTr("Undo")
                    enabled: HistoryController.canUndo
                    onClicked: mc.undo()
                }
            }
            // The pending change
            ColumnLayout {
                objectName: "MeasureLive.proposal"
                visible: mc.proposal.id !== undefined
                Layout.fillWidth: true
                spacing: 4
                Text {
                    visible: (mc.proposal.reason || "").length === 0
                    textFormat: Text.PlainText
                    text: mc.proposal.label || ""
                    font.pixelSize: Theme.fontBody.pixelSize
                    font.weight: Font.DemiBold
                    color: Theme.text
                }
                Repeater {
                    model: mc.proposal.changes || []
                    Text {
                        required property string modelData
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        text: "• " + modelData
                        font: Theme.fontBody
                        color: Theme.text
                        wrapMode: Text.WordWrap
                    }
                }
                Text {
                    visible: text.length > 0
                    text: mc.proposal.gainText || ""
                    font: Theme.fontBody
                    color: Theme.text
                }
                Text {
                    visible: (mc.proposal.reason || "").length === 0
                    Layout.fillWidth: true
                    text: mc.proposal.rmsText || ""
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.WordWrap
                }
                Text {
                    visible: text.length > 0
                    Layout.fillWidth: true
                    text: mc.proposal.reason || ""
                    font: Theme.fontBody
                    // -- WP DL2 -- Modern: a warning icon before words in the text colour
                    color: tab.modern ? Theme.text : Theme.warning
                    wrapMode: Text.WordWrap
                    leftPadding: tab.modern ? 22 : 0
                    Icon {
                        visible: tab.modern
                        y: Math.max(0, (parent.font.pixelSize * 1.3 - height) / 2)
                        name: "triangle-alert"
                        color: Theme.warning
                    }
                }
                RowLayout {
                    visible: (mc.proposal.reason || "").length === 0
                    spacing: Theme.spacingSmall
                    AppButton {
                        objectName: "MeasureLive.apply"
                        text: qsTr("Apply")
                        primary: true
                        enabled: !mc.busy
                        onClicked: mc.liveApply()
                    }
                    AppButton {
                        objectName: "MeasureLive.discard"
                        text: qsTr("Discard")
                        onClicked: mc.liveDiscard()
                    }
                }
            }
            Text {
                objectName: "MeasureLive.said"
                visible: text.length > 0
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: mc.liveSaid
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.WordWrap
            }
        }
    }
}
