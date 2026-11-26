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
    // -- WP REVDFIX -- (REVD D5) the floating surface of the popovers in
    // Modern (shadow, radius 8, no Classic outline); Classic's as before
    background: FloatingBackground {
        implicitWidth: 240
        color: Theme.menu
        radius: LayoutController.modern ? Theme.radius : Theme.radiusSmall
    }
}
