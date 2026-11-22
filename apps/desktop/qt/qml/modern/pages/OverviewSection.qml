import QtQuick
import UConsole

// -- WP LAY3 -- Overview: the dashboard. Choosing a card's channel opens it on
// the Equaliser page. -- WP LAY5a -- Over the cards the device, its effects and
// its connection (OverviewSummary); without a device, the way to one.
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

    Column {
        anchors.centerIn: parent
        visible: !DeviceModel.connected
        spacing: Theme.spacing
        Text {
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("No Device Connected")
            font: Theme.fontTitle
            color: Theme.textMuted
        }
        AppButton {
            objectName: "OverviewSection.devices"
            anchors.horizontalCenter: parent.horizontalCenter
            text: qsTr("Choose a Device…")
            primary: true
            onClicked: if (page.host) page.host.go("devices", "devices")
        }
    }
    Dashboard {
        anchors.fill: parent
        anchors.topMargin: Theme.spacingSmall
        choosable: true
        onChosen: channel => SidebarController.select(channel)
        head: Component { OverviewSummary { host: page.host } }
    }
}
