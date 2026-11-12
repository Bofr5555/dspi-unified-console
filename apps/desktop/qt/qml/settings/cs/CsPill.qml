import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- A card's state (the Mac's status pill): a dot and a word,
// green when good, orange otherwise.
Rectangle {
    id: pill
    property string text
    property bool good

    readonly property color tint: good ? Theme.success : Theme.warning
    implicitWidth: row.implicitWidth + 16
    implicitHeight: row.implicitHeight + 6
    radius: height / 2
    color: Qt.rgba(tint.r, tint.g, tint.b, 0.12)
    RowLayout {
        id: row
        anchors.centerIn: parent
        spacing: 5
        Rectangle {
            implicitWidth: 6
            implicitHeight: 6
            radius: 3
            color: pill.tint
        }
        Text {
            objectName: pill.objectName.length ? pill.objectName + ".text" : ""
            text: pill.text
            font: Qt.font({ pixelSize: Theme.fontCaption.pixelSize, weight: Font.Medium })
            color: pill.tint
        }
    }
    Accessible.role: Accessible.StaticText
    Accessible.name: text
}
