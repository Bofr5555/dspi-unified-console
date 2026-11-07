import QtQuick 2.15

// The banners registered through src/Registrar.h, above the graph. A banner file
// is an Item with these two:
//     property bool shown: <its own condition>     (not `visible`: an item inside a
//                                                   hidden host always reads false)
//     implicitHeight: <its height>
// The host sets its width, hides it while `shown` is false, and takes no space
// when none is shown.
Column {
    id: host
    objectName: "bannerHost"
    spacing: 10
    property int shownCount: 0
    visible: shownCount > 0

    Repeater {
        model: registeredBanners
        Loader {
            width: host.width
            source: modelData
            readonly property bool shown: status === Loader.Ready && item.shown === true
            visible: shown
            onShownChanged: host.shownCount += shown ? 1 : -1
        }
    }
}
