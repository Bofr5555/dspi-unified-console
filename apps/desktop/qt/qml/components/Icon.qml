import QtQuick
import UConsole

// A Lucide (or own) icon by name, tinted through the theme: one SVG serves both
// colour schemes. Decorative unless `accessibleName` is set.
Image {
    id: icon
    property string name
    property color color: Theme.text
    property int size: 16
    property string accessibleName

    width: size
    height: size
    fillMode: Image.PreserveAspectFit
    sourceSize: Qt.size(size * Screen.devicePixelRatio, size * Screen.devicePixelRatio)
    source: name.length ? "image://icon/" + name + "/" + color.toString().slice(1) : ""
    smooth: true

    Accessible.role: Accessible.Graphic
    Accessible.name: accessibleName
    Accessible.ignored: accessibleName.length === 0
}
