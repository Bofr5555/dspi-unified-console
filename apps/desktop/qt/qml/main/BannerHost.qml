import QtQuick
import UConsole

// The banners under the menu bar, top to bottom in their registered order
// (Registrar::Banner). Each banner type decides when it shows.
Column {
    id: host
    objectName: "BannerHost"
    readonly property int bannerCount: repeater.count
    // -- WP LAY3 -- Which of them; Modern's status strip and drawer show a part each
    property var types: Shell.banners
    // How many show now
    property int bannersShown: 0
    function recount() {
        let n = 0
        for (let i = 0; i < repeater.count; ++i) {
            const l = repeater.itemAt(i)
            if (l && l.bannerShown) ++n
        }
        bannersShown = n
    }

    Repeater {
        id: repeater
        model: host.types
        onCountChanged: host.recount()
        delegate: Loader {
            required property string modelData
            width: host.width
            objectName: "BannerHost." + modelData
            sourceComponent: Qt.createComponent("UConsole", modelData)
            // The banner's own `visible` decides; a hidden one takes no room
            height: item && item.visible ? item.implicitHeight : 0
            readonly property bool bannerShown: item !== null && item.visible   // -- WP LAY3 --
            onBannerShownChanged: host.recount()
        }
    }
}
