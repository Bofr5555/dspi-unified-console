import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A menu in the app's look (menu bar, context menus).
Menu {
    id: menu
    font: Theme.fontBody
    delegate: AppMenuItem {}
    background: Rectangle {
        implicitWidth: 240
        color: Theme.menu
        radius: Theme.radiusSmall
        border.width: 1
        border.color: Theme.border
    }
}
