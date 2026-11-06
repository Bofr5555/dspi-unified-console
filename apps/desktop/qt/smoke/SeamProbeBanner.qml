import QtQuick 2.15

// WP QD0: a banner registered through the seam, driven by its registered controller.
Rectangle {
    property bool shown: seamProbe.bannerShown
    implicitHeight: 44
    radius: 8
    color: "#55aa55"
    Text { anchors.centerIn: parent; color: "white"; text: "Seam probe banner" }
}
