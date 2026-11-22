import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP QP7 -- A borderless text button that opens its menu under it (the
// Mac's borderless Menu: "Presets", "Apply preset", the Outputs choice).
// Give it the menu's rows as children of `menu`.
AbstractButton {
    id: button
    default property alias rows: menu.contentData
    readonly property alias menu: menu

    implicitWidth: label.implicitWidth + 18
    implicitHeight: Theme.controlHeight
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    contentItem: Item {
        Row {
            id: label
            anchors.verticalCenter: parent.verticalCenter
            spacing: 2
            Text {
                anchors.verticalCenter: parent.verticalCenter
                textFormat: Text.PlainText
                text: button.text
                font: Theme.fontSmall
                color: button.enabled ? Theme.accentText : Theme.textDisabled
            }
            Icon {
                anchors.verticalCenter: parent.verticalCenter
                name: "chevron-down"
                size: 12
                color: button.enabled ? Theme.accentText : Theme.textDisabled
            }
        }
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        color: button.hovered && button.enabled ? Theme.controlHover : "transparent"
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
