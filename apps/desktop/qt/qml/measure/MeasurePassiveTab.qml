import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P5QUI2 -- The passive microphone RTA (PLAN-P5 section 1.5, T5; the
// web's passive.tsx; P5CURV's dspi_measure_rta_passive_* calls): what a
// connection that may not play the generator gets (a read-only connection,
// the client read-only mode, a hub's Viewer). The microphone (AudioCapture,
// never the simulated one: it hears only the generator) listens to whatever
// plays; its blocks go to the core, whose spectrum, held copy and compare are
// drawn here. Nothing is played and nothing on the DSPi changes. It is offered
// only while no measurement, live view or tune run plays on this console.
ScrollView {
    id: tab
    clip: true
    contentWidth: availableWidth

    readonly property var mc: MeasureController
    readonly property var averagings: [
        { id: "fast", text: qsTr("Fast (125 ms)") }, { id: "slow", text: qsTr("Slow (1 s)") }, { id: "infinite", text: qsTr("Infinite") }
    ]
    readonly property var micChoices: {
        const out = [{ id: "", text: qsTr("The system's default input") }]
        for (const i of mc.inputs) out.push({ id: i.id, text: i.name })
        return out
    }
    function indexOf(list, id) {
        for (let i = 0; i < list.length; ++i) if (list[i].id === id) return i
        return 0
    }
    readonly property bool active: mc.passiveActive

    ColumnLayout {
        width: tab.availableWidth
        spacing: Theme.spacing

        RowLayout {
            spacing: Theme.spacingSmall
            Text {
                Layout.fillWidth: true
                text: qsTr("Passive microphone RTA")
                font: Theme.fontTitle
                color: Theme.text
            }
            AppButton {
                objectName: "MeasurePassive.start"
                visible: !tab.active
                text: qsTr("Listen")
                primary: true
                enabled: mc.passiveBlocker.length === 0 && !mc.running
                onClicked: mc.passiveStart(mc.passiveAveraging)
            }
            AppButton {
                objectName: "MeasurePassive.stop"
                visible: tab.active
                text: qsTr("Stop Listening")
                destructive: true
                onClicked: mc.passiveStop()
            }
        }
        Text {
            Layout.fillWidth: true
            text: qsTr("The microphone listens to what is playing; nothing is played and nothing on the DSPi changes.")
            font: Theme.fontBody
            color: Theme.textMuted
            wrapMode: Text.WordWrap
        }
        Text {
            objectName: "MeasurePassive.blocker"
            visible: text.length > 0
            Layout.fillWidth: true
            text: mc.passiveBlocker
            font: Theme.fontBody
            color: Theme.dangerText
            wrapMode: Text.WordWrap
        }
        Flow {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            AppComboBox {
                objectName: "MeasurePassive.mic"
                label: qsTr("Microphone")
                width: 300
                enabled: !tab.active && !mc.running
                model: tab.micChoices
                textRole: "text"
                currentIndex: tab.indexOf(tab.micChoices, mc.micChoice)
                onActivated: index => mc.micChoice = tab.micChoices[index].id
            }
            AppComboBox {
                objectName: "MeasurePassive.averaging"
                label: qsTr("Averaging")
                width: 150
                model: tab.averagings
                textRole: "text"
                currentIndex: tab.indexOf(tab.averagings, mc.passiveAveraging)
                onActivated: index => mc.setPassiveAveraging(tab.averagings[index].id)
            }
            AppButton {
                objectName: "MeasurePassive.hold"
                text: qsTr("Hold")
                enabled: tab.active && mc.passiveReady
                onClicked: mc.passiveHold(true)
            }
            AppButton {
                objectName: "MeasurePassive.clearHold"
                text: qsTr("Clear Hold")
                enabled: tab.active && mc.passiveHeld
                onClicked: mc.passiveHold(false)
            }
        }
        Text {
            objectName: "MeasurePassive.state"
            Layout.fillWidth: true
            text: tab.active ? qsTr("%n frame(s) averaged", "", mc.passiveFrames) : qsTr("Not listening.")
            font: Theme.fontBody
            color: Theme.text
        }
        Text {
            Layout.fillWidth: true
            visible: mc.micCalibrated
            text: qsTr("Calibration: %1").arg(mc.micSummary)
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.WordWrap
        }
        MeasureGraph {
            objectName: "MeasurePassive.graph"
            title: qsTr("Passive microphone RTA")
            Layout.fillWidth: true
            Layout.preferredHeight: 300
            graph: mc.passiveGraph
        }
        MeasureGraph {
            objectName: "MeasurePassive.compare"
            visible: mc.passiveCompareGraph.series !== undefined
            title: qsTr("Compared with the held spectrum")
            Layout.fillWidth: true
            Layout.preferredHeight: 160
            graph: mc.passiveCompareGraph
        }
    }
}
