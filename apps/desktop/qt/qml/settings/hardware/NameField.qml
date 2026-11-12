import QtQuick
import QtQuick.Layouts
import UConsole

// A name held for the save bar (the Mac's DraftNameField): a dot while it is
// not saved, the field, and a button that empties it. Typing goes to `edit`,
// which returns the cleaned text the device will store.
RowLayout {
    id: row
    property string text
    property bool edited
    property string clearHelp
    property string label
    property var edit: function (t) { return t }

    spacing: 6
    Rectangle {
        implicitWidth: 6
        implicitHeight: 6
        radius: 3
        color: Theme.warning
        opacity: row.edited ? 1 : 0
        HoverHandler { id: dotHover }
        HelpTip {
            visible: dotHover.hovered && row.edited
            text: qsTr("Not saved yet")
        }
    }
    BorderedField {
        id: field
        objectName: row.objectName.length ? row.objectName + ".field" : ""
        implicitWidth: 190
        label: row.label
        placeholderText: qsTr("Add a name")
        text: row.text
        onTextEdited: {
            const clean = row.edit(text)
            if (clean !== text) text = clean
        }
        onAccepted: focus = false
    }
    IconButton {
        objectName: row.objectName.length ? row.objectName + ".clear" : ""
        iconName: "circle-x"
        toolTip: row.clearHelp
        opacity: field.text.length > 0 ? 1 : 0
        enabled: field.text.length > 0
        onClicked: {
            field.text = ""
            row.edit("")
        }
    }
    // A rename from elsewhere, a revert or a device switch
    onTextChanged: if (!field.activeFocus && field.text !== text) field.text = text
}
