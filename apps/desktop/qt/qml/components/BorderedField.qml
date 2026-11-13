import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A bordered text or number field (the Mac's bordered fields). `committed`
// fires on Enter and when focus leaves with the text changed; Escape restores
// the text it had when editing began.
TextField {
    id: field
    property string label
    signal committed(string text)

    property string _start
    onActiveFocusChanged: {
        if (activeFocus) _start = text
        else if (text !== _start) committed(text)
    }
    onAccepted: {
        _start = text
        committed(text)
    }
    Keys.onEscapePressed: {
        text = _start
        focus = false
    }

    implicitWidth: 120
    implicitHeight: Theme.controlHeight
    font: Theme.fontBody
    color: enabled ? Theme.text : Theme.textDisabled
    selectionColor: Theme.selection
    selectedTextColor: Theme.text
    placeholderTextColor: Theme.textMuted
    leftPadding: 6
    rightPadding: 6
    topPadding: 2
    bottomPadding: 2
    verticalAlignment: TextInput.AlignVCenter
    background: Rectangle {
        radius: Theme.radiusSmall
        color: Theme.control
        border.width: field.activeFocus ? 2 : 1
        border.color: field.activeFocus ? Theme.focusRing : Theme.border
    }

    Accessible.role: Accessible.EditableText
    Accessible.name: label
}
