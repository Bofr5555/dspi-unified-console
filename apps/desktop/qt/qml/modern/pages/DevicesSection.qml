import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- Devices: the list of every DSPi the app can open (DevicesContent,
// which the device switcher shows as well). ModernShell keeps the list live
// while the page shows. WP LAY5b makes it This computer and Hubs.
Item {
    id: page
    objectName: "DevicesSection"
    property bool shown: false
    property Item host: null

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacingLarge
        DevicesContent {
            Layout.alignment: Qt.AlignHCenter
            Layout.maximumWidth: 640
            Layout.fillHeight: true
            onDone: if (page.host) page.host.back()
        }
    }
}
