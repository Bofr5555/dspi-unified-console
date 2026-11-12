import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- The thin line between an effect window's sections (the Mac's
// Divider): across a column, or down between two columns (`vertical`).
Rectangle {
    property bool vertical: false
    Layout.fillWidth: !vertical
    Layout.fillHeight: vertical
    implicitWidth: 1
    implicitHeight: 1
    color: Theme.separator
}
