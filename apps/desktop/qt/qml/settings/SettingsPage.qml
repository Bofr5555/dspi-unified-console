import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// A Settings page's frame (the Mac's grouped Form): a scrolling column of
// SettingsGroup sections, opened at its top.
Flickable {
    id: page
    default property alias content: column.data
    property int maxContentWidth: 640

    clip: true
    contentWidth: width
    contentHeight: column.implicitHeight + 2 * Theme.spacingLarge
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: AppScrollBar {}

    ColumnLayout {
        id: column
        x: Math.max(Theme.spacingLarge, (page.width - width) / 2)
        y: Theme.spacingLarge
        width: Math.min(page.width - 2 * Theme.spacingLarge, page.maxContentWidth)
        spacing: Theme.spacingLarge
    }
}
