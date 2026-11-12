import QtQuick
import QtQuick.Controls.Basic
import UConsole

// The sheet while a configuration is written to the device
// (ConfigurationProgressSheet): several hundred transfers, so the window says
// what it is doing instead of looking stuck. The core writes it in one call,
// so the bar moves without a fraction.
Popup {
    id: sheet
    objectName: "ImportProgress"
    modal: true
    closePolicy: Popup.NoAutoClose
    readonly property real hostWidth: parent ? parent.width : 480
    width: 328
    x: (hostWidth - width) / 2
    y: Theme.spacingLarge
    padding: 24

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.color: Theme.border
    }

    contentItem: Column {
        spacing: Theme.spacing
        Accessible.role: Accessible.ProgressBar
        Accessible.name: label.text
        Text {
            id: label
            text: qsTr("Writing settings to the device...")
            font: Theme.fontBody
            color: Theme.text
        }
        // An indeterminate bar: a block sliding along the track
        Rectangle {
            id: track
            width: 280
            height: 6
            radius: 3
            color: Theme.control
            border.color: Theme.border
            clip: true
            Rectangle {
                width: track.width / 3
                height: track.height
                radius: 3
                color: Theme.accent
                NumberAnimation on x {
                    running: sheet.visible
                    loops: Animation.Infinite
                    from: -track.width / 3
                    to: track.width
                    duration: 1200
                }
            }
        }
    }
}
