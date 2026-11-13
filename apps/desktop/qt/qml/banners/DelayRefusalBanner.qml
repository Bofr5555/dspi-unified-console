import QtQuick
import UConsole

// -- WP QP16 -- A delay the core refused (behaviour.md §15), in its own words:
// one line, until the next accepted delay, a dismissal or another device.
Banner {
    objectName: "DelayRefusalBanner"
    kind: "warning"
    iconName: "timer"
    text: ReadOnlyController.delayRefusal
    visible: text.length > 0
    dismissable: true
    onDismissed: ReadOnlyController.dismissDelayRefusal()
}
