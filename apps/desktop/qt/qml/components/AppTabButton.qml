import QtQuick
import QtQuick.Controls.Basic
import UConsole

// One segment of an AppTabBar.
TabButton {
    id: tab
    implicitHeight: Theme.controlHeight
    font: Theme.fontBody
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    contentItem: Text {
        text: tab.text
        font: tab.font
        color: tab.checked ? Theme.textOnAccent : tab.enabled ? Theme.text : Theme.textDisabled
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        color: tab.checked ? Theme.accent : tab.hovered ? Theme.controlHover : "transparent"
        border.width: tab.visualFocus ? 2 : 0
        border.color: Theme.focusRing
    }

    Accessible.role: Accessible.PageTab
    Accessible.name: text
    Accessible.selected: checked
}
