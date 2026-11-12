import QtQuick
import UConsole

// Another console holds the hub's device lock: shown for as long as it does
// (the hub's device record names the holder; this app's own lock is not
// another's). Changes made meanwhile are refused by the hub.
Banner {
    objectName: "LockNotice"
    kind: "warning"
    iconName: "lock"
    text: LinkController.lockNotice
    visible: LinkController.lockNotice.length > 0
}
