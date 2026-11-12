import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// A slider with its label above it, the label carrying the value ("Line
// Width: 1.5pt"); it applies while dragging. Right-click resets to `resetValue`.
ColumnLayout {
    id: row
    property string label
    property alias from: slider.from
    property alias to: slider.to
    property alias stepSize: slider.stepSize
    property alias resetValue: slider.resetValue
    property real value
    signal moved(real value)

    Layout.fillWidth: true
    spacing: 4

    Text {
        Layout.fillWidth: true
        text: row.label
        font: Theme.fontBody
        color: row.enabled ? Theme.text : Theme.textDisabled
    }
    CustomSlider {
        id: slider
        objectName: row.objectName.length ? row.objectName + ".slider" : ""
        Layout.fillWidth: true
        label: row.label
        value: row.value
        snapMode: stepSize > 0 ? Slider.SnapAlways : Slider.NoSnap
        onValueMoved: v => row.moved(v)
        onCommitted: v => row.moved(v)
    }
}
