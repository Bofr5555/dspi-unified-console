import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP18 -- Edit > History...: this console's changes to the open device,
// newest first, from the core's journal (HistoryController), with Undo and
// Redo. A change the core cannot undo (a preset load, an output switched on,
// a flash write) is listed dimmed and stepped over by Undo.
// -- WP LAY1b -- A panel (Panel.qml): PanelTable's row for edit.history gives
// its window's title and sizes.
Panel {
    id: panel
    objectName: "HistoryPanel"

    onShownChanged: if (shown) HistoryController.reload()

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacingSmall

        RowLayout {
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "HistoryWindow.undo"
                text: qsTr("Undo")
                iconName: "undo-2"
                enabled: HistoryController.canUndo
                onClicked: HistoryController.undo()
            }
            AppButton {
                objectName: "HistoryWindow.redo"
                text: qsTr("Redo")
                iconName: "redo-2"
                enabled: HistoryController.canRedo
                onClicked: HistoryController.redo()
            }
            Item { Layout.fillWidth: true }
        }

        Rectangle {
            Layout.fillWidth: true
            Layout.fillHeight: true
            radius: Theme.radius
            color: Theme.panel
            border.color: Theme.separator

            ListView {
                id: list
                objectName: "HistoryWindow.list"
                anchors.fill: parent
                anchors.margins: 1
                clip: true
                model: HistoryController.entries
                activeFocusOnTab: true
                keyNavigationEnabled: true
                ScrollBar.vertical: AppScrollBar {}
                Accessible.role: Accessible.List
                Accessible.name: qsTr("Changes")

                delegate: Rectangle {
                    id: row
                    required property var modelData
                    required property int index
                    width: ListView.view.width
                    height: Theme.rowHeight + 10
                    color: ListView.isCurrentItem && list.activeFocus ? Theme.selection
                         : index % 2 ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03) : "transparent"
                    opacity: row.modelData.undoable ? 1 : 0.6
                    RowLayout {
                        anchors.fill: parent
                        anchors.leftMargin: Theme.spacingSmall
                        anchors.rightMargin: Theme.spacingSmall
                        spacing: Theme.spacingSmall
                        Text {
                            text: row.modelData.time
                            font: Theme.fontMono
                            color: Theme.textMuted
                        }
                        ColumnLayout {
                            Layout.fillWidth: true
                            spacing: 0
                            Text {
                                Layout.fillWidth: true
                                textFormat: Text.PlainText
                                text: row.modelData.title
                                font: Theme.fontBody
                                color: Theme.text
                                elide: Text.ElideRight
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
                        // -- WP MCP4 -- an AI agent's change
                        Pill {
                            objectName: "HistoryWindow.agent." + row.index
                            visible: row.modelData.agent.length > 0
                            text: row.modelData.agent
                            tint: Theme.accent
                        }
                        Text {
                            visible: text.length > 0
                            text: row.modelData.source === "undo" ? qsTr("Undone")
                                : row.modelData.source === "redo" ? qsTr("Redone")
                                : !row.modelData.undoable ? qsTr("Cannot be undone") : ""
                            font: Theme.fontSmall
                            color: Theme.textMuted
                        }
                    }
                    Accessible.role: Accessible.ListItem
                    Accessible.name: [row.modelData.time, row.modelData.agent, row.modelData.title, row.modelData.detail].filter(s => s && s.length).join(", ")
                }

                Text {
                    anchors.centerIn: parent
                    visible: list.count === 0
                    text: DeviceModel.connected ? qsTr("No changes yet") : qsTr("No Device Connected")
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
            }
        }
    }
}
