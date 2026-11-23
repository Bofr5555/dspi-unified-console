import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY8 -- Under the top bar's AI button: whether AI agents are allowed (the
// switch, which asks first), their state (off, waiting with the addresses, the
// agents connected with Disconnect), what they changed last with Undo, Pause, and
// the ways to all of it: Settings > AI Agents and the activity. What an agent
// asks stays AiHost's sheet, which closes this.
PopoverPanel {
    id: popover
    title: qsTr("AI agents")
    property QtObject ai: AiController
    property int maxHeight: 600
    readonly property alias enableDialog: enableQuestion
    readonly property bool asking: ai.state === "asking"
    // The newest changes made; reads and questions are only in the whole activity
    readonly property var changes: ai.activity.filter(r => r.write && r.outcome === "ok").slice(0, 4)

    // -- WP LAY7 -- The rows are made anew when the activity changes: the Undo that
    // had the keyboard gets it back on its own change's new row
    property var undoKey: undefined
    function focusLost() {
        const f = body.Window.activeFocusItem
        return !f || f === body || f === body.parent
    }

    margins: Theme.spacingSmall
    padding: 0
    height: Math.min(implicitHeight, maxHeight)
    onAskingChanged: if (asking) close()
    onOpened: {
        undoKey = undefined
        ai.refresh()
        allow.forceActiveFocus(Qt.PopupFocusReason)
    }

    contentItem: Flickable {
        id: body
        clip: true
        implicitHeight: column.implicitHeight + 2 * Theme.spacing
        contentWidth: width
        contentHeight: implicitHeight
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: AppScrollBar {}

        ColumnLayout {
            id: column
            x: Theme.spacing
            y: Theme.spacing
            width: body.width - 2 * Theme.spacing
            spacing: Theme.spacingSmall

            // ---- On and off
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacingSmall
                Icon {
                    name: "bot"
                    size: 18
                    color: Theme.text
                }
                Text {
                    Layout.fillWidth: true
                    //: %1 is the app's name
                    text: qsTr("Allow AI agents to control %1").arg(AppIdentity.displayName)
                    font: Theme.fontBody
                    color: Theme.text
                    wrapMode: Text.Wrap
                }
                AppSwitch {
                    id: allow
                    objectName: "AiPopover.allow"
                    checked: popover.ai.enabled
                    onToggled: {
                        if (!checked) popover.ai.setEnabled(false)
                        else enableQuestion.open()
                        checked = Qt.binding(() => popover.ai.enabled)
                    }
                    Accessible.name: qsTr("Allow AI agents")
                }
            }

            // ---- The state
            Text {
                objectName: "AiPopover.status"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: popover.ai.working === true && popover.ai.work.text ? popover.ai.work.text : popover.ai.statusLine   // -- WP AIVIS --
                font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
                color: Theme.text
                wrapMode: Text.Wrap
            }
            Note {
                objectName: "AiPopover.off"
                visible: !popover.ai.enabled
                text: qsTr("Nothing listens and nothing is sent while this is off. On, only agents on this computer can connect, and you allow each of them.")
            }
            Note {
                objectName: "AiPopover.addresses"
                visible: popover.ai.running
                //: %1 is a list of addresses
                text: qsTr("Listening on %1").arg(popover.ai.addresses.join(", "))
            }
            Note {
                objectName: "AiPopover.asks"
                visible: popover.ai.running
                color: popover.ai.confirmMode === "never_ask" ? Theme.warning : Theme.textMuted
                text: popover.ai.confirmMode === "never_ask" ? qsTr("Agents read and change settings without asking you.")
                    : popover.ai.confirmMode === "ask_everything" ? qsTr("Agents read settings, and ask you before every change.")
                    : qsTr("Agents read and change settings, and ask you before risky changes.")
            }
            AiGrantRows {   // -- WP ODEC --
                prefix: "AiPopover.grants"
                compact: true
                Layout.fillWidth: true
            }
            RowLayout {
                visible: popover.ai.lastError.length > 0
                Layout.fillWidth: true
                spacing: 6
                Icon {
                    name: "triangle-alert"
                    size: 14
                    color: Theme.warning
                }
                Note {
                    objectName: "AiPopover.error"
                    color: Theme.warning
                    text: popover.ai.lastError
                }
                AppButton {
                    objectName: "AiPopover.retry"
                    visible: popover.ai.enabled && !popover.ai.running && !popover.ai.busy
                    text: qsTr("Try Again")
                    onClicked: popover.ai.dismissError()
                }
            }
            AppButton {
                objectName: "AiPopover.connect"
                visible: popover.ai.running && popover.ai.clients.length === 0
                text: qsTr("Connect an Agent…")
                iconName: "plug"
                primary: true
                onClicked: popover.toSettings()
            }

            // ---- The agents connected
            Repeater {
                model: popover.ai.agents
                delegate: RowLayout {
                    id: agent
                    required property var modelData
                    required property int index
                    objectName: "AiPopover.agent." + index
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                    Rectangle {
                        implicitWidth: 8
                        implicitHeight: 8
                        radius: 4
                        color: Theme.connected
                    }
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        Text {
                            objectName: "AiPopover.agent." + agent.index + ".name"
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: agent.modelData.name
                            font: Theme.fontBody
                            color: Theme.text
                            elide: Text.ElideRight
                        }
                        Note {
                            //: %1 is a time
                            text: qsTr("Connected since %1").arg(agent.modelData.since)
                        }
                    }
                    AppButton {
                        objectName: "AiPopover.agent." + agent.index + ".disconnect"
                        text: qsTr("Disconnect")
                        toolTip: qsTr("Its token stops: it has to be allowed again.")
                        //: %1 is an AI agent's name
                        Accessible.name: qsTr("Disconnect %1").arg(agent.modelData.name)
                        onClicked: popover.ai.disconnectAgent(agent.modelData.id)
                    }
                }
            }

            // ---- What they changed
            Rule {
                visible: popover.changes.length > 0
            }
            SectionHeading {
                visible: popover.changes.length > 0
                text: qsTr("Latest changes")
            }
            Repeater {
                model: popover.changes
                delegate: RowLayout {
                    id: change
                    required property var modelData
                    required property int index
                    readonly property bool kept: modelData.undo === "changed" || modelData.undo === "closed" && modelData.undoNote.length > 0
                    objectName: "AiPopover.change." + index
                    Component.onCompleted: {   // -- WP LAY7 --
                        if (popover.opened && popover.undoKey === modelData.key && undo.visible && popover.focusLost())
                            undo.forceActiveFocus(Qt.TabFocusReason)
                    }
                    Layout.fillWidth: true
                    spacing: Theme.spacingSmall
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 0
                        Text {
                            objectName: "AiPopover.change." + change.index + ".title"
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: change.modelData.detail.length ? change.modelData.detail : change.modelData.title
                            font: Theme.fontBody
                            color: Theme.text
                            wrapMode: Text.Wrap
                            maximumLineCount: 2
                            elide: Text.ElideRight
                        }
                        Note {
                            objectName: "AiPopover.change." + change.index + ".who"
                            color: change.kept ? Theme.warning : Theme.textMuted
                            text: change.kept ? change.modelData.undoNote
                                : [change.modelData.time, change.modelData.client, change.modelData.device].filter(s => s.length).join(" · ")
                        }
                    }
                    Text {
                        visible: !undo.visible
                        textFormat: Text.PlainText
                        text: change.modelData.undo === "undone" ? qsTr("Undone") : change.modelData.outcomeText
                        font: Theme.fontSmall
                        color: Theme.textMuted
                    }
                    AppButton {
                        id: undo
                        objectName: "AiPopover.change." + change.index + ".undo"
                        visible: change.modelData.undo === "can" || change.modelData.undo === "busy"
                        enabled: change.modelData.undo === "can"
                        text: qsTr("Undo")
                        //: %1 is what an AI agent did
                        Accessible.name: qsTr("Undo: %1").arg(change.modelData.detail.length ? change.modelData.detail : change.modelData.title)
                        onActiveFocusChanged: if (activeFocus) popover.undoKey = change.modelData.key   // -- WP LAY7 --
                        onClicked: popover.ai.undo(change.modelData.key)
                    }
                }
            }

            // ---- The rest
            Rule {}
            Flow {
                Layout.fillWidth: true
                spacing: Theme.spacingSmall
                AppButton {
                    objectName: "AiPopover.pause"
                    visible: popover.ai.running
                    text: popover.ai.paused ? qsTr("Resume") : qsTr("Pause")
                    iconName: popover.ai.paused ? "play" : "pause"
                    onClicked: popover.ai.setPaused(!popover.ai.paused)
                }
                // -- WP AIVIS --
                AppButton {
                    objectName: "AiPopover.stop"
                    visible: popover.ai.running
                    text: qsTr("Stop")
                    iconName: "square"
                    toolTip: qsTr("Turn AI agents off: what they run stops, their timed previews are put back")
                    onClicked: popover.ai.setEnabled(false)
                }
                AppButton {
                    objectName: "AiPopover.activity"
                    text: qsTr("Activity…")
                    onClicked: {
                        popover.close()
                        popover.ai.openPanel()
                    }
                }
                AppButton {
                    objectName: "AiPopover.settings"
                    text: qsTr("AI Agents Settings…")
                    onClicked: popover.toSettings()
                }
            }
        }
    }

    function toSettings() {
        close()
        ai.openSettings()
    }

    AiEnableQuestion {
        id: enableQuestion
        objectName: "AiPopover.enableQuestion"
        parent: Overlay.overlay
        ai: popover.ai
        onAnswered: answer => { if (answer === "alternative") popover.close() }
    }

    component Note: Text {
        Layout.fillWidth: true
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
    component Rule: Rectangle {
        Layout.fillWidth: true
        implicitHeight: 1
        color: Theme.separator
    }
}
