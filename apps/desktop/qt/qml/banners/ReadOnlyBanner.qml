import QtQuick
import UConsole

// -- WP QP16 -- Read-only mode (behaviour.md §14.1): the device's firmware
// speaks an older or newer wire format than this app writes. Its settings
// show, nothing is written; the button opens the firmware window.
Banner {
    objectName: "ReadOnlyBanner"
    kind: "warning"
    iconName: "lock"
    text: ReadOnlyController.text
    visible: ReadOnlyController.active

    AppButton {
        objectName: "ReadOnlyBanner.action"
        visible: ReadOnlyController.actionShown
        text: ReadOnlyController.actionText
        onClicked: ReadOnlyController.openDetails()
    }
}
