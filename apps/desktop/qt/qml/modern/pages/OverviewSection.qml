import QtQuick
import UConsole

// -- WP LAY3 -- Overview: the dashboard (WP LAY5a adds the active effects).
// Choosing a card's channel opens it on the Equaliser page.
Item {
    id: page
    objectName: "OverviewSection"
    property bool shown: false
    property Item host: null

    // The device whose overview this is: another one's selection (kept per
    // device) is no card that was chosen here
    property string device: ""
    readonly property string activeDevice: DeviceRegistry.activeId
    onActiveDeviceChanged: Qt.callLater(selectNone)

    // Nothing is selected here: the cards choose
    onShownChanged: if (shown) selectNone()
    function selectNone() {
        if (!shown) return
        device = DeviceRegistry.activeId
        SidebarController.select(-1)
    }
    Connections {
        target: SidebarController
        enabled: page.shown
        function onSelectionChanged() {
            if (SidebarController.selectedChannel < 0) return
            if (page.device !== DeviceRegistry.activeId) Qt.callLater(page.selectNone)
            else if (page.host) page.host.go("equaliser", "peq")
        }
    }

    Text {
        anchors.centerIn: parent
        visible: !DeviceModel.connected
        text: qsTr("No Device Connected")
        font: Theme.fontTitle
        color: Theme.textMuted
    }
    Dashboard {
        anchors.fill: parent
        anchors.topMargin: Theme.spacingSmall
    }
}
