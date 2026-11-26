import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// A Settings page's frame (the Mac's grouped Form): a scrolling column of
// SettingsGroup sections, opened at its top.
// -- WP DL7 -- Modern: the column starts at the left margin and stops at
// Theme.contentMaxWidth (760), sections Theme.spacingSection apart; Classic
// keeps its centred 640 px column.
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
        x: LayoutController.modern ? Theme.spacingLarge : Math.max(Theme.spacingLarge, (page.width - width) / 2)
        y: Theme.spacingLarge
        width: Math.min(page.width - 2 * Theme.spacingLarge, LayoutController.modern ? Theme.contentMaxWidth : page.maxContentWidth)
        spacing: LayoutController.modern ? Theme.spacingSection : Theme.spacingLarge
    }
}
