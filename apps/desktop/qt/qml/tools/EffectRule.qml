import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- The thin line between an effect window's sections (the Mac's
// Divider): across a column, or down between two columns (`vertical`).
// -- WP DL1 -- Modern separates by spacing, not lines (DESIGN-LANGUAGE §5):
// there it draws nothing and takes no height, and the column's spacing on
// either side of it is the gap.
Rectangle {
    property bool vertical: false
    Layout.fillWidth: !vertical
    Layout.fillHeight: vertical
    implicitWidth: 1
    implicitHeight: LayoutController.modern && !vertical ? 0 : 1
    color: LayoutController.modern ? "transparent" : Theme.separator
}
