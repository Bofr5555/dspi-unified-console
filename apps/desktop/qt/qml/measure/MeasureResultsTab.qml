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
// -- WP P5QUI2 -- the curves are the core's (dspi_measure_doc_curves, R1): the
// smoothing chosen here (1/6 octave by default) and the target levelled to
// each output (dashed), which can be switched off. Nothing is computed here.
ScrollView {
    id: tab
    signal showTune()
    clip: true
    contentWidth: availableWidth

    readonly property var mc: MeasureController
    // -- INTC (DL2 fold) -- Modern's page shows Open… in its tab row (MeasurePanel's pageTools)
    property bool toolsAbove: false
    readonly property var smoothings: [0, 1, 2, 3, 6, 12, 24, 48]
    // -- WP DL2 -- Modern: nothing to show is an empty state; Classic keeps its line
    readonly property bool modern: LayoutController.modern

    ColumnLayout {
        width: tab.availableWidth
        spacing: Theme.spacing

        EmptyState {
            objectName: "MeasureResults.empty"
            visible: tab.modern && !mc.hasResult
            Layout.fillWidth: true
            Layout.preferredHeight: Math.max(implicitHeight, tab.availableHeight)
            iconName: "chart-line"
            title: qsTr("No measurement yet")
            text: qsTr("Measure on the Measure tab, or open a measurement file.")
            actionText: qsTr("Open a Measurement…")
            onAction: mc.chooseResultFile()
        }
        RowLayout {
            visible: !tab.modern || mc.hasResult
            spacing: Theme.spacingSmall
            Text {
                objectName: "MeasureResults.name"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: mc.hasResult ? (mc.resultName || qsTr("Measurement")) : qsTr("No measurement yet: run one on the Measure tab, or open a measurement file.")
                font: mc.hasResult ? Theme.fontTitle : Theme.fontBody
                color: mc.hasResult ? Theme.text : Theme.textMuted
                wrapMode: Text.WordWrap
            }
            AppButton {
                visible: !tab.toolsAbove   // -- INTC -- on Modern's page: the tab row's
                text: qsTr("Open…")
                onClicked: mc.chooseResultFile()
            }
        }
        Text {
            visible: mc.hasResult
            Layout.fillWidth: true
            textFormat: Text.PlainText
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
            textFormat: Text.PlainText
            text: mc.resultTiming.length ? mc.resultTiming : qsTr("No timing: a single sweep is magnitude only.")
            font: Theme.fontBody
            // -- WP DL2 -- Modern: the state in the icon before it, the words in the text colour
            color: tab.modern ? (reliable === undefined || reliable === null ? Theme.textMuted : Theme.text)
                              : reliable === true ? Theme.success : reliable === false ? Theme.warning : Theme.textMuted
            wrapMode: Text.WordWrap
            leftPadding: timingIcon.visible ? timingIcon.width + 6 : 0
            Icon {
                id: timingIcon
                visible: tab.modern && (parent.reliable === true || parent.reliable === false)
                y: Math.max(0, (parent.font.pixelSize * 1.3 - height) / 2)
                name: parent.reliable === true ? "circle-check" : "triangle-alert"
                color: parent.reliable === true ? Theme.success : Theme.warning
            }
        }
        RowLayout {
            visible: mc.hasResult
            spacing: Theme.spacingSmall
            AppComboBox {
                objectName: "MeasureResults.smoothing"
                label: qsTr("Results smoothing")
                Layout.preferredWidth: 150
                model: tab.smoothings.map(n => n ? qsTr("1/%1 octave").arg(n) : qsTr("None"))
                currentIndex: Math.max(0, tab.smoothings.indexOf(mc.resultSmoothing))
                onActivated: index => mc.resultSmoothing = tab.smoothings[index]
            }
            AppCheckBox {
                objectName: "MeasureResults.target"
                text: mc.targetName.length ? qsTr("Target (%1), levelled to each output").arg(mc.targetName)
                                           : qsTr("Target (the placement's default), levelled to each output")
                checked: mc.resultTarget
                onToggled: mc.resultTarget = checked
            }
            Item { Layout.fillWidth: true }
        }
        Text {
            objectName: "MeasureResults.curvesNote"
            visible: mc.hasResult
            Layout.fillWidth: true
            text: mc.resultCurvesSmoothing < 0 ? qsTr("Asking the core for the curves…")
                  : mc.resultCurvesSmoothing === 0 ? qsTr("The curves as stored (1/96 octave), from the core.")
                  : qsTr("Smoothed by the core: 1/%1 octave.").arg(mc.resultCurvesSmoothing)
            font: Theme.fontSmall
            color: Theme.textMuted
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
                    textFormat: Text.PlainText
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
