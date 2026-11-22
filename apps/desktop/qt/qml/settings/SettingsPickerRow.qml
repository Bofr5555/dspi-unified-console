import QtQuick
import QtQuick.Layouts
import UConsole

// A labelled popup choice (the Mac's labelled Picker): `texts` are shown,
// `values` are what they stand for, index by index; the popup shows the entry
// of `value`. `chosen(value)` fires on a person's choice only. `inline` puts
// the label and the popup on one line, like a page's field rows.
GridLayout {
    id: row
    property bool inline: false
    property string label
    property var values: []
    property var texts: []
    property var value
    signal chosen(var value)

    columns: inline ? 2 : 1
    rowSpacing: 4
    columnSpacing: Theme.spacing
    Layout.fillWidth: true

    Text {
        textFormat: Text.PlainText
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
        Layout.fillWidth: !row.inline
        Layout.preferredWidth: row.inline ? 220 : -1
        Layout.maximumWidth: 220
    }
}
