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
// -- WP DL3a -- Modern (DESIGN-LANGUAGE §9, §16): the list in the 760 px
// column with no frame, each change a ListRow (time, a quiet icon for what
// undo does with it, the change in words), and an EmptyState when there is
// none. Classic keeps its framed list.
Panel {
    id: panel
    objectName: "HistoryPanel"
    readonly property bool modern: LayoutController.modern   // -- WP DL3a --

    onShownChanged: if (shown) HistoryController.reload()

    // -- WP DL3a -- one change as a Modern row
    Component {
        id: modernRow
        ListRow {
            id: change
            required property var modelData
            required property int index
            readonly property string undoState: modelData.source === "undo" ? "undone"
                                          : modelData.source === "redo" ? "redone"
                                          : !modelData.undoable ? "fixed" : ""
            readonly property string stateText: undoState === "undone" ? qsTr("Undone")
                                              : undoState === "redone" ? qsTr("Redone")
                                              : undoState === "fixed" ? qsTr("Cannot be undone") : ""
            objectName: "HistoryWindow.row." + index
            width: ListView.view.width
            rowIndex: index
            title: modelData.title
            subtitle: modelData.detail
            onClicked: ListView.view.currentIndex = index
            leading: [
                Text {
                    Layout.preferredWidth: 44
                    textFormat: Text.PlainText
                    text: change.modelData.time
                    font: Theme.fontMono
                    color: Theme.textMuted
                },
                // What undo does with it: a quiet icon, the words as its tooltip
                Item {
                    objectName: "HistoryWindow.state." + change.index
                    Layout.preferredWidth: 16
                    Layout.preferredHeight: 16
                    Icon {
                        anchors.centerIn: parent
                        visible: change.undoState.length > 0
                        name: change.undoState === "undone" ? "undo-2" : change.undoState === "redone" ? "redo-2" : "lock"
                        color: Theme.textMuted
                    }
                    HoverHandler { id: stateHover }
                    HelpTip {
                        text: change.stateText
                        visible: stateHover.hovered && change.stateText.length > 0
                    }
                },
                Pill {
                    objectName: "HistoryWindow.agent." + change.index
                    visible: change.modelData.agent.length > 0
                    text: change.modelData.agent
                    tint: Theme.accent
                }
            ]
            Accessible.name: [modelData.time, modelData.agent, modelData.title, modelData.detail, stateText]
                             .filter(s => s && s.length).join(", ")
        }
    }

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacingSmall

        RowLayout {
            Layout.maximumWidth: panel.modern ? Theme.contentMaxWidth : -1   // -- WP DL3a --
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
            // -- WP DL3a -- Modern: the column, no frame
            Layout.maximumWidth: panel.modern ? Theme.contentMaxWidth : -1
            radius: Theme.radius
            color: panel.modern ? "transparent" : Theme.panel
            border.color: Theme.separator
            border.width: panel.modern ? 0 : 1

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

                delegate: panel.modern ? modernRow : classicRow

                Text {
                    anchors.centerIn: parent
                    visible: !panel.modern && list.count === 0
                    text: DeviceModel.connected ? qsTr("No changes yet") : qsTr("No Device Connected")
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
                // -- WP DL3a --
                EmptyState {
                    objectName: "HistoryWindow.empty"
                    anchors.fill: parent
                    visible: panel.modern && list.count === 0
                    iconName: DeviceModel.connected ? "clock" : "unplug"
                    title: DeviceModel.connected ? qsTr("No changes yet") : qsTr("No Device Connected")
                    text: DeviceModel.connected ? qsTr("What you change on this device is listed here, newest first, to undo or redo.")
                                                : qsTr("Open a device to see its changes.")
                }
            }
        }
    }

    Component {
        id: classicRow
        Rectangle {
            id: row
            required property var modelData
            required property int index
            width: ListView.view.width
            height: Theme.rowHeight + 10
            color: ListView.isCurrentItem && list.activeFocus ? Theme.selection
                 : index % 2 ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03) : "transparent"
            opacity: row.modelData.undoable ? 1 : Theme.dim(0.6)
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
    }
}
