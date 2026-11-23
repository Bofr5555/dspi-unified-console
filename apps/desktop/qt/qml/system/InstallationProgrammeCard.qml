import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP FEATUI -- A programme source (FEAT1): where the music comes from (a
// computer over USB, a box on S/PDIF or analogue, or into the sender) and
// which unit or sender inputs it plays on. A description only: the hub writes
// nothing from it; agents read it to know what feeds what.
Card {
    id: card
    required property string programmeId
    property bool editable: false
    readonly property var programme: InstallationController.element(programmeId, InstallationController.generation)
    readonly property string kind: programme.programmeKind || "computer_usb"
    function issueOf(field) { return InstallationController.issueOf(programmeId, field, InstallationController.generation) }
    objectName: "Installation.programme." + programmeId
    Accessible.name: programme.name || ""

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        InstallationName {
            objectName: "Installation.name." + card.programmeId
            label: qsTr("Name of programme source %1").arg(card.programme.name || "")
            elementId: card.programmeId
            name: card.programme.name || ""
            editable: card.editable
        }
        Item { Layout.fillWidth: true }
        AppButton {
            objectName: "Installation.remove." + card.programmeId
            visible: card.editable
            text: qsTr("Remove")
            toolTip: qsTr("Take %1 out of the installation").arg(card.programme.name || "")
            onClicked: InstallationController.remove(card.programmeId)
        }
    }
    Flow {
        Layout.fillWidth: true
        spacing: Theme.spacing
        InstallationField {
            title: qsTr("Comes in by")
            InstallationChoice {
                objectName: "Installation.programmeKind." + card.programmeId
                label: qsTr("How %1 comes in").arg(card.programme.name || "")
                enabled: card.editable
                choices: InstallationController.programmeKinds()
                value: card.kind
                // Another kind may feed another kind of element: chosen again below
                onChosen: id => InstallationController.setProgramme(card.programmeId, id,
                                                                    (id === "alink_sender") === (card.kind === "alink_sender") ? card.programme.feeds : "",
                                                                    card.programme.inputs || "0")
            }
        }
        InstallationField {
            title: card.kind === "alink_sender" ? qsTr("Into sender") : qsTr("Into unit")
            issue: card.issueOf("feeds")
            issueName: "Installation.feedsIssue." + card.programmeId
            InstallationChoice {
                objectName: "Installation.feeds." + card.programmeId
                label: qsTr("What %1 feeds").arg(card.programme.name || "")
                enabled: card.editable
                choices: InstallationController.programmeFeedChoices(card.kind, card.programme.feeds || "", InstallationController.generation)
                value: card.programme.feeds || ""
                onChosen: id => InstallationController.setProgramme(card.programmeId, card.kind, id, card.programme.inputs || "0")
            }
        }
        InstallationField {
            title: card.kind === "alink_sender" ? qsTr("Sender channels") : qsTr("Inputs")
            issue: card.issueOf("inputs")
            issueName: "Installation.inputsIssue." + card.programmeId
            BorderedField {
                id: inputs
                objectName: "Installation.inputs." + card.programmeId
                implicitWidth: 120
                enabled: card.editable
                label: qsTr("Inputs %1 plays on").arg(card.programme.name || "")
                function sync() { if (!activeFocus) text = card.programme.inputs || "" }
                Component.onCompleted: sync()
                Connections {
                    target: InstallationController
                    function onDraftChanged() { inputs.sync() }
                }
                onCommitted: t => { if (!InstallationController.setProgramme(card.programmeId, card.kind, card.programme.feeds || "", t)) text = card.programme.inputs || "" }
            }
        }
    }
    Text {
        objectName: "Installation.issue." + card.programmeId
        Layout.fillWidth: true
        visible: text.length > 0
        text: card.issueOf("name") || card.issueOf("")
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.dangerText
        wrapMode: Text.Wrap
        Accessible.role: Accessible.AlertMessage
        Accessible.name: text
    }
}
