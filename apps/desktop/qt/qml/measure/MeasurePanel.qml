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
    onClosing: MeasureController.stopAll()   // -- WP MDQT2c -- on every device
    // -- WP P5QUI2 -- the client read-only mode read again when it shows
    onShownChanged: if (shown) MeasureController.refreshAccess()
    // The tabs that play go when the connection may no longer play; the
    // passive RTA goes (and stops) when it may again
    onBlockedChanged: {
        if (blocked) tabs.currentIndex = 5
        else if (!blocked && tabs.currentIndex === 5) tabs.currentIndex = 1
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
            // Leaving the passive RTA's tab stops it (it gives the microphone back)
            onCurrentIndexChanged: if (currentIndex !== 5 && MeasureController.passiveActive) MeasureController.passiveStop()
        }
        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: tabs.currentIndex
            MeasureSetupTab { id: setupTab; onShowResults: tabs.currentIndex = 1 }
            MeasureResultsTab { id: resultsTab; onShowTune: tabs.currentIndex = 3 }
            MeasureLiveTab { id: liveTab }
            MeasureTuneTab { id: tuneTab }
            MeasureDocsTab { onShowResults: tabs.currentIndex = 1 }
            MeasurePassiveTab { id: passiveTab }
            MeasureUnitsTab {}   // -- WP ATQT --
        }
    }
}
