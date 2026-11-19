import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P5QUI -- Tools > Measure… (PLAN-P5; the web's Measure screen): the
// setup and the run, the results, the live view, the auto-tune and the
// documents, in tabs. The Stop bar shows whenever anything plays or listens,
// above every tab. A connection that may not play the generator (read-only)
// gets the results and the documents only, and says why. This window plays
// nothing: the DSPi's generator is the only stimulus.
ToolWindow {
    id: window
    objectName: "MeasureWindow"
    windowId: "measure"
    defaultWidth: 940
    defaultHeight: 720
    minimumWidth: 720
    minimumHeight: 520
    title: qsTr("Measure")

    readonly property bool blocked: MeasureController.playBlocker.length > 0

    // Closing the window stops what it started (the guard of an applied tune runs on)
    onClosing: if (MeasureController.running) MeasureController.stop()

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
                    Layout.fillWidth: true
                    text: MeasureController.runningText
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
        Text {
            objectName: "Measure.error"
            visible: text.length > 0
            Layout.fillWidth: true
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
            text: MeasureController.said
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.WordWrap
        }
        Text {
            visible: window.blocked
            Layout.fillWidth: true
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
            AppTabButton { objectName: "Measure.tab.measure"; text: qsTr("Measure"); enabled: !window.blocked }
            AppTabButton { objectName: "Measure.tab.results"; text: qsTr("Results") }
            AppTabButton { objectName: "Measure.tab.live"; text: qsTr("Live"); enabled: !window.blocked }
            AppTabButton { objectName: "Measure.tab.tune"; text: qsTr("Auto-tune"); enabled: !window.blocked }
            AppTabButton { objectName: "Measure.tab.documents"; text: qsTr("Documents") }
            Component.onCompleted: if (window.blocked) currentIndex = 1
        }
        StackLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            currentIndex: tabs.currentIndex
            MeasureSetupTab { onShowResults: tabs.currentIndex = 1 }
            MeasureResultsTab { onShowTune: tabs.currentIndex = 3 }
            MeasureLiveTab {}
            MeasureTuneTab {}
            MeasureDocsTab { onShowResults: tabs.currentIndex = 1 }
        }
    }
}
