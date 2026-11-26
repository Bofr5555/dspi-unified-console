import QtQuick
import UConsole

// -- WP MCP4 -- The main window's place for what AI agents ask the user (both
// layouts): a change held for confirmation, with what it is in words, and an
// agent that asks to pair. One question at a time, pairing first. While the
// app is not in front, its window asks for attention; one hidden in the tray
// comes back, as a question nobody sees is never answered.
Item {
    id: host
    objectName: "AiHost"
    anchors.fill: parent

    readonly property var pairing: AiController.pairing
    readonly property var front: AiController.confirmations.length ? AiController.confirmations[0] : null
    // -- WP UAG -- (UXP D2) the questions one agent raised together are one sheet, a row per device and
    // change. "Answer One by One" splits them.
    // -- WP UB5 -- (UXP B5) the group is the server's (dspi-mcp confirm.rs): it comes here only once
    // sealed and never takes another question, so the sheet's rows never grow while it is up and one
    // Allow answers exactly the questions shown; one raised meanwhile is the next sheet.
    property bool oneByOne: false
    readonly property var group: {
        if (!front) return []
        if (oneByOne) return [front]
        return AiController.confirmations.filter(c => c.group === front.group)
    }
    readonly property bool grouped: group.length > 1
    Connections {
        target: AiController
        function onConfirmationsChanged() { if (!AiController.confirmations.length) host.oneByOne = false }
    }
    readonly property int more: Math.max(0, AiController.confirmations.length - Math.max(1, group.length))
    readonly property bool asking: AiController.state === "asking"
    onAskingChanged: {
        if (!asking) return
        if (TrayController.windowHidden) TrayController.restore()
        else if (Window.window && !Window.window.active) Window.window.alert(0)
    }
    // "1:59"
    function clock(seconds) {
        return Math.floor(seconds / 60) + ":" + String(seconds % 60).padStart(2, "0")
    }
    // -- WP UAG -- (UXP A6) the rows of a question: the whole call (the asked ones marked), else the risks
    function rowsOf(c) {
        return c.batch && c.batch.length ? c.batch : c.riskLines.map(line => ({ said: line, risky: true }))
    }
    // -- WP UAG -- (UXP D2) a grouped sheet: each device once, then its questions' rows
    // -- WP UB5 -- (UXP B5) each device once also when the calls came in mixed: by device, in the
    // order the devices first asked
    readonly property var byDevice: {
        const devices = group.map(c => c.device).filter((d, i, all) => all.indexOf(d) === i)
        return devices.reduce((all, d) => all.concat(group.filter(c => c.device === d)), [])
    }
    readonly property var rows: !front ? []
        : grouped ? byDevice.reduce((all, c, i) => all.concat(i > 0 && byDevice[i - 1].device === c.device ? []
                                                              : [{ said: c.device, risky: false, head: true }], rowsOf(c)), [])
        : rowsOf(front)
    // What a question's Allow leaves to do afterwards (-- WP UAG --: one function for one or a group)
    function afterOf(c) {
        // -- WP FEATUI -- (FUNC1 section 4) sound cannot be undone: a test signal is not promised an undo
        // -- WP FUNC2c -- (FSTORY S7) nor a preset load: it ends the undo history; the snapshot is the way back
        return c.sound
               ? qsTr("Nothing has played yet. Sound cannot be taken back once it plays; Stop Measuring in the agent strip ends it.")
               : c.wholesale
               ? qsTr("Nothing has been changed yet. This replaces the settings as a whole and ends the undo history; the settings as they are now are kept as a snapshot first, to go back to.")
               // -- WP ROLES -- the roles are corrected where they live, not undone
               : c.roleCells.length
               ? qsTr("Nothing has been changed yet. You can confirm or correct the roles afterwards in Settings › Speakers.")
               // -- WP UAG -- (UXP A6) a flash write has no Undo: the sheet says so instead of promising one
               : c.flash
               ? qsTr("Nothing has been changed yet. This is written to the device's memory; Undo cannot take it back.")
               : qsTr("Nothing has been changed yet. You can undo it afterwards in the agents' activity.")
    }
    // Each sentence once
    function once(texts) {
        return texts.filter((t, i) => t.length && texts.indexOf(t) === i)
    }
    readonly property int rowsShown: 12   // -- WP UB5 -- (UXP B5) was 8: a grouped sheet names its devices too
    // -- WP UAG -- (UXP A6) the speaker detection's mechanism waits under Details
    property bool detailsOpen: false
    onFrontChanged: detailsOpen = false
    readonly property bool detailsShown: !front || !front.detect || detailsOpen

    AiSheet {
        id: pairingSheet
        objectName: "AiPairingSheet"
        prefix: "AiPairingSheet"
        parent: host.parent
        wanted: host.pairing.id !== undefined
        question: host.pairing.id
        heading: qsTr("Allow this AI agent?")
        onAnswered: allow => AiController.answerPairing(allow)

        SheetText {
            objectName: "AiPairingSheet.name"
            //: %1 is the AI agent's own name, %2 the app's name
            text: qsTr("“%1” on this computer wants to control your DSPi devices through %2.").arg(host.pairing.name || "").arg(AppIdentity.displayName)   // -- WP UAG --
        }
        Text {
            objectName: "AiPairingSheet.code"
            width: parent.width
            horizontalAlignment: Text.AlignHCenter
            textFormat: Text.PlainText
            text: host.pairing.code || ""
            font: Qt.font({ family: Shell.monospaceFamily, pixelSize: 30, weight: Font.DemiBold, letterSpacing: 6 })
            color: Theme.text
            Accessible.role: Accessible.StaticText
            Accessible.name: qsTr("Pairing code")
            Accessible.description: text
        }
        // -- WP UAG -- (UXP A9) where the code shows: the relay puts it in the agent's chat and its log
        SheetText {
            objectName: "AiPairingSheet.where"
            text: qsTr("Allow it only if you have just added an agent on this computer. The agent's chat or its log shows the same code.")
        }
        SheetText {
            objectName: "AiPairingSheet.left"
            font: Theme.fontSmall
            color: Theme.textMuted
            //: %1 is the time left ("1:59")
            text: qsTr("If you don't answer within %1, the agent is not added.").arg(host.clock(host.pairing.secondsLeft || 0))
        }
    }

    AiSheet {
        id: confirmSheet
        objectName: "AiConfirmSheet"
        prefix: "AiConfirmSheet"
        parent: host.parent
        wanted: host.front !== null && !pairingSheet.wanted
        question: host.front ? host.group.map(c => c.id).join(",") : undefined   // -- WP UAG -- (UXP D2)
        // -- WP ADETQ -- the speaker detection's question (AUTODES 4.1) says what it is
        //: %1 a device's name
        heading: host.front && host.front.detect ? qsTr("Detect the speakers on %1?").arg(host.front.device)
                 // -- WP ROLES -- speaker roles from the person's words
                 //: %1 a device's name
                 : host.front && host.front.roleCells.length ? qsTr("Set the speaker roles on %1?").arg(host.front.device)
                 // -- WP UAG -- (UXP D2) questions raised together
                 //: %1 is the AI agent's name, %n how many changes it asks about at once
                 : host.grouped ? qsTr("%1 wants to make %n changes", "", host.group.length).arg(host.front.agent)
                 // -- WP UAG -- (UXP A6) the change in one line: "Claude Code wants to make Right tweeter louder (Fronts)"
                 : host.front && host.front.heading ? host.front.heading
                 : qsTr("Allow this change?")
        // -- WP UAG -- (UXP D2) exactly the questions shown
        // -- WP UB5 -- all of them at once (answering one by one changed `group` under the loop)
        onAnswered: allow => AiController.answerAll(host.group.map(c => c.id), allow)

        // -- WP UAG -- (UXP A6, C11) who asks, where the heading does not say it: the detection's and the roles'
        // questions (the agent's name without the "AI ·" the sheet already shows)
        SheetText {
            objectName: "AiConfirmSheet.who"
            visible: text.length > 0
            text: !host.front || (!host.front.detect && !host.front.roleCells.length) ? ""
                //: %1 is the AI agent's name
                : qsTr("Asked by %1.").arg(host.front.agent)
        }
        // -- WP UAG -- (UXP A6) the speaker detection: what the person does, first; how it works under Details
        SheetText {
            objectName: "AiConfirmSheet.todo"
            visible: host.front !== null && host.front.detect
            leftPadding: Theme.spacing
            text: ["• " + qsTr("Set up the microphone as for a measurement (usually at your seat)."),
                   "• " + qsTr("Keep the volume where it is."),
                   "• " + qsTr("Stay by Stop Measuring in the agent strip: it ends the sound at once."),
                   "• " + qsTr("Afterwards check the result in Settings › Speakers and confirm or correct it.")].join("\n")
        }
        AppButton {
            objectName: "AiConfirmSheet.details"
            visible: host.front !== null && host.front.detect
            text: host.detailsOpen ? qsTr("Hide Details") : qsTr("Details")
            onClicked: host.detailsOpen = !host.detailsOpen
        }
        // -- WP UAG -- (UXP D2) a grouped sheet can be answered one question at a time instead
        AppButton {
            objectName: "AiConfirmSheet.oneByOne"
            visible: host.grouped
            text: qsTr("Answer One by One")
            onClicked: host.oneByOne = true
        }
        // -- WP UAG -- (UXP A6) the whole call in the screen's words, the asked rows marked
        Column {
            width: parent.width
            spacing: 4
            visible: host.detailsShown
            Repeater {
                // -- WP ROLES -- (the speaker roles' question shows its table below instead)
                model: host.front && !host.front.roleCells.length ? host.rows.slice(0, host.rowsShown) : []
                delegate: Item {
                    id: row
                    required property var modelData
                    required property int index
                    width: parent.width
                    implicitHeight: rowText.implicitHeight
                    height: implicitHeight
                    Text {
                        id: mark
                        x: Theme.spacing
                        width: Theme.spacingLarge
                        textFormat: Text.PlainText
                        font: Theme.fontBody
                        color: Theme.warning
                        text: row.modelData.risky ? "▲" : ""
                        Accessible.ignored: true
                    }
                    SheetText {
                        id: rowText
                        objectName: "AiConfirmSheet.risk." + row.index
                        x: row.modelData.head ? Theme.spacing : mark.x + mark.width   // -- WP UAG -- (UXP D2) a device
                        width: row.width - x
                        font: row.modelData.head ? Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold }) : Theme.fontBody
                        text: row.modelData.said
                        //: Read out before an AI agent's change that the question is about; %1 the change
                        Accessible.name: row.modelData.risky ? qsTr("Asked: %1").arg(text) : text
                    }
                }
            }
            SheetText {
                visible: host.rows.length > host.rowsShown && !(host.front && host.front.roleCells.length)
                leftPadding: Theme.spacing + Theme.spacingLarge
                text: qsTr("and %1 more").arg(Math.max(0, host.rows.length - host.rowsShown))
            }
        }
        // -- WP ROLES -- the whole table of speaker roles from the person's words, then the rest
        Grid {
            objectName: "AiConfirmSheet.roles"
            visible: host.front !== null && host.front.roleCells.length > 0
            columns: 3
            columnSpacing: Theme.spacing * 2
            rowSpacing: 2
            leftPadding: Theme.spacing
            Repeater {
                model: host.front && host.front.roleCells.length
                       ? [qsTr("Output"), qsTr("Speaker"), qsTr("Role")].concat(host.front.roleCells) : []
                delegate: Text {
                    required property string modelData
                    required property int index
                    objectName: "AiConfirmSheet.roles." + index
                    textFormat: Text.PlainText
                    font: index < 3 ? Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold }) : Theme.fontBody
                    color: Theme.text
                    text: modelData
                    Accessible.role: Accessible.StaticText
                    Accessible.name: text
                }
            }
        }
        SheetText {
            objectName: "AiConfirmSheet.rolesAfter"
            visible: text.length > 0
            text: host.front && host.front.rolesAfter ? host.front.rolesAfter : ""
        }
        // -- WP ADETQ -- the warning on the speaker detection's mechanism (the server's words)
        SheetText {
            objectName: "AiConfirmSheet.warning"
            visible: text.length > 0 && host.detailsShown   // -- WP UAG --
            color: Theme.warning
            text: host.front && host.front.warning ? "⚠ " + host.front.warning : ""
        }
        // -- WP ODEC -- one question for an agent's test signals on the device, for a while
        SheetText {
            objectName: "AiConfirmSheet.grant"
            visible: text.length > 0
            font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
            text: host.once(host.group.map(c => c.grantText || "")).join(" ")   // -- WP UAG -- (UXP D2)
        }
        SheetText {
            objectName: "AiConfirmSheet.after"
            text: host.once(host.group.map(c => host.afterOf(c))).join(" ")   // -- WP UAG -- (UXP D2) each once
        }
        SheetText {
            objectName: "AiConfirmSheet.left"
            font: Theme.fontSmall
            color: Theme.textMuted
            // -- WP UAG -- (UXP A6) what happens at zero, not only the clock
            //: %1 is the time left ("1:59")
            text: [qsTr("If you don't answer within %1, nothing is changed and the agent is told so.")
                       .arg(host.clock(AiController.confirmationSecondsLeft)),
                   host.more > 0 ? qsTr("%1 more waiting").arg(host.more) : ""].filter(s => s.length).join(" · ")
        }
    }

    component SheetText: Text {
        width: parent ? parent.width : 0
        textFormat: Text.PlainText
        font: Theme.fontBody
        color: Theme.text
        wrapMode: Text.Wrap
        Accessible.role: Accessible.StaticText
        Accessible.name: text
    }
}
