import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP MCP4 -- Settings > AI Agents (both layouts): allowing AI agents on this
// computer to control the devices, how to connect one, the clients and their
// tokens, when they ask, and what they did.
// -- WP DL7 -- Modern (DESREV): the limits offer and an error as notice rows
// under the first section, one Limits button at a time, the agent picker as
// content-sized segments, warnings as notices instead of orange sentences,
// and an empty activity as an EmptyState. Classic keeps its rows.
SettingsPage {
    id: page
    objectName: "AiAgentsPage"
    readonly property alias enableDialog: enableQuestion
    readonly property alias detectDialog: detectQuestion   // -- WP ADETQ --
    readonly property alias rolesDialog: rolesQuestion     // -- WP ROLES --
    readonly property string rolesQuestionText: qsTr("When you tell an agent which output drives what (“tweeter, mid, woofer, left then right, outputs 1–6”), it may then set those roles without asking you first. They are not checked: the safety rules use them at once (a tweeter keeps its high-pass at 1 kHz or above), an output you did not name is treated as a tweeter, and before anything plays below 1 kHz on a midrange, woofer or sub the app checks that its top end is not a tweeter's. Settings › Speakers shows them as set by AI from your words until you confirm or correct them. An agent may also replace a setup you confirmed when your words say what changed, but a tweeter you set stays a tweeter for agents until you change it in Settings › Speakers. If a stated midrange, woofer or sub is silent above 1 kHz, its check plays the speaker detection's quiet probe (−50 dBFS or quieter, two octaves) below 1 kHz, even with “Agents may detect speakers” off.")   // -- WP ROLES -- REVDFIX (B1, S3)
    readonly property bool watching: visible && Window.window !== null && Window.window.visible
    onWatchingChanged: if (watching) AiController.refresh()
    Component.onDestruction: AiController.clearNewClient()

    // -- WP DL7 -- the offer of a limits profile shows (Classic: its row; Modern: a notice)
    readonly property bool offerLimits: AiController.enabled && LimitsController.connected && !LimitsController.active
    readonly property bool modern: LayoutController.modern

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
            visible: AiController.lastError.length > 0 && !page.modern   // -- WP DL7 --
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
            objectName: page.modern ? "" : "AiAgents.limitsOffer"   // -- WP DL7 -- Modern: the notice below
            visible: page.offerLimits && !page.modern
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

    // -- WP DL7 -- Modern: what goes wrong, and the limits offer, under the section
    NoticeRow {
        objectName: "AiAgents.errorNotice"
        visible: page.modern && AiController.lastError.length > 0
        Layout.fillWidth: true
        kind: "warning"
        text: AiController.lastError
        actionText: AiController.enabled && !AiController.running ? qsTr("Try Again") : ""
        onAction: AiController.dismissError()
    }
    NoticeRow {
        objectName: page.modern ? "AiAgents.limitsOffer" : ""
        visible: page.modern && page.offerLimits
        Layout.fillWidth: true
        text: qsTr("This device has no limits profile. One sets the most gain and volume anyone may set, agents included, and protects your speakers.")
        actionText: qsTr("Set Up Limits…")
        onAction: AiController.openLimits()
    }

    // ---- When they ask ------------------------------------------------------------------
    SettingsGroup {
        objectName: "AiAgents.permissions"
        title: qsTr("Ask Before Changes")
        iconName: "circle-question-mark"
        // -- WP DL7 -- Modern: Never's warning is a notice under the section
        footer: AiController.confirmMode === "never_ask" && page.modern ? ""
                : AiController.confirmMode === "never_ask"
                ? qsTr("Agents' changes are made at once, louder ones and test signals too. Your limits profile and the protection of outputs you set up as tweeters still apply.")
                : AiController.confirmMode === "ask_everything"
                ? qsTr("Every change waits for your answer.")
                : qsTr("Changes that make something louder, unmute or switch on an output, load or save a preset, or lower a tweeter's protection, and test signals, wait for your answer. Other changes are made at once.")
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
        // -- WP ADETQ -- (AUTODES 4.3) under Never the detection asks nothing at run
        // time: this switch, off by default, is its one question, with the warning
        SettingsToggle {
            id: detectToggle
            objectName: "AiAgents.detect"
            title: qsTr("Agents may detect speakers")
            caption: AiController.confirmMode === "never_ask"
                     ? qsTr("Under Never, an agent may play the speaker detection's quiet test sweeps without asking. Off, it is refused.")
                     : qsTr("Needed under Never only: with the other settings, each detection is asked, with its warning.")
            checked: AiController.agentDetect
            onToggled: on => {
                // The warning first; the switch shows the setting, never the click
                control.checked = Qt.binding(() => AiController.agentDetect)
                if (on) detectQuestion.open()
                else AiController.agentDetect = false
            }
        }
        // -- WP ROLES -- (owner 2026-11-23) speaker roles an agent sets from what you
        // told it: one question with the whole table, or under Never this switch
        SettingsToggle {
            objectName: "AiAgents.roles"
            title: qsTr("Agents may set speaker roles from my words")
            caption: AiController.confirmMode === "never_ask"
                     ? qsTr("Under Never, an agent may set which output drives what from what you told it, without asking, also over a setup you confirmed when your words say what changed; a tweeter you set stays a tweeter for agents. Its check may play the detection's quiet probe below 1 kHz on an output silent above it. Off, it is refused.")   // -- WP REVDFIX -- B1, S3
                     : qsTr("Needed under Never only: with the other settings, you are asked once, with the whole table.")
            checked: AiController.agentRoles
            onToggled: on => {
                control.checked = Qt.binding(() => AiController.agentRoles)
                if (on) {
                    rolesQuestion.text = page.rolesQuestionText
                    rolesQuestion.open()
                } else AiController.agentRoles = false
            }
        }
        SettingsValueRow {
            objectName: "AiAgents.limits"
            title: qsTr("Limits profile")
            value: !LimitsController.connected ? qsTr("No device connected")
                 : LimitsController.active ? qsTr("In force: agents are held to it") : qsTr("None")
        }
        RowLayout {
            // -- WP DL7 -- Modern: one Limits button, the notice's while it offers one
            visible: !(page.modern && page.offerLimits)
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "AiAgents.limitsSettings"
                text: qsTr("Limits…")
                onClicked: AiController.openLimits()
            }
        }
    }

    NoticeRow {   // -- WP DL7 --
        objectName: "AiAgents.neverNotice"
        visible: page.modern && AiController.confirmMode === "never_ask"
        Layout.fillWidth: true
        kind: "warning"
        text: qsTr("Agents' changes are made at once, louder ones and test signals too. Your limits profile and the protection of outputs you set up as tweeters still apply.")
    }

    // ---- -- WP FEATUI -- Agents' measurements, and the devices they may open ------------
    SettingsGroup {
        objectName: "AiAgents.measuring"
        title: qsTr("Agents' Measurements and Devices")
        iconName: "gauge"
        footer: qsTr("Agents never play louder than this: they may only ask for less, whatever Ask me says. It needs a microphone calibrated in dB SPL (this app's calibration is used when the agent names none): without one, an agent's measurement is refused. With the cap off, the level ceiling holds.")
        // -- WP INTG -- (RE1) the cap can be turned off
        SettingsToggle {
            objectName: "AiAgents.splCapOn"
            title: qsTr("Limit agents' measurements in dB SPL")
            caption: qsTr("Needs a calibrated microphone; off, only the level ceiling holds.")
            checked: AiController.agentSplCapOn
            onToggled: on => AiController.agentSplCapOn = on
        }
        SettingsSliderRow {
            objectName: "AiAgents.splCap"
            enabled: AiController.agentSplCapOn   // -- WP INTG --
            //: %1 a sound pressure level in dB SPL
            label: qsTr("Loudest measurement: %1 dB SPL").arg(AiController.agentSplCap)
            title: qsTr("Loudest measurement")   // -- WP DL7 -- Modern: the value in its field
            unit: "dB SPL"
            minimum: 60
            maximum: 95
            step: 1
            resetValue: 85
            value: AiController.agentSplCap
            onMoved: v => AiController.agentSplCap = Math.round(v)
        }
        SettingsValueRow {
            objectName: "AiAgents.remembered"
            title: qsTr("Devices agents may open")
            //: %n devices opened in this app before, which agents may ask to open again
            value: AiController.rememberedDevices.length === 0 ? qsTr("Only those open now")
                   : qsTr("%n remembered device(s), and those open now", "", AiController.rememberedDevices.length)
        }
        RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "AiAgents.forgetDevices"
                text: qsTr("Forget Devices")
                enabled: AiController.rememberedDevices.length > 0
                toolTip: qsTr("Forget the devices opened here before; agents then know only those open now and on USB")
                onClicked: AiController.forgetRememberedDevices()
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
            visible: !page.modern   // -- WP DL7 -- Modern: the segments below (this keeps the choice)
            label: qsTr("Agent")
            Layout.fillWidth: true
            AppTabButton { objectName: "AiAgents.tab.desktop"; text: qsTr("Claude Desktop") }
            AppTabButton { objectName: "AiAgents.tab.code"; text: qsTr("Claude Code") }
            AppTabButton { objectName: "AiAgents.tab.other"; text: qsTr("Other Agents") }
        }
        ToolSegments {   // -- WP DL7 -- grey, content-sized (DESIGN-LANGUAGE §7)
            objectName: "AiAgents.segments"
            visible: page.modern
            label: qsTr("Agent")
            model: [qsTr("Claude Desktop"), qsTr("Claude Code"), qsTr("Other Agents")]
            current: agentTabs.currentIndex
            onChosen: index => agentTabs.currentIndex = index
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
        AiGrantRows {   // -- WP ODEC --
            prefix: "AiAgents.grants"
            Layout.fillWidth: true
        }
        // -- WP DL7 -- Modern: an EmptyState while nothing is done (INTC: the list's own)
        AiActivityList {
            id: activityLog
            prefix: "AiAgents.log"
            emptyName: "AiAgents.logEmpty"
            Layout.fillWidth: true
            Layout.preferredHeight: activityLog.showsEmpty ? -1 : 300
        }
    }

    // -- WP ADETQ -- the never-ask users' one confirm box for the speaker detection
    QuestionDialog {
        id: detectQuestion
        objectName: "AiAgents.detectQuestion"
        parent: page.Window.window ? page.Window.window.contentItem : null
        title: qsTr("Let agents detect the speakers without asking?")
        text: [qsTr("An agent may then find out what is wired to a DSPi's outputs by playing quiet test sweeps on each output in turn, without asking you first. Each output is first played only above 1 kHz, at most −24 dBFS. Only an output whose response says it is not a tweeter is played lower, and then at most −50 dBFS. Whatever cannot be told apart is treated as a tweeter."),
               "⚠ " + AiController.detectWarning].join("\n\n")
        acceptText: qsTr("I Understand")
        onAnswered: answer => { if (answer === "accept") AiController.agentDetect = true }
    }

    // -- WP ROLES -- the never-ask users' one question for speaker roles from their words
    QuestionDialog {
        id: rolesQuestion
        objectName: "AiAgents.rolesQuestion"
        parent: page.Window.window ? page.Window.window.contentItem : null
        title: qsTr("Let agents set speaker roles from your words without asking?")
        // (its text is given when it opens: a long text given at creation loops the
        // dialog's implicitHeight, QuestionDialog's note)
        acceptText: qsTr("I Understand")
        onAnswered: answer => { if (answer === "accept") AiController.agentRoles = true }
    }

    AiEnableQuestion {   // -- WP LAY8 -- the popover asks it too
        id: enableQuestion
        objectName: "AiAgents.enableQuestion"
        parent: page.Window.window ? page.Window.window.contentItem : null
    }
}
