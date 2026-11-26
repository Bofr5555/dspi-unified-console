import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP MCP4 -- Device > AI Agents... (Classic's tool window; a panel as any):
// the agents' state, Pause and Stop, what waits for the user, and what they did.
// -- WP DL7 -- Modern (DESREV): Stop is a secondary button in dangerText, not
// a red fill beside Pause; with nothing done yet the page shows an EmptyState
// instead of an empty framed list.
Panel {
    id: panel
    objectName: "AiAgentsPanel"
    readonly property string state: AiController.state
    readonly property bool modern: LayoutController.modern   // -- WP DL7 --

    onShownChanged: if (shown) AiController.refresh()

    ColumnLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        // -- WP DL7 -- Modern: a list stops at Theme.contentMaxWidth, left-aligned
        anchors.rightMargin: panel.modern ? Math.max(Theme.spacing, panel.width - Theme.spacing - Theme.contentMaxWidth) : Theme.spacing
        spacing: Theme.spacing

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            Icon {
                name: "bot"
                size: 18
                color: Theme.text
            }
            Text {
                objectName: "AiAgents.status"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: AiController.statusLine
                font: Theme.fontTitle
                color: Theme.text
                elide: Text.ElideRight
            }
            AppButton {
                objectName: "AiAgents.pause"
                visible: AiController.running
                text: AiController.paused ? qsTr("Resume") : qsTr("Pause")
                iconName: AiController.paused ? "play" : "pause"
                onClicked: AiController.setPaused(!AiController.paused)
            }
            AppButton {
                id: stop
                objectName: "AiAgents.stop"
                visible: AiController.enabled
                text: qsTr("Stop Agents")   // -- WP UAG -- (UXP A7, C4) one meaning per Stop
                iconName: "square"
                destructive: !panel.modern   // -- WP DL7 -- Modern: quiet, its text in dangerText
                danger: panel.modern         // -- INTC (DL7 fold) -- AppButton's quiet danger kind
                onClicked: AiController.stop()
            }
            AppButton {
                objectName: "AiAgents.settings"
                text: qsTr("Settings…")
                onClicked: AiController.openSettings()
            }
        }
        Text {
            Layout.fillWidth: true
            visible: AiController.running
            text: qsTr("Stop Agents turns AI agents off and puts their timed previews back. Changes already made stay, and you can undo them below.")
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }

        // The same questions as the main window's sheets, for a window in front of it
        Repeater {
            model: AiController.confirmations
            delegate: Rectangle {
                id: ask
                required property var modelData
                required property int index
                objectName: "AiAgents.ask." + index
                Layout.fillWidth: true
                implicitHeight: askRow.implicitHeight + 2 * Theme.spacingSmall
                radius: Theme.radius
                color: Theme.bannerFill("warning")
                border.width: 1
                border.color: Theme.bannerEdge("warning")
                RowLayout {
                    id: askRow
                    x: Theme.spacingSmall
                    y: Theme.spacingSmall
                    width: parent.width - 2 * Theme.spacingSmall
                    spacing: Theme.spacingSmall
                    ColumnLayout {
                        Layout.fillWidth: true
                        spacing: 2
                        Text {
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: [ask.modelData.client, ask.modelData.title, ask.modelData.device].filter(s => s.length).join(" · ")
                            font: Theme.fontBody
                            color: Theme.text
                            wrapMode: Text.Wrap
                        }
                        Text {
                            Layout.fillWidth: true
                            textFormat: Text.PlainText
                            text: ask.modelData.riskLines.concat(ask.modelData.warning ? ["⚠ " + ask.modelData.warning] : [])   // -- WP ADETQ --
                                  .concat(ask.modelData.grantText ? [ask.modelData.grantText] : []).join("\n")   // -- WP LAY8 --, ODEC
                            font: Theme.fontSmall
                            color: Theme.text
                            wrapMode: Text.Wrap
                            maximumLineCount: 8
                            elide: Text.ElideRight
                        }
                    }
                    AiAnswerButtons {
                        prefix: ask.objectName
                        question: ask.modelData.id
                        onAnswered: allow => AiController.answer(ask.modelData.id, allow)
                    }
                }
            }
        }

        AiGrantRows {   // -- WP ODEC --
            prefix: "AiAgentsPanel.grants"
            Layout.fillWidth: true
        }

        // -- WP DL7 -- Modern: an EmptyState while nothing is done (INTC: the list's own)
        AiActivityList {
            emptyName: "AiAgentsPanel.empty"
            Layout.fillWidth: true
            Layout.fillHeight: true
        }
    }
}
