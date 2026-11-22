import QtQuick
import UConsole

// -- WP QP12 -- A card's state (the Mac's status pill): a dot and a word on a
// faint capsule of the same tint, green when good, orange otherwise.
Item {
    id: pill
    property string text
    property bool good

    readonly property color tint: good ? Theme.success : Theme.warning
    implicitWidth: word.x + word.implicitWidth + 8
    implicitHeight: 18

    Rectangle {
        width: pill.width
        height: pill.height
        radius: pill.height / 2
        color: pill.tint
        opacity: 0.12
    }
    Rectangle {
        id: dot
        x: 8
        y: (pill.height - 6) / 2
        width: 6
        height: 6
        radius: 3
        color: pill.tint
    }
    Text {
        id: word
        objectName: pill.objectName.length ? pill.objectName + ".text" : ""
        x: dot.x + dot.width + 5
        y: (pill.height - implicitHeight) / 2
        textFormat: Text.PlainText
        text: pill.text
        font: Qt.font({ pixelSize: Theme.fontCaption.pixelSize, weight: Font.Medium })
        color: pill.tint
    }
    Accessible.role: Accessible.StaticText
    Accessible.name: text
}
