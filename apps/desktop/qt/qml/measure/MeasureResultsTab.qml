import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P5QUI -- A measurement's results (the web's results.tsx): each
// output's response as the document holds it (untrusted points faint, the
// spread across positions or reseats as a band), the timing verdict in words
// ("timing unreliable" with its reasons), the placement and the microphone;
// saved in this console's store or on the hub, exported as the document or
// as REW text per output, or a file opened.
ScrollView {
    id: tab
    signal showTune()
    clip: true
    contentWidth: availableWidth

    readonly property var mc: MeasureController

    ColumnLayout {
        width: tab.availableWidth
        spacing: Theme.spacing

        RowLayout {
            spacing: Theme.spacingSmall
            Text {
                objectName: "MeasureResults.name"
                Layout.fillWidth: true
                text: mc.hasResult ? (mc.resultName || qsTr("Measurement")) : qsTr("No measurement yet: run one on the Measure tab, or open a measurement file.")
                font: mc.hasResult ? Theme.fontTitle : Theme.fontBody
                color: mc.hasResult ? Theme.text : Theme.textMuted
                wrapMode: Text.WordWrap
            }
            AppButton {
                text: qsTr("Open…")
                onClicked: mc.chooseResultFile()
            }
        }
        Text {
            visible: mc.hasResult
            Layout.fillWidth: true
            text: mc.resultSummary
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.WordWrap
        }
        Text {
            objectName: "MeasureResults.timing"
            visible: mc.hasResult
            Layout.fillWidth: true
            readonly property var reliable: mc.resultTimingReliable
            text: mc.resultTiming.length ? mc.resultTiming : qsTr("No timing: a single sweep is magnitude only.")
            font: Theme.fontBody
            color: reliable === true ? Theme.success : reliable === false ? Theme.warning : Theme.textMuted
            wrapMode: Text.WordWrap
        }
        MeasureGraph {
            objectName: "MeasureResults.graph"
            visible: mc.hasResult
            title: qsTr("Measured responses")
            Layout.fillWidth: true
            Layout.preferredHeight: 300
            graph: mc.resultGraph
        }
        Repeater {
            model: mc.hasResult ? mc.resultOutputs : []
            RowLayout {
                required property var modelData
                spacing: Theme.spacingSmall
                Rectangle {
                    width: 12
                    height: 12
                    radius: 2
                    color: ChannelPalette.outputs[modelData.colour % ChannelPalette.outputs.length]
                }
                AppCheckBox {
                    objectName: "MeasureResults.output." + modelData.output
                    text: modelData.label
                    Layout.preferredWidth: 220
                    checked: modelData.shown
                    onToggled: mc.setOutputShown(modelData.output, checked)
                }
                Text {
                    Layout.fillWidth: true
                    text: modelData.role + (modelData.speaker ? qsTr(", speaker %1").arg(modelData.speaker) : "") +
                          (modelData.arrivalMs !== undefined && modelData.arrivalMs !== null ? qsTr(", arrives at %1 ms").arg(Number(modelData.arrivalMs).toFixed(3)) : "")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    elide: Text.ElideRight
                }
                AppButton {
                    text: qsTr("REW Text…")
                    onClicked: mc.exportRew(modelData.output)
                }
            }
        }
        RowLayout {
            visible: mc.hasResult
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "MeasureResults.save"
                text: qsTr("Save in This Console")
                enabled: !mc.busy
                onClicked: mc.saveResult()
            }
            AppButton {
                objectName: "MeasureResults.saveHub"
                visible: mc.hubDocuments
                text: qsTr("Save on the Hub")
                enabled: !mc.busy && mc.playBlocker.length === 0
                onClicked: mc.saveResultToHub()
            }
            AppButton {
                text: qsTr("Export…")
                onClicked: mc.exportResult()
            }
            Item { Layout.fillWidth: true }
            AppButton {
                text: qsTr("Auto-tune From This…")
                primary: true
                enabled: mc.playBlocker.length === 0
                onClicked: tab.showTune()
            }
        }
    }
}
