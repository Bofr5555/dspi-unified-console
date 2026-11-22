import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP INSTQ -- A unit of the installation: its name, room, role, and what
// feeds it (a source's time-base outputs instead). The issues are on their
// fields. Everything is the draft's (InstallationController).
Card {
    id: card
    required property string unitId
    property bool editable: false
    readonly property var unit: InstallationController.element(unitId, InstallationController.generation)
    readonly property string role: unit.role || ""
    readonly property bool fed: role !== "usb_direct" && role !== "spdif_source"
    function issueOf(field) { return InstallationController.issueOf(unitId, field, InstallationController.generation) }
    objectName: "Installation.unit." + unitId
    property string shownName: (unit.name || "") + ", " + (roles.find(r => r.id === role)?.name || "")
    readonly property var roles: [
        { id: "receiver", name: qsTr("Receiver") },
        { id: "repeater", name: qsTr("Repeater") },
        { id: "spdif_follower", name: qsTr("S/PDIF follower") },
        { id: "spdif_source", name: qsTr("S/PDIF source") },
        { id: "usb_direct", name: qsTr("USB-direct") }
    ]
    Accessible.name: shownName

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        InstallationName {
            objectName: "Installation.name." + card.unitId
            label: qsTr("Name of unit %1").arg(card.unit.name || "")
            elementId: card.unitId
            name: card.unit.name || ""
            editable: card.editable
        }
        Text {
            Layout.fillWidth: true
            text: card.unit.serial || ""
            textFormat: Text.PlainText
            font: Theme.fontSmall
            color: Theme.textMuted
            elide: Text.ElideRight
        }
        AppButton {
            objectName: "Installation.remove." + card.unitId
            visible: card.editable
            text: qsTr("Remove")
            toolTip: qsTr("Take %1 out of the installation").arg(card.unit.name || "")
            onClicked: InstallationController.remove(card.unitId)
        }
    }
    Flow {
        Layout.fillWidth: true
        spacing: Theme.spacing
        InstallationField {
            title: qsTr("Room")
            issue: card.issueOf("room")
            issueName: "Installation.roomIssue." + card.unitId
            InstallationChoice {
                objectName: "Installation.room." + card.unitId
                label: qsTr("Room of %1").arg(card.unit.name || "")
                enabled: card.editable
                choices: InstallationController.roomChoices(InstallationController.generation)
                value: card.unit.room || ""
                onChosen: id => InstallationController.setRoom(card.unitId, id)
            }
        }
        InstallationField {
            title: qsTr("Role")
            issue: card.issueOf("role")
            issueName: "Installation.roleIssue." + card.unitId
            InstallationChoice {
                objectName: "Installation.role." + card.unitId
                label: qsTr("Role of %1").arg(card.unit.name || "")
                enabled: card.editable
                choices: card.roles
                value: card.role
                onChosen: id => InstallationController.setRole(card.unitId, id)
                HoverHandler { id: roleHover }
                ToolTip.visible: roleHover.hovered
                ToolTip.text: qsTr("How the programme reaches it: the audio link (a repeater also sends it on), S/PDIF from a source unit, or USB from the host")
            }
        }
        InstallationField {
            visible: card.fed
            title: qsTr("Fed by")
            issue: card.issueOf("upstream")
            issueName: "Installation.upstreamIssue." + card.unitId
            InstallationChoice {
                objectName: "Installation.upstream." + card.unitId
                label: qsTr("What feeds %1").arg(card.unit.name || "")
                enabled: card.editable
                choices: InstallationController.feedChoices(card.role, card.unitId, card.unit.upstream || "", InstallationController.generation)
                value: card.unit.upstream || ""
                onChosen: id => InstallationController.setUpstream(card.unitId, id)
            }
        }
        InstallationField {
            visible: card.unit.spdifSource === true
            title: qsTr("Time base on outputs")
            Repeater {
                model: 4
                delegate: AppCheckBox {
                    required property int index
                    objectName: "Installation.slot." + card.unitId + "." + index
                    text: qsTr("%1-%2").arg(2 * index + 1).arg(2 * index + 2)
                    enabled: card.editable
                    checked: ((card.unit.txMask || 0) >> index & 1) === 1
                    Accessible.name: qsTr("%1 sends the time base on outputs %2-%3").arg(card.unit.name || "").arg(2 * index + 1).arg(2 * index + 2)
                    onToggled: {
                        InstallationController.setSpdifSlot(card.unitId, index, checked)
                        checked = Qt.binding(() => ((card.unit.txMask || 0) >> index & 1) === 1)
                    }
                }
            }
        }
    }
    // The name's issue, and one of none of the fields above
    Text {
        objectName: "Installation.issue." + card.unitId
        Layout.fillWidth: true
        visible: text.length > 0
        text: card.issueOf("name") || card.issueOf("") || (card.fed ? "" : card.issueOf("upstream"))
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.dangerText
        wrapMode: Text.Wrap
        Accessible.role: Accessible.AlertMessage
        Accessible.name: text
    }
}
