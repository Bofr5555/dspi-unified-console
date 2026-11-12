import QtQuick
import QtQuick.Controls.Basic
import UConsole

// The Settings search field (behaviour.md §9): it filters the page list as
// you type, by each page's title and its rows. A clear button shows while it
// holds text; Down or Return asks to open the first page left (`openFirst`).
// Escape clears it.
TextField {
    id: field
    signal openFirst()

    implicitHeight: Theme.controlHeight + 2
    font: Theme.fontBody
    color: Theme.text
    selectionColor: Theme.selection
    selectedTextColor: Theme.text
    placeholderText: qsTr("Search")
    placeholderTextColor: Theme.textMuted
    leftPadding: 24
    rightPadding: clear.visible ? 24 : 6
    verticalAlignment: TextInput.AlignVCenter

    Keys.onDownPressed: field.openFirst()
    Keys.onReturnPressed: field.openFirst()
    Keys.onEnterPressed: field.openFirst()
    Keys.onEscapePressed: event => {
        if (field.text.length) {
            field.clear()
            event.accepted = true
        } else {
            event.accepted = false
        }
    }

    background: Rectangle {
        radius: Theme.radiusSmall
        color: Theme.control
        border.width: field.activeFocus ? 2 : 1
        border.color: field.activeFocus ? Theme.focusRing : Theme.border
        Icon {
            x: 6
            anchors.verticalCenter: parent.verticalCenter
            name: "search"
            size: 13
            color: Theme.textMuted
        }
    }

    IconButton {
        id: clear
        objectName: "Settings.searchClear"
        visible: field.text.length > 0
        anchors.right: parent.right
        anchors.verticalCenter: parent.verticalCenter
        iconName: "x"
        iconSize: 12
        toolTip: qsTr("Clear Search")
        onClicked: {
            field.clear()
            field.forceActiveFocus()
        }
    }

    Accessible.role: Accessible.EditableText
    Accessible.name: qsTr("Search settings")
}
