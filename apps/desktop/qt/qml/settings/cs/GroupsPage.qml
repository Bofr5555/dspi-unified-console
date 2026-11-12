import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- Settings > Channel Groups (the Mac's groups section): named
// sets of channels one control drives as a unit. A group waits for its Apply;
// one with no members is not stored.
SettingsPage {
    id: page
    objectName: "GroupsPage"
    property var opened: ({})
    function isOpen(key) { return page.opened[key] === true }
    function setOpen(key, on) {
        const o = Object.assign({}, page.opened)
        o[key] = on
        page.opened = o
    }

    CsKeys { id: cards; page: "groups" }
    Connections {
        target: CsController
        function onAdded(key) { page.setOpen(key, true) }
    }

    CsPageState {}

    CsEmptyState {
        objectName: "Cs.groups.empty"
        visible: CsController.loaded && cards.list.length === 0
        iconName: "group"
        title: qsTr("No Channel Groups Configured")
        text: qsTr("Name a set of channels so one control can drive them together - a stereo pair, a zone, every output at once.")
        AppButton {
            objectName: "Cs.groups.addEmpty"
            text: qsTr("Add Group")
            iconName: "plus"
            primary: true
            enabled: CsController.revision >= 0 && CsController.canAdd("group")
            onClicked: CsController.addGroup()
        }
    }

    Repeater {
        model: cards.list
        delegate: CsCard {
            id: card
            required property string modelData
            objectName: "Cs." + modelData.replace("/", ".")
            rec: CsController.revision < 0 ? ({}) : CsController.record(modelData)
            iconName: "group"
            ready: rec.configured === true
            hint: qsTr("Pick at least one channel.")
            expanded: page.isOpen(modelData)
            onExpandedChanged: if (expanded !== page.isOpen(modelData)) page.setOpen(modelData, expanded)
            readonly property var members: (rec.rows || []).filter(r => r.id.startsWith("member."))
            CsRows {
                objectName: card.objectName + ".rows"
                rows: (card.rec.rows || []).filter(r => !r.id.startsWith("member."))
                enabled: CsController.writable && card.rec.applying !== true
                onEdited: (id, value) => CsController.edit(card.modelData, id, value)
            }
            // The members as the Mac's three columns of check boxes
            HwRow {
                visible: card.members.length > 0
                title: qsTr("Members")
                caption: qsTr("Every channel this group drives together.")
            }
            GridLayout {
                visible: card.members.length > 0
                Layout.fillWidth: true
                columns: 3
                columnSpacing: 6
                rowSpacing: 6
                enabled: CsController.writable && card.rec.applying !== true
                Repeater {
                    model: card.members.length
                    delegate: AppCheckBox {
                        required property int index
                        readonly property var row: card.members[index] || ({})
                        objectName: card.objectName + ".rows.row." + row.id + ".check"
                        Layout.fillWidth: true
                        text: row.title || ""
                        checked: row.on === true
                        onToggled: CsController.edit(card.modelData, row.id, checked)
                    }
                }
            }
        }
    }

    RowLayout {
        visible: CsController.loaded && cards.list.length > 0
        Layout.fillWidth: true
        AppButton {
            objectName: "Cs.groups.add"
            text: qsTr("Add Group")
            iconName: "plus"
            enabled: CsController.revision >= 0 && CsController.canAdd("group")
            onClicked: CsController.addGroup()
        }
        Item { implicitHeight: 1; Layout.fillWidth: true }
        Text {
            visible: CsController.revision >= 0 && !CsController.canAdd("group") && CsController.writable
            text: qsTr("All %1 group slots are in use.").arg(CsController.groupCount)
            font: Theme.fontSmall
            color: Theme.textMuted
        }
    }

    SettingsGroup {
        Text {
            Layout.fillWidth: true
            text: qsTr("A group is a named set of channels one control can drive as a unit: mute a zone, trim a stereo pair, light an LED when any member clips. Turn on \"Address a Group\" in a control, or in a macro step, to use one.\n\nRelative moves (an encoder, a button) step every member from its own value, so the balance between them survives. A knob moves the group's average and keeps those offsets unless \"Match Members Exactly\" is on.\n\nGroups are stored on the device alongside the controls and share their Save and Revert.")
            font: Theme.fontCaption
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }
}
