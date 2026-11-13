import QtQuick
import UConsole

// -- WP QP12 -- A component type's badge (the Mac's csTypeBadge): a rounded
// square in the type's tint with its glyph.
Rectangle {
    id: badge
    property int type
    property int size: 32

    implicitWidth: size
    implicitHeight: size
    radius: size * 0.24
    color: Theme.csTypeTint(type)
    Icon {
        anchors.centerIn: parent
        name: CsController.typeIcon(badge.type)
        size: Math.round(badge.size * 0.5)
        color: Theme.textOnAccent
    }
    Accessible.role: Accessible.Graphic
    Accessible.name: CsController.typeName(badge.type)
}
