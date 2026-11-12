import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// The Overview (the Mac's DashboardOverview): the stereo input card, then the
// output cards, in a grid whose cards per row is Auto or the user's choice
// (the layout popup behind each card's gear). Every card is a header, the
// columns' meters and ten band rows, so the cards in a row line up.
//
// Without a device the cards fade out rather than leave, so connecting is a
// crossfade with no relayout; faded cards take no clicks.
Item {
    id: dashboard
    objectName: "Dashboard"

    readonly property int spacing: DashboardController.spacing
    readonly property bool ready: DeviceModel.connected
    readonly property int columns: DashboardController.columnsFor(DashboardController.cardsPerRow, grid.width,
                                                                  DashboardController.cards.length)

    opacity: ready ? 1 : 0
    enabled: ready
    Behavior on opacity { OpacityAnimator { duration: 300 } }

    Flickable {
        id: flick
        anchors.fill: parent
        contentWidth: width
        // The Mac's 4 px above the cards, and room for the stale note below
        contentHeight: content.implicitHeight + content.y + Theme.spacingLarge
        clip: true
        // A desktop pane: no overshoot past the ends
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: AppScrollBar {}

        ColumnLayout {
            id: content
            x: Theme.spacingLarge
            y: Theme.spacingSmall
            width: flick.width - 2 * Theme.spacingLarge
            spacing: dashboard.spacing

            GridLayout {
                id: grid
                objectName: "Dashboard.grid"
                Layout.fillWidth: true
                columns: dashboard.columns
                columnSpacing: dashboard.spacing
                rowSpacing: dashboard.spacing

                Repeater {
                    model: DashboardController.cards
                    DashboardCard {
                        required property var modelData
                        required property int index
                        objectName: "Dashboard.card." + modelData.key
                        entry: modelData
                        Layout.fillWidth: true
                        Layout.preferredWidth: (grid.width - (grid.columns - 1) * grid.columnSpacing) / grid.columns
                        Layout.alignment: Qt.AlignTop
                    }
                }
            }
        }
    }

    // The meters' state: a note while no reading arrives, over the bottom
    // right corner so the cards do not move. The meters keep the last values
    // read and are never greyed.
    Rectangle {
        id: staleNote
        objectName: "Dashboard.stale"
        x: dashboard.width - width - Theme.spacingLarge
        y: dashboard.height - height - Theme.spacingLarge
        visible: DashboardController.stale
        width: staleRow.implicitWidth + 2 * Theme.spacingSmall
        height: staleRow.implicitHeight + Theme.spacingSmall
        radius: Theme.radiusSmall
        color: Theme.panel
        border.width: 1
        border.color: Theme.warning
        RowLayout {
            id: staleRow
            x: Theme.spacingSmall
            y: Theme.spacingSmall / 2
            spacing: 4
            Icon {
                name: "clock"
                size: 12
                color: Theme.warning
            }
            Text {
                text: qsTr("Meters stale")
                font: Theme.fontSmall
                color: Theme.warning
            }
        }
        HoverHandler { id: staleHover }
        HelpTip {
            text: qsTr("No meter reading has come from the device for a while. The meters show the last values read.")
            visible: staleHover.hovered
        }
        Accessible.role: Accessible.StatusBar
        Accessible.name: qsTr("Meters stale")
    }

    Accessible.role: Accessible.Pane
    Accessible.name: qsTr("Overview")
}
