import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// A slider under its label, the label carrying the value ("Line Width:
// 1.5pt"); it applies while dragging and on release. Right-click puts it back
// to `resetValue`. `step` 0 slides freely.
// -- WP DL7 -- Modern (DESIGN-LANGUAGE §10, §11), when the page gives a
// `title`: one form row, the title (and `caption`) at the left, the slider,
// and a value field at the right in `unit`, never the value in the label.
// The field shows value × `scale` with `decimals` (Grid Opacity's 0..2 is
// 0..200 %); a typed value is held to the range and to `step`.
//
//     SettingsSliderRow {
//         label: qsTr("Line Width: %1pt").arg(width.toFixed(1))   // Classic
//         title: qsTr("Line width"); unit: "pt"; decimals: 1        // Modern
//         minimum: 1; maximum: 4; step: 0.5; value: width
//         onMoved: v => width = v
//     }
ColumnLayout {
    id: row
    property string label
    property real minimum: 0
    property real maximum: 1
    property real step: 0
    property real resetValue: NaN
    property real value
    signal moved(real value)
    // -- WP DL7 --
    property string title
    property string caption
    property string unit
    property int decimals: 0
    property real scale: 1
    readonly property bool formed: LayoutController.modern && title.length > 0

    spacing: 4
    Layout.fillWidth: true

    Text {
        visible: !row.formed   // -- WP DL7 --
        textFormat: Text.PlainText
        text: row.label
        color: row.enabled ? Theme.text : Theme.textDisabled
        font: Theme.fontBody
        Layout.fillWidth: true
    }
    RowLayout {   // -- WP DL7 -- Modern: title, slider, field in one row
        Layout.fillWidth: true
        Layout.minimumHeight: row.formed ? Theme.formRowHeight : 0
        spacing: Theme.spacing
        ColumnLayout {
            visible: row.formed
            spacing: 2
            Layout.fillWidth: false   // (a layout fills by default: the slider takes the room)
            Layout.preferredWidth: 200
            Layout.alignment: Qt.AlignVCenter
            Text {
                objectName: row.objectName.length ? row.objectName + ".title" : ""
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: row.title
                font: Theme.fontBody
                color: row.enabled ? Theme.text : Theme.textDisabled
                wrapMode: Text.Wrap
            }
            Text {
                Layout.fillWidth: true
                visible: text.length > 0
                textFormat: Text.PlainText
                text: row.caption
                font: Theme.fontSmall
                color: row.enabled ? Theme.textMuted : Theme.textDisabled
                wrapMode: Text.Wrap
            }
        }
        CustomSlider {
            objectName: row.objectName.length ? row.objectName + ".slider" : ""
            label: row.formed ? row.title : row.label
            from: row.minimum
            to: row.maximum
            stepSize: row.step
            snapMode: row.step > 0 ? Slider.SnapAlways : Slider.NoSnap
            resetValue: row.resetValue
            value: row.value
            onValueMoved: v => row.moved(v)
            onCommitted: v => row.moved(v)
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignVCenter
        }
        NumericField {
            objectName: row.objectName.length ? row.objectName + ".field" : ""
            visible: row.formed
            implicitWidth: 88
            Layout.alignment: Qt.AlignVCenter
            label: row.title
            unit: row.unit
            decimals: row.decimals
            from: row.minimum * row.scale
            to: row.maximum * row.scale
            step: row.step > 0 ? row.step * row.scale : Math.pow(10, -row.decimals)
            resetValue: row.resetValue * row.scale
            value: Math.round(row.value * row.scale * Math.pow(10, row.decimals)) / Math.pow(10, row.decimals)
            onCommitted: v => {
                let to = v / row.scale
                if (row.step > 0) to = row.minimum + Math.round((to - row.minimum) / row.step) * row.step
                row.moved(Math.max(row.minimum, Math.min(row.maximum, to)))
            }
        }
    }
}
