import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP INSTQ -- Setting up the installation on the hub this console is on
// (MPLAN 1.4), from InstallationController: the units the hub sees that are
// not in it, with Join; each unit's room, role and feed; senders and fan-outs;
// rooms, zones and groups. Below, always in sight: what saving would change,
// and Save. Saving is the hub's Admin's; everyone else reads. Window > System's
// Installation tab, and Modern's System > Installation.
Panel {
    id: panel
    objectName: "InstallationPanel"
    readonly property Flickable body: flick
    readonly property bool editable: InstallationController.editable
    fitBodies: [flick]

    Binding {
        target: InstallationController
        property: "shown"
        value: panel.shown
    }
    Component.onDestruction: if (shown) InstallationController.shown = false

    function listOf(ids) {
        return ids.map(id => InstallationController.element(id, InstallationController.generation).name || id).join(", ")
    }

    // A name, and Add
    component AddRow: RowLayout {
        id: row
        property string kind
        property string placeholder
        property string issue: InstallationController.issueOf("new:" + kind, "", InstallationController.generation)
        property var add: name => false
        default property alias more: extra.data
        visible: panel.editable
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        function commit() { if (add(field.text)) field.text = "" }
        BorderedField {
            id: field
            objectName: "Installation.new." + row.kind
            implicitWidth: 200
            maximumLength: 64
            label: row.placeholder
            placeholderText: row.placeholder
            Keys.onReturnPressed: row.commit()
            Keys.onEnterPressed: row.commit()
        }
        RowLayout {
            id: extra
            Layout.fillWidth: false
            spacing: Theme.spacingSmall
        }
        AppButton {
            objectName: "Installation.add." + row.kind
            text: qsTr("Add")
            Accessible.name: row.placeholder + ": " + text
            onClicked: row.commit()
        }
        Text {
            objectName: "Installation.newIssue." + row.kind
            Layout.fillWidth: true
            text: row.issue
            textFormat: Text.PlainText
            font: Theme.fontSmall
            color: Theme.dangerText
            elide: Text.ElideRight
            Accessible.role: Accessible.AlertMessage
            Accessible.name: text
        }
    }
    component Note: Text {
        Layout.fillWidth: true
        textFormat: Text.PlainText
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }

    Text {
        objectName: "Installation.none"
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.spacingLarge
        visible: !InstallationController.available
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        textFormat: Text.PlainText
        text: InstallationController.error.length > 0 ? InstallationController.error : qsTr("Reading the installation…")
        font: Theme.fontBody
        color: Theme.textMuted
    }

    Flickable {
        id: flick
        objectName: "Installation.body"
        anchors { top: parent.top; left: parent.left; right: parent.right; bottom: footer.top }
        visible: InstallationController.available
        contentWidth: width
        contentHeight: column.implicitHeight + 2 * Theme.spacing
        clip: true
        ScrollBar.vertical: AppScrollBar {}

        ColumnLayout {
            id: column
            x: Theme.spacing
            y: Theme.spacing
            width: flick.width - 2 * Theme.spacing
            spacing: Theme.spacing

            Note {
                objectName: "Installation.note"
                visible: InstallationController.editNote.length > 0
                text: qsTr("%1 This part is read only.").arg(InstallationController.editNote)
            }
            Note {
                visible: InstallationController.error.length > 0
                text: InstallationController.error
                color: Theme.warning
            }

            // ---- Not in the installation -----------------------------------
            SectionHeading {
                Layout.fillWidth: true
                text: qsTr("Not in the installation")
            }
            Note {
                objectName: "Installation.unseenNote"
                visible: !InstallationController.inventory || InstallationController.unseenSerials.length === 0
                text: InstallationController.inventory ? qsTr("Every DSPi this hub has seen is in the installation.")
                                                       : qsTr("This hub keeps no inventory, so it cannot list the units it has seen. Give a unit's serial to join it.")
            }
            Repeater {
                model: InstallationController.unseenSerials
                delegate: InstallationJoinCard {
                    required property string modelData
                    Layout.fillWidth: true
                    serial: modelData
                    editable: panel.editable
                }
            }
            InstallationJoinCard {
                Layout.fillWidth: true
                visible: !InstallationController.inventory && panel.editable
                serial: ""
                editable: panel.editable
            }

            // ---- Units -------------------------------------------------------
            SectionHeading {
                Layout.fillWidth: true
                text: qsTr("Units")
            }
            Note {
                visible: InstallationController.unitIds.length === 0
                text: qsTr("No unit is in the installation yet.")
            }
            Repeater {
                model: InstallationController.unitIds
                delegate: InstallationUnitCard {
                    required property string modelData
                    Layout.fillWidth: true
                    unitId: modelData
                    editable: panel.editable
                }
            }

            // ---- Senders and fan-outs ----------------------------------------
            SectionHeading {
                Layout.fillWidth: true
                text: qsTr("Senders and fan-outs")
            }
            Repeater {
                model: InstallationController.feederIds
                delegate: InstallationMembersCard {
                    required property string modelData
                    Layout.fillWidth: true
                    elementId: modelData
                    editable: panel.editable
                    note: [element.kind === "sender" ? qsTr("Sender") : qsTr("Fan-out, fed by %1").arg(panel.listOf([element.upstream || ""])),
                           element.serial || ""].filter(t => t.length > 0).join(" · ")
                }
            }
            AddRow {
                kind: "sender"
                placeholder: qsTr("New sender")
                add: name => InstallationController.addSender(name)
            }

            // ---- Rooms, zones and groups ---------------------------------------
            SectionHeading {
                Layout.fillWidth: true
                text: qsTr("Rooms")
            }
            Repeater {
                model: InstallationController.roomIds
                delegate: InstallationMembersCard {
                    required property string modelData
                    Layout.fillWidth: true
                    elementId: modelData
                    editable: panel.editable
                }
            }
            AddRow {
                kind: "room"
                placeholder: qsTr("New room")
                add: name => InstallationController.addRoom(name)
            }
            SectionHeading {
                Layout.fillWidth: true
                text: qsTr("Zones: rooms that share a source")
            }
            Repeater {
                model: InstallationController.zoneIds
                delegate: InstallationMembersCard {
                    required property string modelData
                    Layout.fillWidth: true
                    elementId: modelData
                    editable: panel.editable
                    memberIds: InstallationController.roomIds
                    note: element.source ? qsTr("Plays %1").arg(panel.listOf([element.source])) : ""
                }
            }
            AddRow {
                kind: "zone"
                placeholder: qsTr("New zone")
                add: name => InstallationController.addZone(name, zoneSource.value)
                InstallationChoice {
                    id: zoneSource
                    objectName: "Installation.newZoneSource"
                    visible: choices.length > 1
                    label: qsTr("What the new zone plays")
                    choices: InstallationController.sourceChoices(InstallationController.generation)
                    onChosen: id => value = id
                    onChoicesChanged: if (choices.findIndex(c => c.id === value) < 0) value = ""
                }
            }
            SectionHeading {
                Layout.fillWidth: true
                text: qsTr("Groups of units, for scenes")
            }
            Repeater {
                model: InstallationController.groupIds
                delegate: InstallationMembersCard {
                    required property string modelData
                    Layout.fillWidth: true
                    elementId: modelData
                    editable: panel.editable
                    memberIds: InstallationController.unitIds
                }
            }
            AddRow {
                kind: "group"
                placeholder: qsTr("New group")
                add: name => InstallationController.addGroup(name)
            }
        }
    }

    // ---- What saving would change, and Save ---------------------------------
    Rectangle {
        id: footer
        anchors { left: parent.left; right: parent.right; bottom: parent.bottom }
        visible: InstallationController.available
                 && (InstallationController.mayEdit || InstallationController.dirty || InstallationController.message.length > 0)
        height: visible ? bar.implicitHeight + 2 * Theme.spacing : 0
        color: Theme.panel
        Rectangle {
            width: parent.width
            height: 1
            color: Theme.separator
        }
        ColumnLayout {
            id: bar
            x: Theme.spacing
            y: Theme.spacing
            width: parent.width - 2 * Theme.spacing
            spacing: Theme.spacingSmall

            Text {
                objectName: "Installation.stale"
                Layout.fillWidth: true
                visible: InstallationController.stale
                text: qsTr("The installation was changed elsewhere while you edited it. Reload it and make your changes again.")
                font: Theme.fontBody
                color: Theme.warning
                wrapMode: Text.Wrap
                Accessible.role: Accessible.AlertMessage
                Accessible.name: text
            }
            Flickable {
                Layout.fillWidth: true
                implicitHeight: Math.min(changeList.implicitHeight, 120)
                visible: InstallationController.changes.length > 0
                contentHeight: changeList.implicitHeight
                clip: true
                ScrollBar.vertical: AppScrollBar {}
                Column {
                    id: changeList
                    objectName: "Installation.changes"
                    property string shownName: InstallationController.changes.join("\n")
                    width: parent.width
                    Repeater {
                        model: InstallationController.changes
                        delegate: Text {
                            required property string modelData
                            width: changeList.width
                            text: "• " + modelData
                            textFormat: Text.PlainText
                            font: Theme.fontSmall
                            color: Theme.text
                            wrapMode: Text.Wrap
                        }
                    }
                }
            }
            Text {
                objectName: "Installation.issue"
                Layout.fillWidth: true
                visible: InstallationController.dirty && InstallationController.issue.length > 0
                text: qsTr("Not ready to save: %1").arg(InstallationController.issue)
                textFormat: Text.PlainText
                font: Theme.fontSmall
                color: Theme.dangerText
                wrapMode: Text.Wrap
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacingSmall
                AppButton {
                    objectName: "Installation.save"
                    text: qsTr("Save")
                    primary: true
                    visible: !InstallationController.stale
                    enabled: panel.editable && InstallationController.dirty && InstallationController.issue.length === 0
                    toolTip: InstallationController.mayEdit ? qsTr("Save the installation on the hub") : InstallationController.editNote
                    onClicked: InstallationController.save()
                }
                AppButton {
                    objectName: "Installation.reload"
                    text: InstallationController.stale ? qsTr("Reload") : qsTr("Discard")
                    primary: InstallationController.stale
                    enabled: InstallationController.dirty && !InstallationController.busy
                    onClicked: InstallationController.reload()
                }
                Note {
                    objectName: "Installation.message"
                    text: InstallationController.message
                    color: InstallationController.messageFailed ? Theme.dangerText : Theme.textMuted
                    Accessible.role: Accessible.StaticText
                    Accessible.name: text
                }
            }
        }
    }
}
