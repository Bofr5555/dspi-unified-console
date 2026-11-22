import QtQuick
import UConsole

// -- WP LAY5a -- The choice in the row over the Overview: how many cards a row
// has (behind the gear on every card in Classic).
Row {
    objectName: "OverviewTools"
    visible: DeviceModel.connected
    spacing: Theme.spacingSmall

    Text {
        anchors.verticalCenter: parent.verticalCenter
        text: qsTr("Cards per row")
        font: Theme.fontSmall
        color: Theme.textMuted
    }
    ToolSegments {
        objectName: "OverviewTools.perRow"
        label: qsTr("Cards per row")
        model: [qsTr("Auto"), "1", "2", "3"]
        current: DashboardController.cardsPerRow
        onChosen: index => DashboardController.cardsPerRow = index
    }
}
