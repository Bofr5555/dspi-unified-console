import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP QP7 -- A borderless text button that opens its menu under it (the
// Mac's borderless Menu: "Presets", "Apply preset", the Outputs choice).
// Give it the menu's rows as children of `menu`.
// -- WP DL1 -- In Modern a secondary button (DESIGN-LANGUAGE §7): the control
// tone, no outline, radius 6, its label in text and the chevron muted.
AbstractButton {
    id: button
    default property alias rows: menu.contentData
    readonly property alias menu: menu
    readonly property bool modern: LayoutController.modern   // -- WP DL1 --

    implicitWidth: label.implicitWidth + (modern ? 20 : 18)
    implicitHeight: Theme.controlHeight
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    contentItem: Item {
        Row {
            id: label
            anchors.verticalCenter: parent.verticalCenter
            anchors.horizontalCenter: button.modern ? parent.horizontalCenter : undefined
            spacing: 2
            Text {
                anchors.verticalCenter: parent.verticalCenter
                textFormat: Text.PlainText
                text: button.text
                font: Theme.fontSmall
                color: !button.enabled ? Theme.textDisabled : button.modern ? Theme.text : Theme.accentText
            }
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                name: "chevron-down"
                size: 12
                color: !button.enabled ? Theme.textDisabled : button.modern ? Theme.textMuted : Theme.accentText
            }
        }
    }
    background: Rectangle {
        radius: button.modern ? Theme.radiusControl : Theme.radiusSmall
        color: button.modern ? (button.down ? Theme.controlPressed : button.hovered && button.enabled ? Theme.controlHover : Theme.control)
             : button.hovered && button.enabled ? Theme.controlHover : "transparent"
        opacity: button.modern && !button.enabled ? 0.6 : 1
        border.width: button.visualFocus ? 2 : 0
        border.color: Theme.focusRing
    }
    onClicked: menu.popup(button, 0, button.height)

    AppMenu {
        id: menu
    }

    Accessible.role: Accessible.ButtonMenu
    Accessible.name: text
}
