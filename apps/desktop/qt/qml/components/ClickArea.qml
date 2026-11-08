import QtQuick 2.15

// WP QD22: a MouseArea that is a control for the keyboard and a screen reader
// too. Tab reaches it; Space, Return and Enter work as a click, and so does
// the screen reader's press; a ring shows the keyboard focus. `activated` is
// the one handler for all of them (byKeyboard tells a key or the screen
// reader from a click). Give it an Accessible.name, and a role where it is
// not a button (Accessible.checkable/checked for a toggle).
MouseArea {
    id: area
    signal activated()
    property alias focusRadius: ring.radius
    property bool byKeyboard: false
    function keyActivate() {
        if (!enabled) return
        byKeyboard = true
        activated()
        byKeyboard = false
    }
    activeFocusOnTab: enabled
    Accessible.role: Accessible.Button
    Accessible.focusable: true
    Accessible.onPressAction: area.keyActivate()
    Accessible.onToggleAction: area.keyActivate()
    onClicked: function (mouse) { if (mouse.button === Qt.LeftButton) area.activated() }
    // A window's own Space (the Signal Generator's start) waits while a control has the focus
    Keys.onShortcutOverride: function (event) {
        if (event.key === Qt.Key_Space || event.key === Qt.Key_Return || event.key === Qt.Key_Enter) event.accepted = true
    }
    Keys.onSpacePressed: keyActivate()
    Keys.onReturnPressed: keyActivate()
    Keys.onEnterPressed: keyActivate()
    FocusRing { id: ring }
}
