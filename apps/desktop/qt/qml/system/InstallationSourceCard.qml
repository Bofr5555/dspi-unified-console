import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP FEATUI -- An audio link source (FEAT1): its name, the sender it comes
// from and its channels on the link. A zone that plays it routes its units'
// slots to these channels.
Card {
    id: card
    required property string sourceId
    property bool editable: false
    readonly property var source: InstallationController.element(sourceId, InstallationController.generation)
    function issueOf(field) { return InstallationController.issueOf(sourceId, field, InstallationController.generation) }
    objectName: "Installation.source." + sourceId
    Accessible.name: source.name || ""

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        InstallationName {
            objectName: "Installation.name." + card.sourceId
            label: qsTr("Name of source %1").arg(card.source.name || "")
            elementId: card.sourceId
            name: card.source.name || ""
            editable: card.editable
        }
        Item { Layout.fillWidth: true }
        AppButton {
            objectName: "Installation.remove." + card.sourceId
            visible: card.editable
            text: qsTr("Remove")
            toolTip: qsTr("Take %1 out of the installation; zones that play it play nothing chosen").arg(card.source.name || "")
            onClicked: InstallationController.remove(card.sourceId)
        }
    }
    Flow {
        Layout.fillWidth: true
        spacing: Theme.spacing
        InstallationField {
            title: qsTr("From")
            InstallationChoice {
                objectName: "Installation.sender." + card.sourceId
                label: qsTr("Sender of %1").arg(card.source.name || "")
                enabled: card.editable
                choices: InstallationController.senderChoices(InstallationController.generation)
                value: card.source.sender || ""
                onChosen: id => InstallationController.setSource(card.sourceId, id, card.source.channels || "")
            }
        }
        InstallationField {
            title: qsTr("Channels on the audio link")
            issue: card.issueOf("channels")
            issueName: "Installation.channelsIssue." + card.sourceId
            BorderedField {
                id: channels
                objectName: "Installation.channels." + card.sourceId
                implicitWidth: 140
                enabled: card.editable
                label: qsTr("Channels of %1").arg(card.source.name || "")
                function sync() { if (!activeFocus) text = card.source.channels || "" }
                Component.onCompleted: sync()
                Connections {
                    target: InstallationController
                    function onDraftChanged() { channels.sync() }
                }
                onCommitted: t => { if (!InstallationController.setSource(card.sourceId, card.source.sender || "", t)) text = card.source.channels || "" }
            }
        }
    }
    Text {
        objectName: "Installation.issue." + card.sourceId
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
