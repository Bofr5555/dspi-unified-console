import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP DL0 -- One row of a form (DESIGN-LANGUAGE §10): the Settings rows
// (SettingsToggle, SettingsPickerRow, SettingsValueRow) generalised for every
// page. At least Theme.formRowHeight (36) high; the title in fontBody at the
// left with an optional caption under it in fontSmall muted; the control (any
// item: a field with its unit, a combo box, a switch, a ChipGroup, buttons)
// at the right edge. Put rows in a ContentColumn so the form stops at 760 px.
//
//     FormRow {
//         objectName: "Leveller.attack"      // the title text is "<name>.title"
//         title: qsTr("Attack")
//         caption: qsTr("How fast the level comes down")   // optional
//         NumericField { unit: "ms"; implicitWidth: 88; value: panel.attack
//                        onCommitted: v => panel.setAttack(v) }
//         Text { text: "ms"; font: Theme.fontSmall; color: Theme.textMuted }   // more items sit side by side
//     }
//
// Properties: title, caption, controlFills (false: the control keeps its own
// width at the right; true: it takes the room after the title, for a slider
// or a long field), titleWidth (0: as wide as the title; set it to line up
// the controls of a column of rows).
RowLayout {
    id: row
    property string title
    property string caption
    property bool controlFills: false
    property int titleWidth: 0
    default property alias control: slot.data

    Layout.fillWidth: true
    Layout.minimumHeight: Theme.formRowHeight
    implicitHeight: Math.max(Theme.formRowHeight, label.implicitHeight + 8, slot.implicitHeight)
    spacing: Theme.spacing

    ColumnLayout {
        id: label
        spacing: 2
        Layout.fillWidth: !row.controlFills
        Layout.preferredWidth: row.titleWidth > 0 ? row.titleWidth : -1
        Layout.alignment: Qt.AlignVCenter
        Text {
            objectName: row.objectName.length ? row.objectName + ".title" : ""
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: row.title
            font: Theme.fontBody
            color: row.enabled ? Theme.text : Theme.textDisabled
            wrapMode: Text.Wrap
        }
        Text {
            Layout.fillWidth: true
            visible: text.length > 0
            textFormat: Text.PlainText
            text: row.caption
            font: Theme.fontSmall
            color: row.enabled ? Theme.textMuted : Theme.textDisabled
            wrapMode: Text.Wrap
        }
    }
    RowLayout {
        id: slot
        Layout.fillWidth: row.controlFills
        Layout.alignment: Qt.AlignVCenter | Qt.AlignRight
        spacing: Theme.spacingSmall
    }

    Accessible.role: Accessible.Grouping
    Accessible.name: title
    Accessible.description: caption
}
