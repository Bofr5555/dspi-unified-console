import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP0 -- The smoke driver's tool window (SmokeProbe.h): every base
// component once, with what a check reads back
ToolWindow {
    id: probe
    objectName: "SmokeProbeWindow"
    windowId: "smokeProbe"
    title: "Smoke probe"
    defaultWidth: 560
    defaultHeight: 640

    property int buttonClicks: 0
    property string lastAnswer
    property real spinCommitted: NaN

    function ask() { question.open() }

    ScrollView {
        anchors.fill: parent
        ScrollBar.vertical: AppScrollBar { objectName: "SmokeProbeWindow.scrollBar" }
        ColumnLayout {
            width: probe.width - 2 * Theme.spacing
            x: Theme.spacing
            spacing: Theme.spacing
            SectionHeading { objectName: "SmokeProbeWindow.heading"; Layout.topMargin: Theme.spacing; text: "Components"; rule: true }
            Card {
                Layout.fillWidth: true
                title: "Base"
                RowLayout {
                    spacing: Theme.spacingSmall
                    Pill { text: "IN1"; tint: ChannelPalette.input(0) }
                    Pill { text: "OUT9"; tint: ChannelPalette.output(8, true) }
                    Pill { text: "Curve"; tint: ChannelPalette.output(0, false); checkable: true; checked: true }
                    IconButton { iconName: "settings"; toolTip: "Settings" }
                    IconButton { iconName: "sliders-horizontal"; toolTip: "Matrix" }
                }
                CustomSlider { objectName: "SmokeProbeWindow.slider"; label: "Gain"; from: -24; to: 24; value: 0; resetValue: 0; Layout.fillWidth: true }
                BorderedField { label: "Name"; text: "Field"; Layout.fillWidth: true }
            }
            Card {
                Layout.fillWidth: true
                title: "Controls"
                RowLayout {
                    spacing: Theme.spacingSmall
                    AppButton { objectName: "SmokeProbeWindow.button"; text: "Apply"; onClicked: probe.buttonClicks++ }
                    AppButton { text: "Save"; primary: true; iconName: "save" }
                    AppButton { text: "Clear"; destructive: true }
                    AppButton { text: "Off"; enabled: false }
                }
                AppSwitch { objectName: "SmokeProbeWindow.switch"; text: "Enabled"; Layout.fillWidth: true }
                RowLayout {
                    AppCheckBox { objectName: "SmokeProbeWindow.checkBox"; text: "Mute" }
                    AppCheckBox { text: "Some"; tristate: true; checkState: Qt.PartiallyChecked }
                }
                RowLayout {
                    spacing: Theme.spacingSmall
                    AppComboBox { objectName: "SmokeProbeWindow.comboBox"; label: "Source"; model: ["USB", "S/PDIF", "I2S"] }
                    AppSpinBox {
                        objectName: "SmokeProbeWindow.spinBox"; label: "Delay"
                        realFrom: -10; realTo: 10; realStep: 0.5; decimals: 1; unit: "ms"; realValue: 0
                        onCommitted: v => probe.spinCommitted = v
                    }
                }
                AppTabBar {
                    objectName: "SmokeProbeWindow.tabBar"; label: "Filters"
                    AppTabButton { text: "PEQ" }
                    AppTabButton { objectName: "SmokeProbeWindow.tab2"; text: "XO" }
                }
            }
            Banner { Layout.fillWidth: true; kind: "info"; text: "An information banner" }
            Banner { Layout.fillWidth: true; kind: "warning"; text: "A warning banner" }
            Banner { Layout.fillWidth: true; kind: "error"; text: "An error banner" }
            Item { implicitHeight: 200 }   // enough to scroll
        }
    }

    PopoverPanel {
        id: popover
        objectName: "SmokeProbeWindow.popover"
        title: "Graph options"
        x: 40
        y: 40
        AppSwitch { text: "Show phase" }
    }
    function showPopover() { popover.open() }

    QuestionDialog {
        id: question
        objectName: "SmokeProbeWindow.question"
        title: "Save changes?"
        text: "The preset has unsaved changes."
        acceptText: "Save"
        alternativeText: "Don't Save"
        onAnswered: answer => probe.lastAnswer = answer
    }
}
