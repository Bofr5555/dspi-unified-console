import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP INSTQ -- An element with a name and, for a zone its rooms, for a group
// its units; also a room, a sender or a fan-out, which have none.
Card {
    id: card
    required property string elementId
    property bool editable: false
    property var memberIds: []          // what it may hold: room ids, unit ids
    readonly property var element: InstallationController.element(elementId, InstallationController.generation)
    readonly property var members: element.kind === "zone" ? element.rooms : element.kind === "group" ? element.units : []
    readonly property string issue: InstallationController.issueOf(elementId, "name", InstallationController.generation)
                                     || InstallationController.issueOf(elementId, "", InstallationController.generation)
                                     || InstallationController.issueOf(elementId, "upstream", InstallationController.generation)
    // After the name: a fan-out's feed, a zone's source, a serial
    property string note
    objectName: "Installation.element." + elementId
    Accessible.name: element.name || ""

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        InstallationName {
            objectName: "Installation.name." + card.elementId
            label: qsTr("Name of %1").arg(card.element.name || "")
            elementId: card.elementId
            name: card.element.name || ""
            editable: card.editable
        }
        Text {
            Layout.fillWidth: true
            text: card.note
            textFormat: Text.PlainText
            font: Theme.fontSmall
            color: Theme.textMuted
            elide: Text.ElideRight
        }
        AppButton {
            objectName: "Installation.remove." + card.elementId
            visible: card.editable
            text: qsTr("Remove")
            toolTip: qsTr("Take %1 out of the installation").arg(card.element.name || "")
            onClicked: InstallationController.remove(card.elementId)
        }
    }
    Flow {
        Layout.fillWidth: true
        visible: card.memberIds.length > 0
        spacing: Theme.spacing
        Repeater {
            model: card.memberIds
            delegate: AppCheckBox {
                required property string modelData
                readonly property string memberName: InstallationController.element(modelData, InstallationController.generation).name || ""
                objectName: "Installation.member." + card.elementId + "." + modelData
                text: memberName
                enabled: card.editable
                checked: card.members.indexOf(modelData) >= 0
                Accessible.name: qsTr("%1 in %2").arg(memberName).arg(card.element.name || "")
                onToggled: {
                    InstallationController.setMember(card.elementId, modelData, checked)
                    checked = Qt.binding(() => card.members.indexOf(modelData) >= 0)
                }
            }
        }
    }
    Text {
        objectName: "Installation.issue." + card.elementId
        Layout.fillWidth: true
        visible: text.length > 0
        text: card.issue
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.dangerText
        wrapMode: Text.Wrap
        Accessible.role: Accessible.AlertMessage
        Accessible.name: text
    }
}
