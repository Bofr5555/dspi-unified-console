import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A borderless icon button with a tooltip. `secondaryClicked` is the
// right-click (the quick strip opens a tool with it). The tooltip text is its
// accessible name.
AbstractButton {
    id: button
    property string iconName
    property string toolTip
    property color iconColor: checked ? Theme.accentText : Theme.text
    property int iconSize: 16
    signal secondaryClicked()

    implicitWidth: iconSize + 12
    implicitHeight: iconSize + 12
    hoverEnabled: true
    focusPolicy: Qt.TabFocus

    contentItem: Item {
        Icon {
            anchors.centerIn: parent
            name: button.iconName
            size: button.iconSize
            color: button.enabled ? button.iconColor : Theme.textDisabled
        }
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        color: button.down ? Theme.controlPressed : button.hovered ? Theme.controlHover : "transparent"
        border.width: button.visualFocus ? 2 : 0
        border.color: Theme.focusRing
    }

    TapHandler {
        acceptedButtons: Qt.RightButton
        onTapped: button.secondaryClicked()
    }

    HelpTip {
        text: button.toolTip
        visible: button.hovered && button.toolTip.length > 0
    }

    Accessible.role: Accessible.Button
    Accessible.name: toolTip
    Accessible.onPressAction: button.clicked()
}
