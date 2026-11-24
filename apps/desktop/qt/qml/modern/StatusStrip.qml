import QtQuick
import UConsole

// -- WP LAY3 -- Under the top bar: only the states that limit what can be done
// (the banners registered as Registrar::Blocking: demo, read-only firmware,
// the installer's lock, a lost hub, a safe apply, an open file, a sender). The
// rest is the notification drawer's.
// -- WP DL5 -- In the strip's own tone (DESIGN-LANGUAGE §14, the mockup's
// banner): each banner on Theme.bannerFill(kind) (an info banner on the calm
// panel tone, INTC), each with a 3 px edge at the left in
// Theme.bannerEdge(kind) and no line under it. The banners are Classic's too,
// where they keep their look: the strip dresses each one as it loads.
BannerHost {
    id: strip
    objectName: "StatusStrip"
    types: Shell.blockingBanners

    // A Banner (it has a kind) gets the strip's dress once
    property var dressed: []
    function dress() {
        for (let i = 0; i < children.length; ++i) {
            const banner = children[i].item
            if (!banner || banner.kind === undefined || dressed.indexOf(banner) >= 0) continue
            dressed.push(banner)
            dressing.createObject(banner, { banner: banner })
        }
    }
    Component.onCompleted: dress()
    onBannerCountChanged: Qt.callLater(dress)
    onBannersShownChanged: dress()

    readonly property Component dressing: Component {
        Item {
            id: dress
            required property Item banner
            objectName: banner.objectName.length ? banner.objectName + ".edge" : ""
            anchors.fill: parent
            // The fill
            Binding {
                target: dress.banner
                property: "color"
                value: Theme.bannerFill(dress.banner.kind)
            }
            // Over the banner's own line under it (Banner.qml, Classic's): the fill again
            Rectangle {
                anchors.bottom: parent.bottom
                width: parent.width
                height: 1
                color: dress.banner.color
            }
            // The edge
            Rectangle {
                width: 3
                height: parent.height
                color: Theme.bannerEdge(dress.banner.kind)
            }
        }
    }
}
