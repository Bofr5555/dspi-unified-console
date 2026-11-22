import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP QP8 -- An output limiter's settings in a popover over the limiter cell
// of the output card, 320 wide. -- WP LAY2 -- The content is
// LimiterSettingsContent, which Modern's inspector shows as well.
PopoverPanel {
    id: popover
    property int output: 0

    objectName: "LimiterSettings"
    title: qsTr("Output Limiter")
    width: 320
    margins: Theme.spacing
    padding: 16

    onOpened: content.active = true
    onClosed: content.active = false

    contentItem: LimiterSettingsContent {
        id: content
        output: popover.output
    }
}
