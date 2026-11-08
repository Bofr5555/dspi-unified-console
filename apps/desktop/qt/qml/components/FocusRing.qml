import QtQuick 2.15

// WP QD22: the keyboard focus ring, just outside its parent while `shown`: by
// default while the parent has the active focus, which a custom control only
// gets from the keyboard; a Quick control passes its visualFocus, so a click
// shows none. It takes no room, so the layout is as it was. (FocusFollower
// keeps the focused control in view.)
Rectangle {
    id: ring
    property Item target: parent
    property bool shown: target ? target.activeFocus : false
    property real gap: 2
    anchors.fill: parent
    anchors.margins: -gap
    radius: 4 + gap
    color: "transparent"
    border.width: 2
    border.color: isMacOS ? MacColors.keyboardFocusIndicator : "#0a7cff"
    visible: shown
    z: 1000
    Accessible.ignored: true
}
