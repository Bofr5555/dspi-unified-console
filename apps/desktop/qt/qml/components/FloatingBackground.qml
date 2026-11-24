import QtQuick
import UConsole

// -- INTC (DL5 fold) -- The surface of something floating over the window, a
// popover or a menu (DESIGN-LANGUAGE §5). In Modern: the panel tone, radius
// 8, a shadow from the DL0 tokens (shadowColor, shadowOffset, shadowBlur) as a
// few translucent rounded layers (QtQuick.Effects is not in
// packaging/qt-modules.txt), and a 1 px edge in the dark scheme only. A
// Popup's background may draw outside its bounds, so the shadow shows. In
// Classic: the panel with its 1 px border, as before.
//
//     PopoverPanel { background: FloatingBackground { Accessible.name: title } }
Item {
    id: surface
    property color color: Theme.panel
    property real radius: Theme.radius
    readonly property bool floating: LayoutController.modern

    Repeater {
        model: surface.floating ? 4 : 0
        Rectangle {
            required property int index
            readonly property real spread: Theme.shadowBlur * (index + 1) / 8
            x: -spread
            y: Theme.shadowOffset * (index + 1) / 4 - spread
            width: surface.width + 2 * spread
            height: surface.height + 2 * spread
            radius: surface.radius + spread
            color: Qt.rgba(Theme.shadowColor.r, Theme.shadowColor.g, Theme.shadowColor.b, Theme.shadowColor.a / 6)
        }
    }
    Rectangle {
        objectName: "FloatingBackground.surface"
        anchors.fill: parent
        radius: surface.radius
        color: surface.color
        border.width: !surface.floating ? 1 : Theme.dark ? 1 : 0
        border.color: surface.floating ? Theme.separator : Theme.border
    }
}
