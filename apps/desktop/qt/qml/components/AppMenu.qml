import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A menu in the app's look (menu bar, context menus).
Menu {
    id: menu
    font: Theme.fontBody
    delegate: AppMenuItem {}
    // A small inset above the first row and below the last, as the system's menus have
    topPadding: Theme.radiusSmall
    bottomPadding: Theme.radiusSmall
    background: Rectangle {
        implicitWidth: 240
        color: Theme.menu
        radius: Theme.radiusSmall
        border.width: 1
        border.color: Theme.border
    }
}
