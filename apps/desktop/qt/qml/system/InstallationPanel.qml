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

    // -- WP LAY5b -- counted: a page and a window may each show one
    onShownChanged: InstallationController.panelShown(shown)
    Component.onDestruction: if (shown) InstallationController.panelShown(false)

    function listOf(ids) {
        return ids.map(id => InstallationController.element(id, InstallationController.generation).name || id).join(", ")
    }

    // -- WP FEATUI -- an alignment was asked from here: its outcome is said here
    property bool alignedHere: false

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
    // -- WP FEATUI -- AddRow for a new element with several fields: wraps at a narrow width
    component AddFlow: Flow {
        id: flowRow
        property string kind
        property string placeholder
        property string issue: InstallationController.issueOf("new:" + kind, "", InstallationController.generation)
        property var add: name => false
        default property alias more: extraFlow.data
        visible: panel.editable
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        function commit() { if (add(flowField.text)) flowField.text = "" }
        BorderedField {
            id: flowField
            objectName: "Installation.new." + flowRow.kind
            implicitWidth: 200
            maximumLength: 64
            label: flowRow.placeholder
            placeholderText: flowRow.placeholder
            Keys.onReturnPressed: flowRow.commit()
            Keys.onEnterPressed: flowRow.commit()
        }
        Flow {
            id: extraFlow
            width: Math.min(implicitWidth, flowRow.width)
            spacing: Theme.spacingSmall
        }
        AppButton {
            objectName: "Installation.add." + flowRow.kind
            text: qsTr("Add")
            Accessible.name: flowRow.placeholder + ": " + text
            onClicked: flowRow.commit()
        }
        Text {
            objectName: "Installation.newIssue." + flowRow.kind
            width: Math.min(implicitWidth, flowRow.width)
            text: flowRow.issue
            textFormat: Text.PlainText
            font: Theme.fontSmall
            color: Theme.dangerText
            wrapMode: Text.Wrap
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

            // ---- -- WP FEATUI -- Audio link sources (FEAT1) ---------------------
            SectionHeading {
                Layout.fillWidth: true
                text: qsTr("Sources: channels on the audio link")
            }
            Note {
                visible: InstallationController.sourceIds.length === 0
                text: qsTr("A source names channels of a sender (0 to 63); a zone that plays it routes its units' inputs to them.")
            }
            Repeater {
                model: InstallationController.sourceIds
                delegate: InstallationSourceCard {
                    required property string modelData
                    Layout.fillWidth: true
                    sourceId: modelData
                    editable: panel.editable
                }
            }
            AddFlow {
                kind: "source"
                placeholder: qsTr("New source")
                add: name => {
                    const added = InstallationController.addSource(name, newSourceSender.value, newSourceChannels.text)
                    if (added) newSourceChannels.text = ""
                    return added
                }
                InstallationChoice {
                    id: newSourceSender
                    objectName: "Installation.newSourceSender"
                    visible: choices.length > 1
                    label: qsTr("The new source's sender")
                    choices: InstallationController.senderChoices(InstallationController.generation)
                    onChosen: id => value = id
                }
                BorderedField {
                    id: newSourceChannels
                    objectName: "Installation.newSourceChannels"
                    implicitWidth: 110
                    label: qsTr("The new source's channels")
                    placeholderText: qsTr("Channels: 8 9")
                }
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
                    // -- WP FEATUI -- its source, and the units it routes
                    InstallationField {
                        id: zoneField
                        readonly property var routed: InstallationController.zoneUnits(modelData, InstallationController.generation)
                        title: qsTr("Plays")
                        issue: InstallationController.issueOf(modelData, "source", InstallationController.generation)
                        issueName: "Installation.zoneSourceIssue." + modelData
                        InstallationChoice {
                            objectName: "Installation.zoneSource." + modelData
                            label: qsTr("What %1 plays").arg(element.name || "")
                            enabled: panel.editable
                            choices: InstallationController.sourceChoices(InstallationController.generation)
                            value: element.source || ""
                            onChosen: id => InstallationController.setZoneSource(modelData, id)
                        }
                        Text {
                            objectName: "Installation.zoneUnits." + modelData
                            Layout.maximumWidth: 300
                            text: zoneField.routed.length === 0 ? qsTr("No audio link unit is in its rooms.")
                                  //: %1 a list of units
                                  : element.source ? qsTr("Routes %1 to its channels.").arg(zoneField.routed.join(", "))
                                  //: %1 a list of units
                                  : qsTr("A source would route %1; their own slots stay until then.").arg(zoneField.routed.join(", "))
                            textFormat: Text.PlainText
                            font: Theme.fontSmall
                            color: Theme.textMuted
                            wrapMode: Text.Wrap
                        }
                    }
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

            // ---- -- WP FEATUI -- Scenes and programme sources (FEAT1) -----------
            SectionHeading {
                Layout.fillWidth: true
                text: qsTr("Scenes")
            }
            Repeater {
                model: InstallationController.sceneIds
                delegate: InstallationSceneCard {
                    required property string modelData
                    Layout.fillWidth: true
                    sceneId: modelData
                    editable: panel.editable
                }
            }
            AddRow {
                kind: "scene"
                placeholder: qsTr("New scene")
                add: name => InstallationController.addScene(name)
            }
            SectionHeading {
                Layout.fillWidth: true
                text: qsTr("Programme sources: where the music comes from")
            }
            Note {
                text: qsTr("A DSPi has no player: it plays what its inputs receive. Say which computer or box feeds which unit or sender, so people and AI agents know.")
            }
            Repeater {
                model: InstallationController.programmeIds
                delegate: InstallationProgrammeCard {
                    required property string modelData
                    Layout.fillWidth: true
                    programmeId: modelData
                    editable: panel.editable
                }
            }
            AddFlow {
                kind: "programme"
                placeholder: qsTr("New programme source")
                add: name => {
                    const added = InstallationController.addProgramme(name, newProgrammeKind.value, newProgrammeFeeds.value, newProgrammeInputs.text)
                    if (added) newProgrammeInputs.text = ""
                    return added
                }
                InstallationChoice {
                    id: newProgrammeKind
                    objectName: "Installation.newProgrammeKind"
                    label: qsTr("How the new programme source comes in")
                    choices: InstallationController.programmeKinds()
                    value: "computer_usb"
                    onChosen: id => value = id
                }
                InstallationChoice {
                    id: newProgrammeFeeds
                    objectName: "Installation.newProgrammeFeeds"
                    label: qsTr("What the new programme source feeds")
                    choices: InstallationController.programmeFeedChoices(newProgrammeKind.value, "", InstallationController.generation)
                    onChosen: id => value = id
                    onChoicesChanged: if (choices.findIndex(c => c.id === value) < 0) value = ""
                }
                BorderedField {
                    id: newProgrammeInputs
                    objectName: "Installation.newProgrammeInputs"
                    implicitWidth: 90
                    label: qsTr("The new programme source's inputs")
                    placeholderText: qsTr("Inputs: 0 1")
                }
            }
        }
    }

    // ---- What saving would change, and Save ---------------------------------
    Rectangle {
        id: footer
        anchors { left: parent.left; right: parent.right; bottom: parent.bottom }
        visible: InstallationController.available
                 && (InstallationController.mayEdit || InstallationController.dirty || InstallationController.message.length > 0
                     || InstallationController.lost.length > 0)
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
            // -- WP LAY5b -- what a Reload of a stale draft left out (the web's)
            ColumnLayout {
                objectName: "Installation.lost"
                Layout.fillWidth: true
                visible: InstallationController.lost.length > 0 && !InstallationController.stale
                spacing: Theme.spacingSmall
                property string shownName: InstallationController.lost.join("\n")
                Text {
                    Layout.fillWidth: true
                    text: qsTr("The installation was changed elsewhere and has been read again: these changes were not saved. Make them again on the installation as it is now, or leave them.")
                    font: Theme.fontBody
                    color: Theme.warning
                    wrapMode: Text.Wrap
                    Accessible.role: Accessible.AlertMessage
                    Accessible.name: text
                }
                Flickable {
                    Layout.fillWidth: true
                    implicitHeight: Math.min(lostText.implicitHeight, 80)
                    contentHeight: lostText.implicitHeight
                    clip: true
                    ScrollBar.vertical: AppScrollBar {}
                    Text {
                        id: lostText
                        width: parent.width
                        text: InstallationController.lost.map(c => "• " + c).join("\n")
                        textFormat: Text.PlainText
                        font: Theme.fontSmall
                        color: Theme.text
                        wrapMode: Text.Wrap
                    }
                }
                RowLayout {
                    spacing: Theme.spacingSmall
                    AppButton {
                        objectName: "Installation.remake"
                        text: qsTr("Make Them Again")
                        primary: true
                        enabled: panel.editable
                        onClicked: InstallationController.remake()
                    }
                    AppButton {
                        objectName: "Installation.dropLost"
                        text: qsTr("Leave Them")
                        onClicked: InstallationController.dropLost()
                    }
                }
            }
            // -- WP FEATUI -- a zone's source replaces hand-made slots: said before Save
            Text {
                objectName: "Installation.rerouted"
                Layout.fillWidth: true
                visible: InstallationController.rerouted.length > 0
                text: InstallationController.rerouted.map(r => r.handMade
                          //: %1 a unit, %2 a zone, %3 an audio link source
                          ? qsTr("%1's own slots will be replaced: %2 plays %3.").arg(r.name).arg(r.zone).arg(r.source)
                          //: %1 a unit, %2 a zone, %3 an audio link source
                          : qsTr("%1 will play %3's channels, as %2 does.").arg(r.name).arg(r.zone).arg(r.source)).join(" ")
                      + " " + qsTr("The units play them after the next alignment.")
                textFormat: Text.PlainText
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
                // -- WP FEATUI -- saved slots go live at the alignment
                AppButton {
                    objectName: "Installation.align"
                    visible: InstallationController.alignPending > 0
                    text: qsTr("Align")
                    toolTip: qsTr("Align the audio link now: the units take their new channels")
                    enabled: !SystemController.busy
                    onClicked: alignQuestion.open()
                }
            }
            Note {
                objectName: "Installation.alignMessage"
                visible: panel.alignedHere && text.length > 0
                text: SystemController.actionMessage
                color: SystemController.actionFailed ? Theme.dangerText : Theme.textMuted
            }
        }
    }

    // -- WP FEATUI -- as the System view's
    QuestionDialog {
        id: alignQuestion
        objectName: "Installation.alignQuestion"
        title: qsTr("Align audio link delays?")
        text: qsTr("Every unit restarts its outputs in step with a short silence (2 to 6 ms).")
        acceptText: qsTr("Align")
        onAnswered: answer => {
            if (answer !== "accept") return
            panel.alignedHere = true
            InstallationController.alignAsked()
            SystemController.align()
        }
    }
}
