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
    onIndexChanged: leavePassive()
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

        // The Stop bar
        Rectangle {
            objectName: "Measure.stopBar"
            visible: MeasureController.running
            Layout.fillWidth: true
            implicitHeight: stopRow.implicitHeight + 12
            radius: Theme.radius
            color: Theme.selection
            RowLayout {
                id: stopRow
                anchors.fill: parent
                anchors.margins: 6
                Text {
                    objectName: "Measure.runningText"
                    Layout.fillWidth: true
                    // -- WP MDQT2c -- with several devices open, on which
                    textFormat: Text.PlainText
                    text: MeasureController.runningOn.length
                          ? qsTr("Running on %1. %2").arg(MeasureController.runningOn).arg(MeasureController.runningText)
                          : MeasureController.runningText
                    font: Theme.fontBody
                    color: Theme.text
                    wrapMode: Text.WordWrap
                }
                AppButton {
                    objectName: "Measure.stop"
                    text: qsTr("Stop")
                    destructive: true
                    onClicked: MeasureController.stop()
                }
            }
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
                radius: Theme.radius
                color: Theme.selection
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
                        destructive: true
                        onClicked: MeasureController.stopOther(other.modelData.id)
                    }
                }
            }
        }
        Text {
            objectName: "Measure.error"
            visible: text.length > 0
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
            visible: panel.blocked
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: MeasureController.playBlocker
            font: Theme.fontBody
            color: Theme.warning
            wrapMode: Text.WordWrap
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
            MeasureResultsTab { id: resultsTab; onShowTune: panel.showPart("tune") }
            MeasureLiveTab { id: liveTab }
            MeasureTuneTab { id: tuneTab }
            MeasureDocsTab { onShowResults: panel.showPart("results") }
            MeasurePassiveTab { id: passiveTab }
            MeasureUnitsTab {}   // -- WP ATQT --
        }
    }
}
