import QtQuick
import UConsole

// The banners under the menu bar, top to bottom in their registered order
// (Registrar::Banner). Each banner type decides when it shows.
Column {
    id: host
    objectName: "BannerHost"
    readonly property int bannerCount: repeater.count

    Repeater {
        id: repeater
        model: Shell.banners
        delegate: Loader {
            required property string modelData
            width: host.width
            objectName: "BannerHost." + modelData
            sourceComponent: Qt.createComponent("UConsole", modelData)
            // The banner's own `visible` decides; a hidden one takes no room
            height: item && item.visible ? item.implicitHeight : 0
        }
    }
}
