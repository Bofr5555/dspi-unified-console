import QtCore
import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A tool window (the Mac's NSWindow tools): the system's frame, the theme's
// background, the menu table's shortcuts, and its size and place remembered
// under `windowId` (Wayland ignores the place). Default
// sizes are the Mac's content sizes (mockups).
ApplicationWindow {
    id: window
    required property string windowId
    property int defaultWidth: 600
    property int defaultHeight: 400
    // -- WP QTSENDER -- The menu action that opened this window (Shell sets
    // it), and whether that action does nothing on the open device (the alink
    // sender): the content is then greyed under a banner that says why
    property string actionId: ""
    readonly property bool senderLocked: actionId.length > 0 && DeviceInfoController.sender
                                         && !DeviceInfoController.senderKeeps(actionId)
    contentItem.enabled: !senderLocked
    header: Banner {
        objectName: "ToolWindow.senderBanner"
        visible: window.senderLocked
        kind: "info"
        iconName: "radio"
        text: qsTr("This is an audio link sender. It has no DSP, so this window has nothing to show or change.")
    }

    width: defaultWidth
    height: defaultHeight
    color: Theme.window
    palette.window: Theme.window
    palette.windowText: Theme.text
    palette.base: Theme.control
    palette.text: Theme.text
    palette.button: Theme.control
    palette.buttonText: Theme.text
    palette.highlight: Theme.accent
    palette.highlightedText: Theme.textOnAccent

    Settings {
        category: "windows/" + window.windowId
        property alias x: window.x
        property alias y: window.y
        property alias width: window.width
        property alias height: window.height
    }

    Shortcut {
        sequences: [StandardKey.Close]
        onActivated: window.close()
    }

    // The menu table's shortcuts work here too
    TableShortcuts {
        window: window
    }
}
