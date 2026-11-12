import QtQuick
import QtQuick.Layouts
import UConsole

// A labelled popup choice (the Mac's labelled Picker): `texts` are shown,
// `values` are what they stand for, index by index; the popup shows the entry
// of `value`. `chosen(value)` fires on a person's choice only.
ColumnLayout {
    id: row
    property string label
    property var values: []
    property var texts: []
    property var value
    signal chosen(var value)

    spacing: 4
    Layout.fillWidth: true

    Text {
        text: row.label
        color: row.enabled ? Theme.text : Theme.textDisabled
        font: Theme.fontBody
        Layout.fillWidth: true
    }
    AppComboBox {
        objectName: row.objectName.length ? row.objectName + ".combo" : ""
        label: row.label
        model: row.texts
        currentIndex: row.values.indexOf(row.value)
        onActivated: choice => row.chosen(row.values[choice])
        Layout.fillWidth: true
        Layout.maximumWidth: 220
    }
}
