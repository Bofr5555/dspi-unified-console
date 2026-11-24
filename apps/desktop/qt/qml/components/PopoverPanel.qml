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
    // A Popup is not an Item: its background carries the accessible role and name
    // -- INTC (DL5 fold) -- a floating surface in Modern (a shadow, no outline)
    background: FloatingBackground {
        Accessible.role: Accessible.Dialog
        Accessible.name: popover.title
    }
}
