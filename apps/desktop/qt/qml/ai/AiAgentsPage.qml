import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP MCP4 -- Settings > AI Agents (both layouts): allowing AI agents on this
// computer to control the devices, how to connect one, the clients and their
// tokens, when they ask, and what they did.
SettingsPage {
    id: page
    objectName: "AiAgentsPage"
    readonly property alias enableDialog: enableQuestion
    readonly property bool watching: visible && Window.window !== null && Window.window.visible
    onWatchingChanged: if (watching) AiController.refresh()
    Component.onDestruction: AiController.clearNewClient()

    function when(epochSeconds) {
        return epochSeconds > 0 ? new Date(epochSeconds * 1000).toLocaleString(Qt.locale(), Locale.ShortFormat) : ""
    }

    // ---- On and off -------------------------------------------------------------------
    SettingsGroup {
        objectName: "AiAgents.group"
        footer: qsTr("Agents on this computer connect through MCP. Nothing on your network can reach this. What an agent reads, it may pass on to its own service.")
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing
            Rectangle {
                implicitWidth: 40
                implicitHeight: 40
                radius: 9
                color: Theme.settingsTint("ai")
                Icon {
                    anchors.centerIn: parent
                    name: "bot"
                    size: 20
                    color: Theme.textOnAccent
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 3
                Text {
                    Layout.fillWidth: true
                    //: %1 is the app's name
                    text: qsTr("Allow AI agents to control %1").arg(AppIdentity.displayName)
                    font: Theme.fontTitle
                    color: Theme.text
                    wrapMode: Text.Wrap
                }
                Text {
                    objectName: "AiAgents.statusLine"
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: AiController.statusLine
                    font: Theme.fontBody
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
                Text {
                    objectName: "AiAgents.addresses"
                    visible: AiController.running
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    //: %1 is a list of addresses
                    text: qsTr("Listening on %1").arg(AiController.addresses.join(", "))
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
            AppSwitch {
                id: allow
                objectName: "AiAgents.allow"
                checked: AiController.enabled
                onToggled: {
                    if (!checked) AiController.setEnabled(false)
                    else enableQuestion.open()
                    checked = Qt.binding(() => AiController.enabled)
                }
                Accessible.name: qsTr("Allow AI agents")
            }
        }
        RowLayout {
            objectName: "AiAgents.error"
            visible: AiController.lastError.length > 0
            Layout.fillWidth: true
            spacing: 6
            Icon {
                name: "triangle-alert"
                size: 14
                color: Theme.warning
            }
            Text {
                objectName: "AiAgents.errorText"
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: AiController.lastError
                font: Theme.fontBody
                color: Theme.warning
                wrapMode: Text.Wrap
            }
            AppButton {
                objectName: "AiAgents.retry"
                visible: AiController.enabled && !AiController.running
                text: qsTr("Try Again")
                onClicked: AiController.dismissError()
            }
        }
        // Offered, never required (owner): the agent policy is the floor either way
        RowLayout {
            objectName: "AiAgents.limitsOffer"
            visible: AiController.enabled && LimitsController.connected && !LimitsController.active
            Layout.fillWidth: true
            spacing: 6
            Icon {
                name: "info"
                size: 14
                color: Theme.textMuted
            }
            Text {
                Layout.fillWidth: true
                text: qsTr("This device has no limits profile. One sets the most gain and volume anyone may set, agents included, and protects your speakers.")
                font: Theme.fontBody
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            AppButton {
                objectName: "AiAgents.setLimits"
                text: qsTr("Set Up Limits…")
                onClicked: AiController.openLimits()
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing
            Text {
                Layout.fillWidth: true
                text: qsTr("Port")
                font: Theme.fontBody
                color: Theme.text
            }
            BorderedField {
                objectName: "AiAgents.port"
                label: qsTr("Port")
                Layout.preferredWidth: 80
                horizontalAlignment: TextInput.AlignRight
                enabled: !AiController.running && !AiController.busy
                validator: IntValidator { bottom: 1; top: 65535 }
                text: String(AiController.port)
                onCommitted: typed => {
                    AiController.setPort(parseInt(typed))
                    text = Qt.binding(() => String(AiController.port))
                }
            }
        }
    }

    // ---- When they ask ------------------------------------------------------------------
    SettingsGroup {
        objectName: "AiAgents.permissions"
        title: qsTr("Ask Before Changes")
        iconName: "circle-question-mark"
        footer: AiController.confirmMode === "never_ask"
                ? qsTr("Agents' changes are made at once, louder ones and test signals too. Your limits profile and the protection of outputs you set up as tweeters still apply.")
                : AiController.confirmMode === "ask_everything"
                ? qsTr("Every change waits for your answer.")
                : qsTr("Changes that make something louder, unmute or switch on an output, load or save a preset, or lower a tweeter's protection wait for your answer. Others are made at once.")
        footerColor: AiController.confirmMode === "never_ask" ? Theme.warning : Theme.textMuted
        SettingsPickerRow {
            objectName: "AiAgents.confirm"
            inline: true
            label: qsTr("Ask me")
            values: ["ask_everything", "ask_risky", "never_ask"]
            texts: [qsTr("About every change"), qsTr("About risky changes"), qsTr("Never")]
            value: AiController.confirmMode
            onChosen: mode => AiController.confirmMode = mode
        }
        SettingsValueRow {
            objectName: "AiAgents.limits"
            title: qsTr("Limits profile")
            value: !LimitsController.connected ? qsTr("No device connected")
                 : LimitsController.active ? qsTr("In force: agents are held to it") : qsTr("None")
        }
        RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "AiAgents.limitsSettings"
                text: qsTr("Limits…")
                onClicked: AiController.openLimits()
            }
        }
    }

    // ---- Connecting one -------------------------------------------------------------------
    SettingsGroup {
        objectName: "AiAgents.connect"
        title: qsTr("Connect an Agent")
        iconName: "plug"
        footer: agentTabs.currentIndex < 2
                ? qsTr("The first time it connects, you are asked to allow it here.")
                : qsTr("Add a client below to get a token.")
        AppTabBar {
            id: agentTabs
            objectName: "AiAgents.tabs"
            label: qsTr("Agent")
            Layout.fillWidth: true
            AppTabButton { objectName: "AiAgents.tab.desktop"; text: qsTr("Claude Desktop") }
            AppTabButton { objectName: "AiAgents.tab.code"; text: qsTr("Claude Code") }
            AppTabButton { objectName: "AiAgents.tab.other"; text: qsTr("Other Agents") }
        }
        Text {
            Layout.fillWidth: true
            text: agentTabs.currentIndex === 0 ? qsTr("Add this to claude_desktop_config.json, then start Claude Desktop again:")
                : agentTabs.currentIndex === 1 ? qsTr("Run this in a terminal:")
                : qsTr("An agent that starts a program: this command. One that connects over HTTP: the address, with the header “Authorization: Bearer” and a client's token.")
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        AiCopyField {
            objectName: "AiAgents.config"
            label: qsTr("Configuration")
            text: agentTabs.currentIndex === 0 ? AiController.desktopConfig
                : agentTabs.currentIndex === 1 ? AiController.codeCommand
                : AiController.relayCommand
        }
        AiCopyField {
            objectName: "AiAgents.url"
            visible: agentTabs.currentIndex === 2
            label: qsTr("Address")
            text: AiController.url
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing
            Text {
                Layout.fillWidth: true
                text: qsTr("An agent on another computer connects through SSH.")
                font: Theme.fontBody
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            AppButton {
                objectName: "AiAgents.otherComputer"
                text: qsTr("Use from Another Computer…")
                iconName: "external-link"
                onClicked: Shell.openUrl(AiController.otherComputerUrl)
            }
        }
    }

    // ---- Clients --------------------------------------------------------------------------
    SettingsGroup {
        objectName: "AiAgents.clients"
        title: qsTr("Clients")
        iconName: "key-round"
        footer: qsTr("Each agent has its own token. Removing a client stops its token at once. What it changed stays, and can be undone below.")
        Text {
            visible: AiController.clients.length === 0
            Layout.fillWidth: true
            text: qsTr("No clients yet.")
            font: Theme.fontBody
            color: Theme.textMuted
        }
        Repeater {
            model: AiController.clients
            delegate: RowLayout {
                id: clientRow
                required property var modelData
                required property int index
                objectName: "AiAgents.client." + index
                Layout.fillWidth: true
                spacing: 10
                Icon {
                    name: "bot"
                    size: 20
                    color: Theme.textMuted
                }
                ColumnLayout {
                    Layout.fillWidth: true
                    spacing: 2
                    Text {
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        text: clientRow.modelData.name
                        font: Theme.fontBody
                        color: Theme.text
                        elide: Text.ElideRight
                    }
                    Text {
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        text: clientRow.modelData.lastUsed > 0
                              //: %1 and %2 are dates
                              ? qsTr("Added %1, last used %2").arg(page.when(clientRow.modelData.created)).arg(page.when(clientRow.modelData.lastUsed))
                              : qsTr("Added %1, not used yet").arg(page.when(clientRow.modelData.created))
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                }
                IconButton {
                    objectName: "AiAgents.client." + clientRow.index + ".remove"
                    iconName: "circle-minus"
                    iconColor: Theme.dangerText
                    toolTip: qsTr("Remove this client")
                    onClicked: AiController.revoke(clientRow.modelData.id)
                }
            }
        }
        // The token of one just made, shown this once
        ColumnLayout {
            objectName: "AiAgents.newClient"
            visible: AiController.newClient.token !== undefined
            Layout.fillWidth: true
            spacing: 6
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                //: %1 is the name the user gave an AI agent
                text: qsTr("The token of “%1”. It is shown only now:").arg(AiController.newClient.name || "")
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.Wrap
            }
            AiCopyField {
                objectName: "AiAgents.token"
                label: qsTr("Token")
                text: AiController.newClient.token || ""
            }
            Text {
                Layout.fillWidth: true
                text: qsTr("For Claude Code over HTTP:")
                font: Theme.fontBody
                color: Theme.text
            }
            AiCopyField {
                objectName: "AiAgents.httpCommand"
                label: qsTr("Command")
                text: AiController.httpCommand
            }
            AppButton {
                objectName: "AiAgents.tokenDone"
                Layout.alignment: Qt.AlignRight
                text: qsTr("Done")
                onClicked: AiController.clearNewClient()
            }
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing
            BorderedField {
                id: newName
                objectName: "AiAgents.newName"
                label: qsTr("Client name")
                placeholderText: qsTr("Client name")
                Layout.fillWidth: true
                maximumLength: 48
            }
            AppButton {
                objectName: "AiAgents.add"
                text: qsTr("Add Client…")
                enabled: AiController.running && newName.text.trim().length > 0
                onClicked: {
                    AiController.addClient(newName.text)
                    newName.text = ""
                }
            }
        }
    }

    // ---- Activity -------------------------------------------------------------------------
    SettingsGroup {
        objectName: "AiAgents.activity"
        title: qsTr("Activity")
        iconName: "list-ordered"
        AiActivityList {
            prefix: "AiAgents.log"
            Layout.fillWidth: true
            Layout.preferredHeight: 300
        }
    }

    QuestionDialog {
        id: enableQuestion
        objectName: "AiAgents.enableQuestion"
        parent: page.Window.window ? page.Window.window.contentItem : null
        title: qsTr("Allow AI agents?")
        text: [qsTr("An agent on this computer that you allow may read your devices' settings and change them: equaliser, crossovers, gains, routing, effects, presets and more."),
               qsTr("It cannot update firmware, change the limits or the installer's lock, set up speakers, pair hubs or delete presets."),
               qsTr("You can undo each change, pause the agents or stop them at any time."),
               LimitsController.connected && !LimitsController.active
                   ? qsTr("This device has no limits profile. You can set one up first; it holds agents too.") : ""]
              .filter(s => s.length).join("\n\n")
        acceptText: qsTr("Allow")
        alternativeText: LimitsController.connected && !LimitsController.active ? qsTr("Set Up Limits…") : ""
        onAnswered: answer => {
            if (answer === "accept") AiController.setEnabled(true)
            else if (answer === "alternative") AiController.openLimits()
        }
    }
}
