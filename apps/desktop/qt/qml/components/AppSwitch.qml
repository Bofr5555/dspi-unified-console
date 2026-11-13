import QtQuick
import QtQuick.Controls.Basic
import UConsole

// An on/off switch with its label on the left (the Mac's Toggle in a form
// row). `toggled` fires on a click or Space, not on a binding change.
Switch {
    id: control
    font: Theme.fontBody
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    spacing: Theme.spacingSmall
    padding: 2

    indicator: Rectangle {
        implicitWidth: 32
        implicitHeight: 18
        x: control.width - width - control.rightPadding
        y: control.topPadding + (control.availableHeight - height) / 2
        radius: height / 2
        color: control.checked ? Theme.accent : Theme.border
        opacity: control.enabled ? 1 : 0.5
        border.width: control.visualFocus ? 2 : 0
        border.color: Theme.focusRing
        Rectangle {
            x: control.checked ? parent.width - width - 2 : 2
            anchors.verticalCenter: parent.verticalCenter
            width: 14
            height: 14
            radius: 7
            color: Theme.textOnAccent
            Behavior on x { NumberAnimation { duration: 120 } }
        }
    }
    contentItem: Text {
        rightPadding: control.indicator.width + control.spacing
        text: control.text
        font: control.font
        color: control.enabled ? Theme.text : Theme.textDisabled
        verticalAlignment: Text.AlignVCenter
    }

    Accessible.role: Accessible.CheckBox
    Accessible.name: text
    Accessible.checkable: true
    Accessible.checked: checked
}
