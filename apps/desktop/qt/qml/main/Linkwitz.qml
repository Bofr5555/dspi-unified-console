import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP QP2 -- The Linkwitz Transform panel in a popover from its band row;
// closing it drops the draft. -- WP LAY2 -- The content, with the draft and
// Apply, is LinkwitzContent, which Modern's inspector shows as well.
PopoverPanel {
    id: panel
    property int channel: -1
    property int band: 0

    title: qsTr("Linkwitz Transform")
    width: 320
    padding: Theme.spacingLarge
    onAboutToShow: content.active = true
    onClosed: content.active = false

    contentItem: LinkwitzContent {
        id: content
        channel: panel.channel
        band: panel.band
    }
}
