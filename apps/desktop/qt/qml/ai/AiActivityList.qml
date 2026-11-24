import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP MCP4 -- What AI agents did, newest first (AiController.activity), with
// Undo per change: it puts back what the row changed, as the user's own edit,
// and leaves alone what someone has changed since (the row then says so).
// -- INTC (DL7 fold) -- Modern: no outline around the list (Theme.lineWidth),
// and with nothing done an EmptyState in place of the list and its buttons
// (`emptyName` is its objectName; its action shows the reads while they are
// hidden). Classic keeps the framed list and its sentence.
// -- WP REVDFIX -- (REVD D10) Modern: no panel box around the list (not "one
// card as a frame"), the rows ListRows with the outcome as a StatusChip
// (no warning-orange text) and the time at the right.
ColumnLayout {
    id: log
    property string prefix: "AiActivity"
    property string emptyName: prefix + ".empty"
    readonly property bool showsEmpty: LayoutController.modern && AiController.activity.length === 0
    spacing: Theme.spacingSmall

    EmptyState {
        objectName: log.emptyName
        visible: log.showsEmpty
        Layout.fillWidth: true
        Layout.fillHeight: true
        iconName: "bot"
        title: qsTr("No agent activity yet")
        text: qsTr("What AI agents change shows here, newest first, each with Undo.")
        actionText: AiController.showReads ? "" : qsTr("Show Reads")
        onAction: AiController.showReads = true
    }

    RowLayout {
        visible: !log.showsEmpty
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
        visible: !log.showsEmpty
        Layout.fillWidth: true
        Layout.fillHeight: true
        Layout.minimumHeight: 120
        radius: Theme.radius
        color: LayoutController.modern ? "transparent" : Theme.panel   // -- WP REVDFIX --
        border.width: Theme.lineWidth   // -- INTC (DL7 fold) -- none in Modern
        border.color: Theme.separator

        ListView {
            id: list
            objectName: log.prefix + ".list"
            // -- WP LAY7 -- The rows are made anew when the activity changes: the
            // Undo that had the keyboard gets it back on its own row
            property var undoKey: undefined
            function focusLost() {
                const f = Window.activeFocusItem
                return !f || f === list || f === list.contentItem
            }
            anchors.fill: parent
            anchors.margins: Theme.lineWidth
            clip: true
            model: AiController.activity
            boundsBehavior: Flickable.StopAtBounds
            activeFocusOnTab: true
            keyNavigationEnabled: true
            ScrollBar.vertical: AppScrollBar {}
            Accessible.role: Accessible.List
            Accessible.name: qsTr("Agent activity")

            delegate: LayoutController.modern ? modernRow : classicRow   // -- WP REVDFIX --

            // -- WP REVDFIX -- (REVD D10) Modern's row
            Component {
                id: modernRow
                ListRow {
                    id: mrow
                    required property var modelData
                    required property int index
                    readonly property bool canUndo: modelData.undo === "can" || modelData.undo === "busy"
                    objectName: log.prefix + ".row." + index
                    Component.onCompleted: {   // -- WP LAY7 --
                        if (list.undoKey === modelData.key && mundo.visible && list.focusLost()) mundo.forceActiveFocus(Qt.TabFocusReason)
                    }
                    width: ListView.view.width
                    rowIndex: index
                    selected: ListView.isCurrentItem && list.activeFocus
                    actionsAlways: canUndo
                    title: modelData.title
                    subtitle: [modelData.undo === "changed" || modelData.undo === "closed" && modelData.undoNote.length
                               ? modelData.undoNote : modelData.detail,
                               [modelData.client, modelData.device].filter(t => t.length).join(" · ")]
                              .filter(t => t.length).join(" · ")
                    value: modelData.time
                    onClicked: list.currentIndex = index
                    leading: StatusChip {
                        objectName: log.prefix + ".row." + mrow.index + ".outcome"
                        kind: mrow.modelData.undo === "changed" || mrow.modelData.outcome === "error" ? "warning" : "idle"
                        text: mrow.modelData.undo === "undone" ? qsTr("Undone") : mrow.modelData.outcomeText
                    }
                    AppButton {
                        id: mundo
                        objectName: log.prefix + ".row." + mrow.index + ".undo"
                        onActiveFocusChanged: {   // -- WP LAY7 --
                            if (activeFocus) list.undoKey = mrow.modelData.key
                        }
                        visible: mrow.canUndo
                        enabled: mrow.modelData.undo === "can"
                        iconName: "undo-2"
                        text: qsTr("Undo")
                        //: %1 is what an AI agent did
                        Accessible.name: qsTr("Undo: %1").arg(mrow.modelData.detail.length ? mrow.modelData.detail : mrow.modelData.title)
                        onClicked: AiController.undo(mrow.modelData.key)
                    }
                }
            }

            Component {
                id: classicRow
                Rectangle {
                    id: row
                    required property var modelData
                    required property int index
                    objectName: log.prefix + ".row." + index
                    Component.onCompleted: {   // -- WP LAY7 --
                        if (list.undoKey === modelData.key && undo.visible && list.focusLost()) undo.forceActiveFocus(Qt.TabFocusReason)
                    }
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
                            id: undo
                            objectName: log.prefix + ".row." + row.index + ".undo"
                            onActiveFocusChanged: {   // -- WP LAY7 --
                                if (activeFocus) list.undoKey = row.modelData.key
                            }
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
