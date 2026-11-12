import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// A slider under its label, the label carrying the value ("Line Width:
// 1.5pt"); it applies while dragging and on release. Right-click puts it back
// to `resetValue`. `step` 0 slides freely.
ColumnLayout {
    id: row
    property string label
    property real minimum: 0
    property real maximum: 1
    property real step: 0
    property real resetValue: NaN
    property real value
    signal moved(real value)

    spacing: 4
    Layout.fillWidth: true

    Text {
        text: row.label
        color: row.enabled ? Theme.text : Theme.textDisabled
        font: Theme.fontBody
        Layout.fillWidth: true
    }
    CustomSlider {
        objectName: row.objectName.length ? row.objectName + ".slider" : ""
        label: row.label
        from: row.minimum
        to: row.maximum
        stepSize: row.step
        snapMode: row.step > 0 ? Slider.SnapAlways : Slider.NoSnap
        resetValue: row.resetValue
        value: row.value
        onValueMoved: v => row.moved(v)
        onCommitted: v => row.moved(v)
        Layout.fillWidth: true
    }
}
