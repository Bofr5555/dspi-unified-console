import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP MCP4 -- What AI agents did, newest first (AiController.activity), with
// Undo per change: it puts back what the row changed, as the user's own edit,
// and leaves alone what someone has changed since (the row then says so).
ColumnLayout {
    id: log
    property string prefix: "AiActivity"
    spacing: Theme.spacingSmall

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        AppCheckBox {
            objectName: log.prefix + ".reads"
            text: qsTr("Show reads")
            checked: AiController.showReads
            onToggled: AiController.showReads = checked
        }
        Item { Layout.fillWidth: true }
        AppButton {
            objectName: log.prefix + ".export"
            text: qsTr("Export…")
            enabled: list.count > 0
            onClicked: AiController.exportActivity()
        }
        AppButton {
            objectName: log.prefix + ".undoAll"
            text: qsTr("Undo All")
            iconName: "undo-2"
            enabled: AiController.canUndoAny
            onClicked: AiController.undoAll()
        }
    }

    Rectangle {
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.minimumHeight: 120
        radius: Theme.radius
        color: Theme.panel
        border.width: 1
        border.color: Theme.separator

        ListView {
            id: list
            objectName: log.prefix + ".list"
            anchors.fill: parent
            anchors.margins: 1
            clip: true
            model: AiController.activity
            boundsBehavior: Flickable.StopAtBounds
            activeFocusOnTab: true
            keyNavigationEnabled: true
            ScrollBar.vertical: AppScrollBar {}
            Accessible.role: Accessible.List
            Accessible.name: qsTr("Agent activity")

            delegate: Rectangle {
                id: row
                required property var modelData
                required property int index
                objectName: log.prefix + ".row." + index
                width: ListView.view.width
                height: line.implicitHeight + 10
                color: ListView.isCurrentItem && list.activeFocus ? Theme.selection
                     : index % 2 ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03) : "transparent"
                RowLayout {
                    id: line
                    anchors.left: parent.left
                    anchors.right: parent.right
                    anchors.verticalCenter: parent.verticalCenter
                    anchors.leftMargin: Theme.spacingSmall
                    anchors.rightMargin: Theme.spacing
                    spacing: Theme.spacingSmall
                    Text {
                        Layout.alignment: Qt.AlignTop
                        textFormat: Text.PlainText
                        text: row.modelData.time
                        font: Theme.fontMono
                        color: Theme.textMuted
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        Text {
                            objectName: log.prefix + ".row." + row.index + ".title"
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: row.modelData.title
                            font: Theme.fontBody
                            color: Theme.text
                            elide: Text.ElideRight
                        }
                        Text {
                            objectName: log.prefix + ".row." + row.index + ".detail"
                            Layout.fillWidth: true
                            visible: text.length > 0
                            textFormat: Text.PlainText
                            text: row.modelData.undo === "changed" || row.modelData.undo === "closed" && row.modelData.undoNote.length
                                  ? row.modelData.undoNote : row.modelData.detail
                            font: Theme.fontSmall
                            color: row.modelData.undo === "changed" ? Theme.warning : Theme.textMuted
                            wrapMode: Text.Wrap
                        }
                        Text {
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: [row.modelData.client, row.modelData.device].filter(s => s.length).join(" · ")
                            font: Theme.fontSmall
                            color: Theme.textMuted
                            elide: Text.ElideRight
                        }
                    }
                    Text {
                        objectName: log.prefix + ".row." + row.index + ".outcome"
                        textFormat: Text.PlainText
                        text: row.modelData.undo === "undone" ? qsTr("Undone") : row.modelData.outcomeText
                        font: Theme.fontSmall
                        color: row.modelData.outcome === "error" ? Theme.warning : Theme.textMuted
                    }
                    AppButton {
                        objectName: log.prefix + ".row." + row.index + ".undo"
                        visible: row.modelData.undo === "can" || row.modelData.undo === "busy"
                        enabled: row.modelData.undo === "can"
                        text: qsTr("Undo")
                        //: %1 is what an AI agent did
                        Accessible.name: qsTr("Undo: %1").arg(row.modelData.detail.length ? row.modelData.detail : row.modelData.title)
                        onClicked: AiController.undo(row.modelData.key)
                    }
                }
                Accessible.role: Accessible.ListItem
                Accessible.name: [row.modelData.time, row.modelData.client, row.modelData.title, row.modelData.detail,
                                  row.modelData.outcomeText].filter(s => s && s.length).join(", ")
            }

            Text {
                anchors.centerIn: parent
                visible: list.count === 0
                text: qsTr("No agent has done anything yet")
                font: Theme.fontBody
                color: Theme.textMuted
            }
        }
    }
}
