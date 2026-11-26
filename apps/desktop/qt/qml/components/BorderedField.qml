import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A bordered text or number field (the Mac's bordered fields). `committed`
// fires on Enter and when focus leaves with the text changed; Escape restores
// the text it had when editing began.
//
// -- WP USR1 -- Leaving the field commits what the person typed, and only
// that: `edited` is true from their first key until Enter, Escape or the focus
// leaves. Text set by the app (the value shown again after Enter, a handler of
// a field built on this one, a Binding) is never sent back as an edit: a
// field showed the old value again after Enter, kept it while it had the
// focus, and a click elsewhere then wrote that old value to the device.
// `number(text, unit)` reads a typed number the same way in every field.
//
// -- WP QTTWEAK -- With `selectAllOnFocus`, Tab or a click into the field
// selects all its text, so typing replaces the value; a click in a field that
// already has the focus places the cursor, and a drag selects part of it.
//
// -- WP DL0 -- Modern: no outline, radius 6; the focus ring is the only line.
//
// -- WP INTE -- (REVE C1) Enter writes only typed text too: with nothing typed
// it commits nothing and the field keeps the text it shows (it wrote the shown,
// rounded value back); `enteredUntyped` says so, for an editor that closes on
// Enter (a rename).
TextField {
    id: field
    property string label
    property bool selectAllOnFocus: false
    signal committed(string text)
    signal enteredUntyped() // -- WP INTE --

    property string _start
    property bool _selectOnRelease: false
    // -- WP USR1 -- What the person typed since editing began or since Enter
    readonly property bool edited: _edited
    property bool _edited: false
    property string _typed
    onTextEdited: {
        _typed = text
        _edited = true
    }
    onActiveFocusChanged: {
        if (activeFocus) {
            _start = text
            _edited = false
            if (selectAllOnFocus) {
                selectAll()
                // The press that gave the focus has put the cursor down: select on its release
                _selectOnRelease = focusReason === Qt.MouseFocusReason
            }
        } else {
            _selectOnRelease = false
            // -- WP USR1 -- the typed text, whatever has been put in the field since
            const typed = _typed
            const send = _edited && typed !== _start
            _edited = false
            if (send) committed(typed)
        }
    }
    onReleased: {
        if (_selectOnRelease && selectedText.length === 0) selectAll()
        _selectOnRelease = false
    }
    onAccepted: {
        _start = text
        // -- WP INTE -- (REVE C1) only what was typed is written
        const typed = _edited
        _edited = false   // -- WP USR1 --
        if (typed) committed(text)
        else enteredUntyped()
    }
    Keys.onEscapePressed: {
        _edited = false   // -- WP USR1 --
        text = _start
        focus = false
    }

    // -- WP USR1 -- A typed number in any common form: "-3.5", "-3,5", "−3,5"
    // (the minus sign U+2212, as some keyboards and copied text have it),
    // "+2", " -3.5 dB " with the field's `unit` after it (any case), and a
    // space, a no-break space or a thin space anywhere. One comma is the
    // decimal separator; a comma and a point together are not a number. NaN
    // when it is not a number.
    function number(t, unit) {
        let s = String(t).replace(/[\s\u00a0\u202f\u2009]+/g, "").replace(/[\u2212\u2012\u2013\ufe63\uff0d]/g, "-")
        const u = String(unit || "").replace(/\s+/g, "").toLowerCase()
        if (u.length && s.toLowerCase().endsWith(u)) s = s.slice(0, s.length - u.length)
        if (s.indexOf(",") >= 0) {
            if (s.indexOf(".") >= 0 || s.indexOf(",") !== s.lastIndexOf(",")) return NaN
            s = s.replace(",", ".")
        }
        if (!/^[+-]?(\d+\.?\d*|\.\d+)$/.test(s)) return NaN
        return Number(s)
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
        radius: Theme.radiusControl   // -- WP DL0 --
        color: Theme.control
        border.width: field.activeFocus ? Theme.focusRingWidth : Theme.controlLineWidth
        border.color: field.activeFocus ? Theme.focusRing : Theme.controlLine   // -- WP UB13 --
    }

    Accessible.role: Accessible.EditableText
    Accessible.name: label
}
