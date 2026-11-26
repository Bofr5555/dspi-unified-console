import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P5QUI -- Tools > Measure… (PLAN-P5; the web's Measure screen): the
// setup and the run, the results, the live view, the auto-tune and the
// documents, in tabs. The Stop bar shows whenever anything plays or listens,
// above every tab. A connection that may not play the generator (read-only,
// the client read-only mode, a hub's Viewer) gets the passive microphone RTA,
// the results and the documents only, and says why (-- WP P5QUI2 --). This
// window plays nothing: the DSPi's generator is the only stimulus.
// -- WP LAY1b -- A panel (Panel.qml): PanelTable's row for tools.measure gives
// its window's title and sizes.
Panel {
    id: panel
    objectName: "MeasurePanel"
    // -- WP QTPOP -- the tabs that are a ScrollView
    fitBodies: [setupTab.contentItem, resultsTab.contentItem, liveTab.contentItem, tuneTab.contentItem, passiveTab.contentItem]

    readonly property bool blocked: MeasureController.playBlocker.length > 0
    readonly property bool modern: LayoutController.modern   // -- WP DL2 --
    // -- WP UA2 -- a refused Measure names its problems one by one (MeasureRefusalRow)
    readonly property bool refused: MeasureController.refusals.length > 0

    // Closing the window stops what it started (the guard of an applied tune runs on)
    // (review P5-D: always, so a start still on its way is withdrawn too)
    // -- WP MDQT2c -- on every device; -- WP LAY5b -- in Modern a window's close
    // leaves the panel on its page: the shown device's, and "Show Here" stops nothing
    onClosing: {
        const pages = inPage ? null : Shell.pageHost
        if (!pages || pages.moving !== actionId) MeasureController.panelClosing(pages !== null)
    }
    // -- WP P5QUI2 -- the client read-only mode read again when it shows
    // (-- WP LAY5b -- by panelShown, which counts the hosts)
    onShownChanged: {
        MeasureController.panelShown(shown)
        leavePassive()
        Qt.callLater(takePart)   // -- WP LAY7 -- not inside the page's own change
    }
    Component.onDestruction: if (shown) MeasureController.panelShown(false)

    // -- INTC (DL2 fold) -- Modern's page: Results' Open… and Documents' kind
    // filter, Reload and Import Calibration… in the tab row (PageHost.pageTools);
    // a window keeps them in its tabs
    readonly property Component pageTools: part === "results" ? resultsTools : part === "documents" ? docsTools : null
    Component {
        id: resultsTools
        AppButton {
            objectName: "MeasureResults.open"
            text: qsTr("Open…")
            onClicked: MeasureController.chooseResultFile()
        }
    }
    Component {
        id: docsTools
        Row {
            objectName: "MeasureDocsTools"
            spacing: 6
            AppComboBox {
                objectName: "MeasureDocs.kind"
                anchors.verticalCenter: parent.verticalCenter
                visible: !docsTab.none
                label: qsTr("Kind")
                width: 160
                model: docsTab.kinds
                textRole: "text"
                currentIndex: Math.max(0, docsTab.kinds.findIndex(k => k.id === docsTab.kind))
                onActivated: index => docsTab.kind = docsTab.kinds[index].id
            }
            AppButton {
                objectName: "MeasureDocs.reload"
                anchors.verticalCenter: parent.verticalCenter
                visible: !docsTab.none
                text: qsTr("Reload")
                onClicked: MeasureController.reloadDocuments()
            }
            AppButton {
                objectName: "MeasureDocs.importCalibration"
                anchors.verticalCenter: parent.verticalCenter
                text: qsTr("Import Calibration…")
                onClicked: MeasureController.chooseMicFile()
            }
        }
    }

    // -- WP LAY5b -- Modern's page has the tabs (SectionTable.parts): the part is
    // its; a window has its own tabs, as Classic
    readonly property var partIds: ["measure", "results", "live", "tune", "documents", "passive", "units"]
    readonly property bool paged: part.length > 0
    readonly property int index: paged ? Math.max(0, partIds.indexOf(part)) : tabs.currentIndex
    function showPart(id) {
        if (paged) partChosen(id)
        else if (partIds.indexOf(id) >= 0) tabs.currentIndex = partIds.indexOf(id)
    }
    // -- WP LAY7 -- An AI agent's start brings the tab it is run from
    function takePart() {
        if (!shown) return
        const id = MeasureController.takePart()
        if (id.length && !blocked) showPart(id)
    }
    Connections {
        target: MeasureController
        function onPartAsked() { panel.takePart() }
    }
    // Leaving the passive RTA's tab stops it (it gives the microphone back);
    // not a panel that is not shown: the other host's may be on it
    onIndexChanged: {
        leavePassive()
        // -- WP UA2 -- (A2) "The measurement is complete: see Results." (and
        // "Opened …: see Results.") is done with once Results or the Auto-tune show
        if (shown && (index === 1 || index === 3) && MeasureController.error.length === 0 && MeasureController.said.length > 0)
            MeasureController.clearError()
    }
    function leavePassive() { if (shown && index !== 5 && MeasureController.passiveActive) MeasureController.passiveStop() }

    // The tabs that play go when the connection may no longer play; the
    // passive RTA goes (and stops) when it may again
    onBlockedChanged: {
        if (blocked) showPart("passive")
        else if (index === 5) showPart("results")
        if (!blocked && MeasureController.passiveActive) MeasureController.passiveStop()
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacingSmall

        // -- WP UA2 -- (A2) the action bar of the job under way, both layouts:
        // progress and Stop while something plays or listens (it was the Stop
        // bar), a proposal's actions on the Auto-tune tab, and Keep and Revert
        // with the countdown while an applied tune waits, on every tab
        MeasureActionBar {
            Layout.fillWidth: true
            onTune: panel.index === 3
        }
        // -- WP MDQT2c -- what runs on the other open devices, each with its Stop
        Repeater {
            model: MeasureController.otherRuns
            Rectangle {
                id: other
                required property var modelData
                objectName: "Measure.otherRun"
                Layout.fillWidth: true
                implicitHeight: otherRow.implicitHeight + 12
                // -- WP DL2 -- Modern: a notice row's look
                radius: panel.modern ? Theme.radiusControl : Theme.radius
                color: panel.modern ? Theme.bannerFill("info") : Theme.selection
                RowLayout {
                    id: otherRow
                    anchors.fill: parent
                    anchors.margins: 6
                    Text {
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        text: qsTr("Running on %1. %2").arg(other.modelData.title).arg(other.modelData.text)
                        font: Theme.fontBody
                        color: Theme.text
                        wrapMode: Text.WordWrap
                    }
                    AppButton {
                        objectName: "Measure.otherRun.show"
                        text: qsTr("Show")
                        onClicked: MeasureController.showOther(other.modelData.id)
                    }
                    AppButton {
                        objectName: "Measure.otherRun.stop"
                        text: qsTr("Stop")
                        destructive: !panel.modern
                        onClicked: MeasureController.stopOther(other.modelData.id)
                    }
                }
            }
        }
        Text {
            objectName: panel.modern ? "" : "Measure.error"
            visible: text.length > 0 && !panel.modern && !panel.refused   // -- WP UA2 --
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: MeasureController.error
            font: Theme.fontBody
            color: Theme.dangerText
            wrapMode: Text.WordWrap
            Accessible.role: Accessible.AlertMessage
        }
        Text {
            objectName: "Measure.said"
            visible: text.length > 0 && MeasureController.error.length === 0
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: MeasureController.said
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.WordWrap
        }
        Text {
            visible: panel.blocked && !panel.modern
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: MeasureController.playBlocker
            font: Theme.fontBody
            color: Theme.warning
            wrapMode: Text.WordWrap
        }
        // -- WP DL2 -- Modern: the error and why nothing may play as notice rows
        NoticeRow {
            objectName: panel.modern ? "Measure.error" : ""
            visible: text.length > 0 && panel.modern && !panel.refused   // -- WP UA2 -- the rows below say it
            Layout.fillWidth: true
            kind: "error"
            text: MeasureController.error
        }
        // -- WP UA2 -- (A2) a refused Measure: one row per problem, with the fix
        // where the app has one (both layouts)
        Repeater {
            objectName: "Measure.refusals"
            model: MeasureController.refusals
            MeasureRefusalRow {
                required property var modelData
                required property int index
                objectName: "Measure.refusal." + index
                Layout.fillWidth: true
                key: modelData.key
                output: modelData.output
                text: modelData.text
                onShowTune: panel.showPart("tune")
            }
        }
        NoticeRow {
            objectName: "Measure.blocked"
            visible: panel.blocked && panel.modern
            Layout.fillWidth: true
            kind: "warning"
            text: MeasureController.playBlocker
        }

        AppTabBar {
            id: tabs
            objectName: "Measure.tabs"
            visible: !panel.paged   // -- WP LAY5b --
            label: qsTr("Measurement")
            Layout.fillWidth: true
            AppTabButton { objectName: "Measure.tab.measure"; text: qsTr("Measure"); enabled: !panel.blocked }
            AppTabButton { objectName: "Measure.tab.results"; text: qsTr("Results") }
            AppTabButton { objectName: "Measure.tab.live"; text: qsTr("Live"); enabled: !panel.blocked }
            AppTabButton { objectName: "Measure.tab.tune"; text: qsTr("Auto-tune"); enabled: !panel.blocked }
            AppTabButton { objectName: "Measure.tab.documents"; text: qsTr("Documents") }
            // -- WP P5QUI2 -- only for a connection that may not play
            AppTabButton { objectName: "Measure.tab.passive"; text: qsTr("Passive RTA"); visible: panel.blocked; width: visible ? implicitWidth : 0 }
            // -- WP ATQT -- several units of the hub, on one time base (not for a connection that may not play)
            AppTabButton { objectName: "Measure.tab.units"; text: qsTr("Several units"); visible: !panel.blocked; width: visible ? implicitWidth : 0 }
            Component.onCompleted: if (panel.blocked) currentIndex = 5
        }
        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: panel.index   // -- WP LAY5b --
            MeasureSetupTab { id: setupTab; onShowResults: panel.showPart("results") }
            MeasureResultsTab { id: resultsTab; toolsAbove: panel.paged; onShowTune: panel.showPart("tune") }   // -- INTC -- toolsAbove
            MeasureLiveTab { id: liveTab }
            MeasureTuneTab { id: tuneTab }
            MeasureDocsTab { id: docsTab; toolsAbove: panel.paged; onShowResults: panel.showPart("results") }   // -- INTC -- toolsAbove
            MeasurePassiveTab { id: passiveTab }
            MeasureUnitsTab {}   // -- WP ATQT --
        }
    }
}
