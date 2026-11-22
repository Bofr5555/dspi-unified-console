import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A bordered text or number field (the Mac's bordered fields). `committed`
// fires on Enter and when focus leaves with the text changed; Escape restores
// the text it had when editing began.
//
// -- WP QTTWEAK -- With `selectAllOnFocus`, Tab or a click into the field
// selects all its text, so typing replaces the value; a click in a field that
// already has the focus places the cursor, and a drag selects part of it.
TextField {
    id: field
    property string label
    property bool selectAllOnFocus: false
    signal committed(string text)

    property string _start
    property bool _selectOnRelease: false
    onActiveFocusChanged: {
        if (activeFocus) {
            _start = text
            if (selectAllOnFocus) {
                selectAll()
                // The press that gave the focus has put the cursor down: select on its release
                _selectOnRelease = focusReason === Qt.MouseFocusReason
            }
        } else {
            _selectOnRelease = false
            if (text !== _start) committed(text)
        }
    }
    onReleased: {
        if (_selectOnRelease && selectedText.length === 0) selectAll()
        _selectOnRelease = false
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
    selectionColor: Theme.textSelection
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
