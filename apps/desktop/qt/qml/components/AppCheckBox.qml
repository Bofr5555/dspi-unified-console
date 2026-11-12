import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A check box with its label on the right (lists of options, the matrix's
// ENABLE row). Supports the partly checked state for "some of them".
CheckBox {
    id: control
    font: Theme.fontBody
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus
    spacing: 6
    padding: 2

    indicator: Rectangle {
        implicitWidth: 16
        implicitHeight: 16
        x: control.leftPadding
        y: control.topPadding + (control.availableHeight - height) / 2
        radius: 3
        color: control.checkState === Qt.Unchecked ? Theme.control : Theme.accent
        opacity: control.enabled ? 1 : 0.5
        border.width: control.visualFocus ? 2 : 1
        border.color: control.visualFocus ? Theme.focusRing : control.checkState === Qt.Unchecked ? Theme.border : Theme.accent
        Icon {
            anchors.centerIn: parent
            visible: control.checkState === Qt.Checked
            name: "check"
            size: 12
            color: Theme.textOnAccent
        }
        Rectangle {
            anchors.centerIn: parent
            visible: control.checkState === Qt.PartiallyChecked
            width: 8
            height: 2
            color: Theme.textOnAccent
        }
    }
    contentItem: Text {
        leftPadding: control.indicator.width + control.spacing
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
