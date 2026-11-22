import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP INSTQ -- A unit the hub sees that is not in the installation, with
// what it joins as (Join): its name, role, room and feed. With an empty
// `serial` the serial is typed (a hub with no inventory).
Card {
    id: card
    required property string serial
    property bool editable: false
    readonly property var unit: InstallationController.unseenUnit(serial, InstallationController.generation)
    readonly property string joinAs: asChoice.value
    readonly property bool fed: ["receiver", "repeater", "fan_out", "spdif_follower"].indexOf(joinAs) >= 0
    readonly property string issueId: "serial:" + (serial.length ? serial : serialField.text.trim())
    function issueOf(field) { return InstallationController.issueOf(issueId, field, InstallationController.generation) }
    objectName: "Installation.unseen." + serial

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        BorderedField {
            id: nameField
            objectName: "Installation.joinName." + card.serial
            implicitWidth: 200
            enabled: card.editable
            maximumLength: 64
            label: card.serial.length ? qsTr("Name to join %1 as").arg(card.unit.shownSerial || "") : qsTr("Name of the unit to join")
            placeholderText: qsTr("Name")
            Component.onCompleted: text = card.unit.name || ""
        }
        BorderedField {
            id: serialField
            objectName: "Installation.joinSerial"
            visible: card.serial.length === 0
            implicitWidth: 200
            enabled: card.editable
            maximumLength: 64
            label: qsTr("Serial of the unit to join")
            placeholderText: qsTr("Serial")
        }
        Text {
            Layout.fillWidth: true
            text: card.serial.length === 0 ? ""
                  : (card.unit.online ? qsTr("%1 · online").arg(card.unit.shownSerial || "") : qsTr("%1 · offline").arg(card.unit.shownSerial || ""))
            textFormat: Text.PlainText
            font: Theme.fontSmall
            color: Theme.textMuted
            elide: Text.ElideRight
        }
        AppButton {
            objectName: "Installation.join." + card.serial
            text: qsTr("Join")
            primary: true
            enabled: card.editable
            toolTip: qsTr("Add it to the installation")
            onClicked: {
                const serial = card.serial.length ? card.serial : serialField.text.trim()
                if (InstallationController.join(serial, nameField.text, card.joinAs, roomChoice.value, card.fed ? feedChoice.value : "")
                    && card.serial.length === 0) {
                    serialField.text = ""
                    nameField.text = ""
                }
            }
        }
    }
    Flow {
        Layout.fillWidth: true
        spacing: Theme.spacing
        InstallationField {
            title: qsTr("Join as")
            InstallationChoice {
                id: asChoice
                objectName: "Installation.joinAs." + card.serial
                label: qsTr("What %1 joins as").arg(nameField.text)
                enabled: card.editable
                value: "receiver"
                choices: [
                    { id: "receiver", name: qsTr("Receiver") },
                    { id: "repeater", name: qsTr("Repeater") },
                    { id: "spdif_follower", name: qsTr("S/PDIF follower") },
                    { id: "spdif_source", name: qsTr("S/PDIF source") },
                    { id: "usb_direct", name: qsTr("USB-direct") },
                    { id: "sender", name: qsTr("Sender") },
                    { id: "fan_out", name: qsTr("Fan-out") }
                ]
                onChosen: id => { value = id; feedChoice.value = "" }
            }
        }
        InstallationField {
            visible: card.joinAs !== "sender" && card.joinAs !== "fan_out"
            title: qsTr("Room")
            InstallationChoice {
                id: roomChoice
                objectName: "Installation.joinRoom." + card.serial
                label: qsTr("Room of %1").arg(nameField.text)
                enabled: card.editable
                choices: InstallationController.roomChoices(InstallationController.generation)
                onChosen: id => value = id
                // Its room was removed meanwhile
                onChoicesChanged: if (choices.findIndex(c => c.id === value) < 0) value = ""
            }
        }
        InstallationField {
            visible: card.fed
            title: qsTr("Fed by")
            issue: card.issueOf("upstream")
            issueName: "Installation.joinUpstreamIssue." + card.serial
            InstallationChoice {
                id: feedChoice
                objectName: "Installation.joinUpstream." + card.serial
                label: qsTr("What feeds %1").arg(nameField.text)
                enabled: card.editable
                choices: InstallationController.feedChoices(card.joinAs, "", "", InstallationController.generation)
                onChosen: id => value = id
                onChoicesChanged: if (choices.findIndex(c => c.id === value) < 0) value = ""
            }
        }
    }
    Text {
        objectName: "Installation.joinIssue." + card.serial
        Layout.fillWidth: true
        visible: text.length > 0
        text: card.issueOf("") || card.issueOf("serial")
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.dangerText
        wrapMode: Text.Wrap
        Accessible.role: Accessible.AlertMessage
        Accessible.name: text
    }
}
