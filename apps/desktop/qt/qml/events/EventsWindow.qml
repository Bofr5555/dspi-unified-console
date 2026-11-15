import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P2QT -- Device > Events…: what happened to the open DSPi, newest
// first. "On this console" is this computer's own log; "On the hub" is the
// hub's shared history over DSPi Link, with who caused each record (only on a
// hub that keeps one). EventsController reads both through the core.
ToolWindow {
    id: window
    objectName: "EventsWindow"
    windowId: "events"
    defaultWidth: 560
    defaultHeight: 480
    minimumWidth: 400
    minimumHeight: 280
    title: qsTr("Events")

    onVisibleChanged: EventsController.active = visible

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacingSmall

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            AppTabBar {
                objectName: "Events.source"
                visible: EventsController.hubAvailable
                label: qsTr("Where the events are kept")
                currentIndex: EventsController.source === "hub" ? 1 : 0
                AppTabButton {
                    objectName: "Events.source.local"
                    text: qsTr("On This Console")
                    onClicked: EventsController.source = "local"
                }
                AppTabButton {
                    objectName: "Events.source.hub"
                    text: qsTr("On the Hub")
                    onClicked: EventsController.source = "hub"
                }
            }
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "Events.reload"
                text: qsTr("Refresh")
                iconName: "refresh-cw"
                enabled: DeviceModel.connected && !EventsController.busy
                onClicked: EventsController.reload()
            }
        }

        Text {
            objectName: "Events.problem"
            Layout.fillWidth: true
            visible: text.length > 0
            text: EventsController.problem
            font: Theme.fontSmall
            color: Theme.warning
            wrapMode: Text.Wrap
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            radius: Theme.radius
            color: Theme.panel
            border.color: Theme.separator

            ListView {
                id: list
                objectName: "Events.list"
                anchors.fill: parent
                anchors.margins: 1
                clip: true
                model: EventsController.entries
                activeFocusOnTab: true
                keyNavigationEnabled: true
                ScrollBar.vertical: AppScrollBar {}
                Accessible.role: Accessible.List
                Accessible.name: EventsController.source === "hub" ? qsTr("Events on the hub") : qsTr("Events on this console")

                delegate: Rectangle {
                    id: row
                    required property var modelData
                    required property int index
                    objectName: "Events.row." + index
                    width: ListView.view.width
                    height: column.implicitHeight + 10
                    color: ListView.isCurrentItem && list.activeFocus ? Theme.selection
                         : index % 2 ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03) : "transparent"
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.spacingSmall
                        anchors.rightMargin: Theme.spacingSmall
                        spacing: Theme.spacingSmall
                        Text {
                            Layout.alignment: Qt.AlignTop
                            Layout.topMargin: 5
                            text: row.modelData.time
                            font: Theme.fontMono
                            color: Theme.textMuted
                            ToolTip.visible: timeHover.hovered
                            ToolTip.text: row.modelData.day
                            HoverHandler { id: timeHover }
                        }
                        ColumnLayout {
                            id: column
                            Layout.fillWidth: true
                            Layout.topMargin: 5
                            spacing: 0
                            Text {
                                Layout.fillWidth: true
                                text: row.modelData.count > 1
                                      //: An event and how many times it happened in a row
                                      ? qsTr("%1 (×%2)").arg(row.modelData.text).arg(row.modelData.count)
                                      : row.modelData.text
                                font: Theme.fontBody
                                color: Theme.text
                                wrapMode: Text.Wrap
                            }
                            Text {
                                Layout.fillWidth: true
                                visible: text.length > 0
                                text: row.modelData.detail
                                font: Theme.fontSmall
                                color: Theme.textMuted
                                elide: Text.ElideRight
                            }
                        }
                        Text {
                            objectName: "Events.by." + row.index
                            Layout.alignment: Qt.AlignTop
                            Layout.topMargin: 5
                            Layout.maximumWidth: 160
                            visible: row.modelData.by.length > 0
                            //: Who caused an event: a console, an API key or a schedule, by name
                            text: qsTr("by %1").arg(row.modelData.by)
                            font: Theme.fontSmall
                            color: Theme.textMuted
                            elide: Text.ElideRight
                        }
                    }
                    Accessible.role: Accessible.ListItem
                    Accessible.name: [row.modelData.time, row.modelData.text, row.modelData.detail,
                                      row.modelData.by.length ? qsTr("by %1").arg(row.modelData.by) : ""]
                                     .filter(s => s && s.length).join(", ")
                }

                Text {
                    anchors.centerIn: parent
                    width: parent.width - 2 * Theme.spacingLarge
                    horizontalAlignment: Text.AlignHCenter
                    wrapMode: Text.Wrap
                    visible: list.count === 0
                    text: !DeviceModel.connected ? qsTr("No Device Connected")
                        : EventsController.busy ? qsTr("Reading…")
                        : qsTr("No events yet")
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
            }
        }
    }
}
