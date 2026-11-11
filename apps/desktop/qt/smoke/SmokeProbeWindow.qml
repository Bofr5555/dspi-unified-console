import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP0 -- The smoke driver's tool window (SmokeProbe.h), and a sample of
// the base components in it
ToolWindow {
    objectName: "SmokeProbeWindow"
    windowId: "smokeProbe"
    title: "Smoke probe"
    defaultWidth: 460
    defaultHeight: 320

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacing
        Card {
            Layout.fillWidth: true
            title: "Components"
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
        Banner { Layout.fillWidth: true; kind: "info"; text: "An information banner" }
        Banner { Layout.fillWidth: true; kind: "error"; text: "An error banner" }
    }
}
