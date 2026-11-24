import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP ROLEQ -- Which output drives what: one row an output of the device,
// over SpeakersController's draft (the stored speaker layout, else the
// proposal). The Getting Started step and Settings › Speakers show it.
ColumnLayout {
    id: table
    readonly property var sc: SpeakersController
    property bool editable: sc.editable
    readonly property var roles: [
        { id: "full", text: qsTr("Full range") }, { id: "sub", text: qsTr("Subwoofer") },
        { id: "low", text: qsTr("Low (woofer)") }, { id: "mid", text: qsTr("Mid") },
        { id: "high", text: qsTr("High (tweeter)") }
    ]
    function indexOf(id) {
        for (let i = 0; i < roles.length; ++i) if (roles[i].id === id) return i
        return 0
    }
    spacing: Theme.spacingSmall

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
                text: row.modelData.label
                Layout.preferredWidth: 170   // -- WP ADETQ -- (190) room for the pill
                enabled: table.editable
                checked: row.modelData.included
                onToggled: table.sc.setIncluded(row.modelData.output, checked)
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
                onActivated: index => table.sc.setRole(row.modelData.output, table.roles[index].id)
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
                onCommitted: t => table.sc.setSpeaker(row.modelData.output, t)
            }
            AppSpinBox {
                objectName: "Speakers.group." + row.modelData.output
                label: qsTr("Group of %1").arg(row.modelData.label)
                Layout.preferredWidth: 90
                enabled: table.editable && row.modelData.included
                realFrom: 0
                realTo: 15
                realValue: row.modelData.group
                onCommitted: value => table.sc.setGroup(row.modelData.output, Math.round(value))
            }
        }
    }
    Text {
        Layout.fillWidth: true
        text: qsTr("The outputs of one speaker share a group: they are measured in one run and aligned to each other.")
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.WordWrap
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
