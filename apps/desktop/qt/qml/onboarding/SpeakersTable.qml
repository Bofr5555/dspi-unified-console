import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP ROLEQ -- Which output drives what: one row an output of the device,
// over SpeakersController's draft (the stored speaker layout, else the
// proposal). The Getting Started step and Settings › Speakers show it.
// -- WP UA4 -- (UXP A4, Q11) Column headings; groups counted from 1 (the
// layout keeps 0..15); a speaker named like another row's joins its group.
// -- WP UB7 -- (UXP B7) The one speaker table: Settings › Speakers and the
// Getting Started step show it; Measure and Auto-tune show its summary
// (SpeakersSummary). Headings Output · Role · Speaker · Measured together; the
// group follows the speaker's name (SpeakersController) and shows as the names
// measured together; "Choose which outputs are measured together" is the
// override (on by itself while the groups are not the names'), and off again
// puts the groups back to the names.
ColumnLayout {
    id: table
    readonly property var sc: SpeakersController
    property bool editable: sc.editable
    // -- WP UB7 -- the person chose to pick the groups; the table shows the
    // pickers also while the groups are not the names'
    property bool groupsAsked: false
    readonly property bool byHand: groupsAsked || !sc.groupsFollowNames
    // The groups to pick from: each group of the layout by its names, in output
    // order, then a group of its own
    readonly property var groupChoices: {
        const out = [], seen = []
        for (const r of sc.rows) {
            if (!r.included || seen.indexOf(r.group) >= 0) continue
            seen.push(r.group)
            out.push({ group: r.group, text: r.groupName })
        }
        let free = 0
        while (seen.indexOf(free) >= 0 && free < 15) ++free
        //: The speaker table's choice of an output measured alone, in a group of its own
        out.push({ group: free, text: qsTr("On its own"), alone: true })
        return out
    }
    function groupIndex(group) {
        for (let i = 0; i < groupChoices.length; ++i) if (groupChoices[i].group === group && !groupChoices[i].alone) return i
        return groupChoices.length - 1
    }
    // -- WP UA4 -- the person changed a row (not a reload)
    signal changedByUser()
    // -- WP UB7 -- the glossary's words (A10): woofer, mid driver, tweeter
    readonly property var roles: [
        { id: "full", text: qsTr("Full range") }, { id: "sub", text: qsTr("Subwoofer") },
        { id: "low", text: qsTr("Woofer") }, { id: "mid", text: qsTr("Mid driver") },
        { id: "high", text: qsTr("Tweeter") }
    ]
    function indexOf(id) {
        for (let i = 0; i < roles.length; ++i) if (roles[i].id === id) return i
        return 0
    }
    spacing: Theme.spacingSmall

    // -- WP UA4 -- the headings, over the columns of the rows below
    RowLayout {
        objectName: "Speakers.headings"
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        component Heading: Text {
            textFormat: Text.PlainText
            font: Theme.fontHeading
            color: Theme.textMuted
            elide: Text.ElideRight
            Accessible.role: Accessible.ColumnHeader
            Accessible.name: text
        }
        Heading { Layout.preferredWidth: 170; text: qsTr("OUTPUT") }
        Heading { Layout.preferredWidth: 140; text: qsTr("ROLE") }
        Item { Layout.preferredWidth: 104 }
        Heading { Layout.preferredWidth: 72; text: qsTr("SPEAKER") }
        //: The speaker table's column of groups: the outputs measured in one run and aligned to each other
        // -- WP UB7 -- (GROUP) the rest of the row; two lines where it is narrow
        Heading {
            Layout.fillWidth: true
            Layout.minimumWidth: 72
            text: qsTr("MEASURED TOGETHER")
            wrapMode: Text.WordWrap
            elide: Text.ElideNone
        }
    }
    Rectangle {
        Layout.fillWidth: true
        Layout.topMargin: -4
        implicitHeight: 1
        color: Theme.separator
    }

    Repeater {
        model: table.sc.rows
        delegate: RowLayout {
            id: row
            required property var modelData
            objectName: "Speakers.row." + modelData.output
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            AppCheckBox {
                objectName: "Speakers.included." + row.modelData.output
                // -- WP FIN015 -- (REVF D1) the output, and the name the other pages give it:
                // "Out 1 (Centre woofer)"
                readonly property string name: DeviceModel.revision < 0 ? "" : DeviceModel.channels.outputName(row.modelData.output)
                //: An output in the speaker layout: %1 its label ("Out 1"), %2 the name the pages give it ("Centre woofer")
                text: name.length && name !== row.modelData.label ? qsTr("%1 (%2)").arg(row.modelData.label).arg(name) : row.modelData.label
                Layout.preferredWidth: 170   // -- WP ADETQ -- (190) room for the pill
                enabled: table.editable
                checked: row.modelData.included
                onToggled: {
                    table.changedByUser()   // -- WP UA4 -- first: the edit rebuilds this row
                    table.sc.setIncluded(row.modelData.output, checked)
                }
                Accessible.name: qsTr("%1 is in the layout").arg(row.modelData.label)
            }
            AppComboBox {
                objectName: "Speakers.role." + row.modelData.output
                label: qsTr("Role of %1").arg(row.modelData.label)
                Layout.preferredWidth: 140   // -- WP ADETQ -- (150) room for the pill
                enabled: table.editable && row.modelData.included
                model: table.roles
                textRole: "text"
                currentIndex: table.indexOf(row.modelData.role)
                onActivated: index => {
                    table.changedByUser()   // -- WP UA4 -- first: the edit rebuilds this row
                    table.sc.setRole(row.modelData.output, table.roles[index].id)
                }
            }
            // -- WP ADETQ -- (AUTODES 5.1) whose word the role is, how sure a detection was
            // A slot of its own on every row, so the columns after it line up
            Item {
                Layout.preferredWidth: 104
                Layout.preferredHeight: 18
                Rectangle {
                    id: pill
                    objectName: "Speakers.pill." + row.modelData.output
                    readonly property string kind: row.modelData.pillKind || ""
                    readonly property bool warn: ["uncertain", "unknown", "nothing"].indexOf(kind) >= 0
                    visible: kind.length > 0
                    anchors.verticalCenter: parent.verticalCenter
                    width: Math.min(parent.width, pillText.implicitWidth + 12)
                    height: 18
                    radius: height / 2
                    color: "transparent"
                    border.color: warn ? Theme.warning : kind === "suggested" || kind === "off" ? Theme.border : Theme.accent
                    Text {
                        id: pillText
                        anchors.centerIn: parent
                        width: Math.min(implicitWidth, pill.parent.width - 12)
                        elide: Text.ElideRight
                        textFormat: Text.PlainText
                        text: row.modelData.pill || ""
                        font: Theme.fontCaption
                        color: pill.warn ? Theme.warning : Theme.text
                    }
                    HoverHandler { id: pillHover }
                    HelpTip {
                        visible: pillHover.hovered && text.length > 0
                        text: row.modelData.pillReason || ""
                    }
                    Accessible.role: Accessible.StaticText
                    Accessible.name: pillText.text
                    Accessible.description: row.modelData.pillReason || ""
                }
            }
            BorderedField {
                objectName: "Speakers.speaker." + row.modelData.output
                label: qsTr("Speaker of %1").arg(row.modelData.label)
                Layout.preferredWidth: 72
                enabled: table.editable && row.modelData.included
                //: A speaker field with no speaker named (Measure and Settings › Speakers)
                placeholderText: qsTr("Not set")   // -- INTC (DL7 fold) -- Measure's placeholder (was "Speaker")
                text: row.modelData.speaker
                onCommitted: t => {
                    // -- WP UA4 -- read before the first edit, which rebuilds this row
                    // -- WP UB7 -- the controller puts it in its name's group
                    const owner = table, output = row.modelData.output
                    owner.changedByUser()
                    owner.sc.setSpeaker(output, t)
                }
            }
            // -- WP UB7 -- (UXP B7) measured together: the names, or the picker
            Text {
                objectName: "Speakers.together." + row.modelData.output
                visible: !table.byHand
                Layout.fillWidth: true
                Layout.minimumWidth: 72
                textFormat: Text.PlainText
                text: row.modelData.included ? row.modelData.groupName : ""
                font: Theme.fontBody
                color: Theme.text
                elide: Text.ElideRight
                Accessible.role: Accessible.StaticText
                //: %1 an output ("Out 1"), %2 the speakers measured with it ("L", "L + Sub")
                Accessible.name: qsTr("%1 is measured with %2").arg(row.modelData.label).arg(text)
            }
            AppComboBox {
                objectName: "Speakers.group." + row.modelData.output
                visible: table.byHand
                //: %1 an output ("Out 1")
                label: qsTr("%1 is measured with").arg(row.modelData.label)
                Layout.fillWidth: true
                Layout.minimumWidth: 72
                Layout.maximumWidth: 200
                enabled: table.editable && row.modelData.included
                model: table.groupChoices
                textRole: "text"
                currentIndex: table.groupIndex(row.modelData.group)
                onActivated: index => {
                    const owner = table, output = row.modelData.output, group = owner.groupChoices[index].group
                    owner.changedByUser()   // first: the edit rebuilds this row
                    owner.sc.setGroup(output, group)
                }
            }
        }
    }
    Text {
        Layout.fillWidth: true
        // -- WP UB7 --
        text: qsTr("Outputs with the same speaker name are measured together in one run and aligned to each other.")
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.WordWrap
    }
    // -- WP UB7 -- (UXP B7) the advanced override; off puts the groups back to the names
    AppCheckBox {
        objectName: "Speakers.byHand"
        text: qsTr("Choose which outputs are measured together")
        enabled: table.editable
        checked: table.byHand
        onToggled: {
            table.groupsAsked = checked
            checked = Qt.binding(() => table.byHand)   // the click broke it
            if (table.groupsAsked || table.sc.groupsFollowNames) return
            table.changedByUser()
            table.sc.followNames()
        }
    }
    // The proposal's remarks, while nothing is stored
    Repeater {
        model: table.sc.status === "none" ? table.sc.notes : []
        delegate: Text {
            required property string modelData
            objectName: "Speakers.note"
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: modelData
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.WordWrap
        }
    }
}
