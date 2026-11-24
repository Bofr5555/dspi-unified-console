import QtQuick
import QtQuick.Controls.Basic
import UConsole

// One segment of an AppTabBar.
// -- WP REVDFIX -- (REVD D2) Modern: ToolSegments' segment (selection tone,
// accent text); Classic keeps the accent fill.
TabButton {
    id: tab
    readonly property bool modern: LayoutController.modern
    implicitHeight: modern ? Theme.controlHeight - 4 : Theme.controlHeight
    font: modern ? Theme.fontSmall : Theme.fontBody
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    contentItem: Text {
        textFormat: Text.PlainText
        text: tab.text
        font: tab.font
        color: tab.modern ? (!tab.enabled ? Theme.textDisabled : tab.checked ? Theme.accentText : Theme.text)
             : tab.checked ? Theme.textOnAccent : tab.enabled ? Theme.text : Theme.textDisabled
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
    }
    background: Rectangle {
        radius: tab.modern ? Theme.radiusControl - 2 : Theme.radiusSmall
        color: tab.checked ? (tab.modern ? Theme.selection : Theme.accent)
             : tab.hovered && (!tab.modern || tab.enabled) ? Theme.controlHover : "transparent"
        border.width: tab.visualFocus ? 2 : 0
        border.color: Theme.focusRing
    }

    Accessible.role: Accessible.PageTab
    Accessible.name: text
    Accessible.selected: checked
}
