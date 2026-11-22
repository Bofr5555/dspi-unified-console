import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// Device > Devices...: every DSPi this app can open, in one list (the Mac's
// device picker and its iOS Devices sheet, with the Networking page's "Other
// Hubs"). USB and the demo first, then DSPi Link: the session's state in the
// core's words, the hub's devices, the hubs found on this network and a field
// for a hub's address. -- WP LAY2 -- The content is DevicesContent, which
// Modern shows as well.
PopoverPanel {
    id: popup
    objectName: "DevicesPopup"
    title: qsTr("Devices")

    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 600
    width: Math.min(460, hostWidth - 2 * Theme.spacingLarge)
    height: Math.min(implicitHeight, hostHeight - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: (parent && parent.topInset !== undefined ? parent.topInset : 0) + Theme.spacingSmall

    onOpened: content.takeFocus()
    onClosed: DevicesController.listShown = false

    contentItem: DevicesContent {
        id: content
        title: popup.title
        onDone: popup.close()
    }
}
