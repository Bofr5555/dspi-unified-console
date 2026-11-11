import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A popover anchored to a control (the Mac's popovers): a panel that closes on
// Escape or a press outside it. Position it with x/y relative to its parent.
Popup {
    id: popover
    property string title

    padding: Theme.spacing
    modal: false
    focus: true
    closePolicy: Popup.CloseOnEscape | Popup.CloseOnPressOutsideParent
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.width: 1
        border.color: Theme.border
    }

    Accessible.role: Accessible.Dialog
    Accessible.name: title
}
