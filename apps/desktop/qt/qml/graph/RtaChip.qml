import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP QP9 -- A channel of the spectrum's selection (the Mac's chip): a small
// dot in the channel's colour and its name; the colour stays a tag, the chip
// itself takes only a light tint while on.
AbstractButton {
    id: chip
    property color tint: Theme.accent
    property bool on: false

    implicitHeight: 24
    implicitWidth: 120
    hoverEnabled: true
    focusPolicy: Qt.TabFocus

    background: Rectangle {
        radius: 6
        color: chip.on ? Theme.selection : chip.hovered ? Theme.controlHover : Theme.control
        border.width: chip.visualFocus ? 2 : 1
        border.color: chip.visualFocus ? Theme.focusRing : Theme.separator
        Behavior on color { ColorAnimation { duration: 120 } }
    }
    contentItem: Item {
        implicitWidth: dot.width + 6 + name.implicitWidth
        implicitHeight: name.implicitHeight
        Rectangle {
            id: dot
            anchors { left: parent.left; leftMargin: 8; verticalCenter: parent.verticalCenter }
            width: 7
            height: 7
            radius: 3.5
            color: chip.tint
            opacity: chip.on ? 1 : 0.4
        }
        Text {
            id: name
            anchors { left: dot.right; leftMargin: 6; right: parent.right; rightMargin: 8; verticalCenter: parent.verticalCenter }
            text: chip.text
            font: Qt.font({ pixelSize: Theme.fontSmall.pixelSize, weight: chip.on ? Font.DemiBold : Font.Normal })
            color: chip.on ? Theme.text : Theme.textMuted
            elide: Text.ElideRight
        }
    }
    HelpTip {
        text: chip.text
        visible: chip.hovered
    }
    Accessible.role: Accessible.CheckBox
    Accessible.name: text
    Accessible.checkable: true
    Accessible.checked: on
}
