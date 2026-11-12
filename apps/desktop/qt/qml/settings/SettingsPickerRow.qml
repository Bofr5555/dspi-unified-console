import QtQuick
import QtQuick.Layouts
import UConsole

// A labelled popup choice over [{text, value}] options: the label above, the
// popup under it (the Mac's labelled Picker). `chosen(value)` on a person's choice.
ColumnLayout {
    id: row
    property string label
    property var options: []
    property var value
    signal chosen(var value)

    Layout.fillWidth: true
    spacing: 4

    function indexOf(v) {
        for (let i = 0; i < options.length; ++i)
            if (options[i].value === v) return i
        return -1
    }

    Text {
        Layout.fillWidth: true
        text: row.label
        font: Theme.fontBody
        color: row.enabled ? Theme.text : Theme.textDisabled
    }
    AppComboBox {
        objectName: row.objectName.length ? row.objectName + ".combo" : ""
        Layout.fillWidth: true
        Layout.maximumWidth: 220
        label: row.label
        model: row.options
        textRole: "text"
        currentIndex: row.indexOf(row.value)
        onActivated: index => row.chosen(row.options[index].value)
    }
}
