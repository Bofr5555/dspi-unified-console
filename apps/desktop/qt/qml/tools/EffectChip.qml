import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- One output, pair or channel of an effect's mask (the Mac's
// channel chips): filled with the accent while it is in the mask, its name
// in the tooltip.
AbstractButton {
    id: chip
    property bool on: false
    property string toolTip

    Layout.fillWidth: true
    Layout.minimumWidth: 28
    implicitHeight: 26
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    checkable: false

    contentItem: Text {
        text: chip.text
        font: Theme.fontSmall
        color: chip.on ? Theme.textOnAccent : chip.enabled ? Theme.textMuted : Theme.textDisabled
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
    background: Rectangle {
        radius: 6
        color: chip.on ? Theme.accent : chip.hovered && chip.enabled ? Theme.controlHover : Theme.control
        opacity: chip.enabled ? 1 : 0.6
        border.width: chip.visualFocus ? 2 : chip.on ? 0 : 1
        border.color: chip.visualFocus ? Theme.focusRing : Theme.border
        Behavior on color { ColorAnimation { duration: 120 } }
    }

    HelpTip {
        text: chip.toolTip
        visible: chip.hovered && chip.toolTip.length > 0
    }

    Accessible.role: Accessible.CheckBox
    Accessible.name: toolTip.length ? text + ", " + toolTip : text
    Accessible.checkable: true
    Accessible.checked: on
}
