import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- The body of a two-column effect window (the Mac's ToolColumns
// with toolColumn()): two equal columns with a line between them, pinned to
// the top, scrolling when the window is shorter than they are.
Flickable {
    id: body
    property alias leftItems: leftColumn.data
    property alias rightItems: rightColumn.data
    readonly property alias leftHeight: leftColumn.implicitHeight
    readonly property alias rightHeight: rightColumn.implicitHeight

    contentHeight: row.implicitHeight + 2 * Theme.spacingLarge
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: AppScrollBar {}

    RowLayout {
        id: row
        y: Theme.spacingLarge
        width: body.width
        spacing: 0

        ColumnLayout {
            id: leftColumn
            Layout.preferredWidth: row.width / 2
            Layout.alignment: Qt.AlignTop
            Layout.leftMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
            spacing: 14
        }
        EffectRule { vertical: true }
        ColumnLayout {
            id: rightColumn
            Layout.preferredWidth: row.width / 2
            Layout.alignment: Qt.AlignTop
            Layout.leftMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
            spacing: 14
        }
    }
}
