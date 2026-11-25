import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP UA1 -- (UXP A1, C5) The last row of an Equaliser table: the outputs
// nothing is routed to, folded under one row ("5 unused" with a chevron); a
// click (`clicked`) opens them in their places, as "3 · None", and folds them again.
AbstractButton {
    id: fold
    property int count: 0
    property bool open: false

    visible: count > 0
    height: visible ? Theme.rowHeight : 0
    hoverEnabled: true
    focusPolicy: Qt.TabFocus

    background: Rectangle {
        color: fold.hovered ? Theme.controlHover : "transparent"
        opacity: fold.hovered ? 0.6 : 1
        border.width: fold.visualFocus ? Theme.focusRingWidth : 0
        border.color: Theme.focusRing
    }
    contentItem: RowLayout {
        spacing: 6
        Item { Layout.preferredWidth: Theme.spacing - 6 }
        Icon {
            name: fold.open ? "chevron-down" : "chevron-right"
            size: 14
            color: Theme.textMuted
        }
        Text {
            objectName: fold.objectName + ".text"
            Layout.fillWidth: true
            textFormat: Text.PlainText
            //: The outputs nothing is routed to, folded in a table
            text: fold.count === 1 ? qsTr("1 unused") : qsTr("%1 unused").arg(fold.count)
            font: Theme.fontBody
            color: Theme.textMuted
            elide: Text.ElideRight
        }
    }

    Accessible.role: Accessible.Button
    Accessible.name: fold.open
        ? (fold.count === 1 ? qsTr("Hide 1 unused output") : qsTr("Hide %1 unused outputs").arg(fold.count))
        : (fold.count === 1 ? qsTr("Show 1 unused output") : qsTr("Show %1 unused outputs").arg(fold.count))
    Accessible.onPressAction: fold.clicked()
}
