import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A small rounded label in a channel's colour ("IN1", "OUT3"). Checkable
// pills are toggles (the sidebar's curve visibility): filled when checked,
// outlined when not.
AbstractButton {
    id: pill
    property color tint: Theme.accent
    property string accessibleName: text

    implicitWidth: label.implicitWidth + 12
    implicitHeight: 16
    hoverEnabled: true
    focusPolicy: checkable ? Qt.TabFocus : Qt.NoFocus

    contentItem: Text {
        id: label
        textFormat: Text.PlainText
        text: pill.text
        font: Theme.fontCaption
        color: !pill.checkable || pill.checked ? Theme.textOnTint : Theme.text
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }
    background: Rectangle {
        radius: height / 2
        color: !pill.checkable || pill.checked ? pill.tint : "transparent"
        border.width: pill.visualFocus ? 2 : 1
        border.color: pill.visualFocus ? Theme.focusRing : pill.tint
        opacity: pill.enabled ? 1 : 0.4
    }

    Accessible.role: checkable ? Accessible.CheckBox : Accessible.StaticText
    Accessible.name: accessibleName
    Accessible.checkable: checkable
    Accessible.checked: checked
}
