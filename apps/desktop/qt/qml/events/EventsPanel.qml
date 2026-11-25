import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P2QT -- Device > Events…: what happened to the open DSPi, newest
// first. "On this console" is this computer's own log; "On the hub" is the
// hub's shared history over DSPi Link, with who caused each record (only on a
// hub that keeps one). EventsController reads both through the core.
// -- WP LAY1b -- A panel (Panel.qml): PanelTable's row for device.events gives
// its window's title and sizes.
// -- WP DL3b -- Modern (DESIGN-LANGUAGE §9): the rows are ListRows on the page,
// no box, at most 760 wide; on Modern's page the source and Refresh are in the
// row over it (pageTools). Classic keeps its window as it was.
Panel {
    id: panel
    objectName: "EventsPanel"
    readonly property bool modern: LayoutController.modern   // -- WP DL3b --

    onShownChanged: EventsController.active = shown
    Component.onDestruction: if (shown) EventsController.active = false   // -- WP LAY1b -- a page host unloaded

    // -- WP DL3b -- the row over Modern's page (PageHost.pageTools)
    readonly property Component pageTools: Component {
        Row {
            objectName: "EventsTools"
            spacing: 6
            ToolSegments {
                objectName: "EventsTools.source"
                anchors.verticalCenter: parent.verticalCenter
                visible: EventsController.hubAvailable
                label: qsTr("Where the events are kept")
                model: [qsTr("On This Console"), qsTr("On the Hub")]
                current: EventsController.source === "hub" ? 1 : 0
                onChosen: index => EventsController.source = index === 1 ? "hub" : "local"
            }
            AppButton {
                objectName: "EventsTools.reload"
                text: qsTr("Refresh")
                iconName: "refresh-cw"
                enabled: DeviceModel.connected && !EventsController.busy
                onClicked: EventsController.reload()
            }
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacingSmall

        RowLayout {
            Layout.fillWidth: true
            visible: !panel.inPage || !panel.modern   // -- WP DL3b -- on Modern's page: pageTools
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

        // -- WP DL3b -- Modern: a notice, not an orange sentence
        NoticeRow {
            objectName: panel.modern ? "Events.problem" : ""
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.contentMaxWidth
            visible: panel.modern && text.length > 0
            kind: "warning"
            text: EventsController.problem
        }
        Text {
            objectName: panel.modern ? "" : "Events.problem"   // -- WP DL3b --
            Layout.fillWidth: true
            visible: !panel.modern && text.length > 0   // -- WP DL3b -- modern
            textFormat: Text.PlainText
            text: EventsController.problem
            font: Theme.fontSmall
            color: Theme.warning
            wrapMode: Text.Wrap
        }

        // -- WP DL3b -- Modern: the shared rows, on the page
        ListView {
            id: rows
            objectName: panel.modern ? "Events.list" : ""
            Layout.fillWidth: true
            Layout.maximumWidth: Theme.contentMaxWidth
            Layout.fillHeight: true
            visible: panel.modern
            clip: true
            model: panel.modern ? EventsController.entries : []
            activeFocusOnTab: true
            keyNavigationEnabled: true
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}
            Accessible.role: Accessible.List
            Accessible.name: EventsController.source === "hub" ? qsTr("Events on the hub") : qsTr("Events on this console")

            delegate: ListRow {
                id: event
                required property var modelData
                required property int index
                objectName: "Events.row." + index
                width: ListView.view.width
                rowIndex: index
                title: modelData.count > 1
                       //: An event and how many times it happened in a row
                       ? qsTr("%1 (×%2)").arg(modelData.text).arg(modelData.count)
                       : modelData.text
                // -- WP SHOWFIX -- (finding 2) the device's own record on demand:
                // under the chosen row, and in the tooltip
                readonly property string raw: modelData.raw ?? ""
                //: An event's details row: the device's own record, as the Interrupt Monitor shows it
                readonly property string rawLine: raw.length ? qsTr("Device record: %1").arg(raw) : ""
                subtitle: rows.currentIndex === index && rawLine.length
                          ? [modelData.detail, rawLine].filter(s => s && s.length).join(" · ") : modelData.detail
                //: Who caused an event: a console, an API key or a schedule, by name
                value: modelData.by.length > 0 ? qsTr("by %1").arg(modelData.by) : ""
                monoValue: false
                onClicked: rows.currentIndex = index
                leading: Text {
                    Layout.preferredWidth: implicitWidth
                    textFormat: Text.PlainText
                    text: event.modelData.time
                    font: Theme.fontMono
                    color: Theme.textMuted
                }
                ToolTip.visible: hovered
                ToolTip.delay: 700
                ToolTip.text: [modelData.day + " " + modelData.time, title, modelData.detail, rawLine].filter(s => s && s.length).join("\n")
                Accessible.name: [modelData.time, title, subtitle, value].filter(s => s && s.length).join(", ")
            }

            EmptyState {
                objectName: "Events.empty"
                anchors.fill: parent
                visible: rows.count === 0
                iconName: "list-ordered"
                title: !DeviceModel.connected ? qsTr("No Device Connected")
                     : EventsController.busy ? qsTr("Reading…")
                     : qsTr("No events yet")
            }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            visible: !panel.modern   // -- WP DL3b --
            radius: Theme.radius
            color: Theme.panel
            border.color: Theme.separator

            ListView {
                id: list
                objectName: panel.modern ? "" : "Events.list"   // -- WP DL3b --
                anchors.fill: parent
                anchors.margins: 1
                clip: true
                model: panel.modern ? [] : EventsController.entries   // -- WP DL3b --
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
                            textFormat: Text.PlainText
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
                                textFormat: Text.PlainText
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
                                textFormat: Text.PlainText
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
                            textFormat: Text.PlainText
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
                    textFormat: Text.PlainText
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
