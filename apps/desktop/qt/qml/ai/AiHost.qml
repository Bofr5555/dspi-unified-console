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
    readonly property int more: Math.max(0, AiController.confirmations.length - 1)
    readonly property bool asking: AiController.state === "asking"
    onAskingChanged: {
        if (!asking) return
        if (TrayController.windowHidden) TrayController.restore()
        else if (Window.window && !Window.window.active) Window.window.alert(0)
    }
    // "1:59"
    function timeLeft(seconds) {
        return qsTr("Time left: %1").arg(Math.floor(seconds / 60) + ":" + String(seconds % 60).padStart(2, "0"))
    }

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
            text: qsTr("“%1” wants to control your DSPi devices through %2.").arg(host.pairing.name || "").arg(AppIdentity.displayName)
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
        SheetText {
            text: qsTr("Allow it only if you have just connected an agent on this computer. Its log shows the same code.")
        }
        SheetText {
            font: Theme.fontSmall
            color: Theme.textMuted
            text: host.timeLeft(host.pairing.secondsLeft || 0)
        }
    }

    AiSheet {
        id: confirmSheet
        objectName: "AiConfirmSheet"
        prefix: "AiConfirmSheet"
        parent: host.parent
        wanted: host.front !== null && !pairingSheet.wanted
        question: host.front ? host.front.id : undefined
        // -- WP ADETQ -- the speaker detection's question (AUTODES 4.1) says what it is
        //: %1 a device's name
        heading: host.front && host.front.detect ? qsTr("Detect the speakers on %1?").arg(host.front.device)
                 // -- WP ROLES -- speaker roles from the person's words
                 //: %1 a device's name
                 : host.front && host.front.roleCells.length ? qsTr("Set the speaker roles on %1?").arg(host.front.device)
                 : qsTr("Allow this change?")
        onAnswered: allow => { if (host.front) AiController.answer(host.front.id, allow) }

        SheetText {
            objectName: "AiConfirmSheet.who"
            text: !host.front ? ""
                : host.front.device.length
                //: %1 is the AI agent's name, %2 what it asks to do ("Set a gain"), %3 the device's name
                ? qsTr("%1 asks: %2, on %3.").arg(host.front.client).arg(host.front.title).arg(host.front.device)
                //: %1 is the AI agent's name, %2 what it asks to do ("Set a gain")
                : qsTr("%1 asks: %2.").arg(host.front.client).arg(host.front.title)
        }
        Column {
            width: parent.width
            spacing: 4
            Repeater {
                // -- WP ROLES -- (the speaker roles' question shows its table below instead)
                model: host.front && !host.front.roleCells.length ? host.front.riskLines.slice(0, 8) : []   // -- WP LAY8 -- with its kind
                delegate: SheetText {
                    required property string modelData
                    required property int index
                    objectName: "AiConfirmSheet.risk." + index
                    leftPadding: Theme.spacing
                    // -- WP FIN015 -- (REVF D1) on the open device, "Out 1 (Centre woofer)" as its pages
                    // name it; the agent's words stay the core's
                    text: "• " + (host.front && host.front.deviceId === DeviceRegistry.activeId && DeviceModel.revision >= 0
                                  ? DeviceModel.channels.withNames(modelData) : modelData)
                }
            }
            SheetText {
                visible: host.front !== null && host.front.risks.length > 8
                leftPadding: Theme.spacing
                text: host.front ? qsTr("and %1 more").arg(Math.max(0, host.front.risks.length - 8)) : ""
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
            visible: text.length > 0
            color: Theme.warning
            text: host.front && host.front.warning ? "⚠ " + host.front.warning : ""
        }
        // -- WP ODEC -- one question for an agent's test signals on the device, for a while
        SheetText {
            objectName: "AiConfirmSheet.grant"
            visible: text.length > 0
            font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
            text: host.front && host.front.grantText ? host.front.grantText : ""
        }
        SheetText {
            objectName: "AiConfirmSheet.after"
            // -- WP FEATUI -- (FUNC1 section 4) sound cannot be undone: a test signal is not promised an undo
            // -- WP FUNC2c -- (FSTORY S7) nor a preset load: it ends the undo history; the snapshot is the way back
            text: host.front && host.front.sound
                  ? qsTr("Nothing has played yet. Sound cannot be taken back once it plays; Stop Measuring in the agent strip ends it.")
                  : host.front && host.front.wholesale
                  ? qsTr("Nothing has been changed yet. This replaces the settings as a whole and ends the undo history; the settings as they are now are kept as a snapshot first, to go back to.")
                  // -- WP ROLES -- the roles are corrected where they live, not undone
                  : host.front && host.front.roleCells.length
                  ? qsTr("Nothing has been changed yet. You can confirm or correct the roles afterwards in Settings › Speakers.")
                  : qsTr("Nothing has been changed yet. You can undo it afterwards in the agents' activity.")
        }
        SheetText {
            objectName: "AiConfirmSheet.left"
            font: Theme.fontSmall
            color: Theme.textMuted
            text: [host.timeLeft(AiController.confirmationSecondsLeft),
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
