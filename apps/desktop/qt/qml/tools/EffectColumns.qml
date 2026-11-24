import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- The body of a two-column effect window (the Mac's ToolColumns
// with toolColumn()): two equal columns with a line between them, pinned to
// the top, scrolling when the window is shorter than they are.
// -- WP DL1 -- Modern: no line between them, the section gap (24) under the
// intro row, and the pair no wider than `modernMaxWidth` (1200), from the
// left of the page.
Flickable {
    id: body
    property alias leftItems: leftColumn.data
    property alias rightItems: rightColumn.data
    readonly property alias leftHeight: leftColumn.implicitHeight
    readonly property alias rightHeight: rightColumn.implicitHeight
    property int modernMaxWidth: 1200   // -- WP DL1 --
    readonly property int topGap: LayoutController.modern ? Theme.spacingSection : Theme.spacingLarge

    contentHeight: row.implicitHeight + topGap + Theme.spacingLarge
    clip: true
    boundsBehavior: Flickable.StopAtBounds
    ScrollBar.vertical: AppScrollBar {}

    // Its panel's `shown`: opened again, it starts at the top, as the Mac's does
    required property bool shown   // -- WP LAY1a --
    onShownChanged: if (shown) contentY = 0

    RowLayout {
        id: row
        y: body.topGap
        width: LayoutController.modern ? Math.min(body.width, body.modernMaxWidth) : body.width
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
