import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP DL0 -- A set of options as chips (DESIGN-LANGUAGE §13), in place of a
// row of accent-filled buttons (the Effects pages' outputs "1 … 9", the
// Detector / Apply rows, the Signal Generator's outputs, Crossfeed's pairs):
// an optional section label with "All" and "None" at its right, then the
// chips, wrapping onto more lines when the row is too narrow.
//
// Many (default): `selected` lists the chosen indexes; a click emits
// `toggled(index, on)` with the new state, All / None emit `setAll(on)`.
// One (`exclusive: true`): `current` is the chosen index; a click emits
// `chosen(index)`. Either way the owner writes the device and binds
// `selected` / `current` back; the group never changes them itself.
//
//     ChipGroup {
//         objectName: "Crossfeed.outputs"    // chips "Crossfeed.outputs.chip.<i>",
//                                            // "Crossfeed.outputs.all", ".none"
//         Layout.fillWidth: true
//         label: qsTr("Outputs")
//         model: ["1", "2", "3"]             // or [{ text, toolTip, enabled }]
//         selected: panel.outputIndexes      // e.g. [0, 2]
//         onToggled: (index, on) => panel.setOutput(index, on)
//         onSetAll: on => panel.setAllOutputs(on)
//     }
//
// Properties: label, model, selected, current, exclusive, allNone (shows All /
// None; default: many and more than two chips), spacing between chips (6).
ColumnLayout {
    id: group
    property string label
    property var model: []
    property var selected: []
    property int current: -1
    property bool exclusive: false
    property bool allNone: !exclusive && count > 2
    property int chipSpacing: 6
    readonly property int count: Array.isArray(model) ? model.length : (model ? model : 0)
    signal toggled(int index, bool on)
    signal chosen(int index)
    signal setAll(bool on)

    function isChecked(i) {
        return exclusive ? i === current : (selected && selected.indexOf !== undefined && selected.indexOf(i) >= 0)
    }
    function entry(i) {
        const m = Array.isArray(model) ? model[i] : undefined
        return m !== null && typeof m === "object" ? m : { text: m !== undefined ? String(m) : String(i + 1) }
    }

    spacing: Theme.spacingSmall

    // A text button with no fill until hovered ("All", "None")
    component QuietButton: AbstractButton {
        id: quiet
        implicitHeight: 20
        implicitWidth: quietText.implicitWidth + 12
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        contentItem: Text {
            id: quietText
            textFormat: Text.PlainText
            text: quiet.text
            font: Theme.fontSmall
            color: quiet.enabled ? Theme.accentText : Theme.textDisabled
            horizontalAlignment: Text.AlignHCenter
            verticalAlignment: Text.AlignVCenter
        }
        background: Rectangle {
            radius: Theme.radiusSmall
            color: quiet.hovered && quiet.enabled ? Theme.controlHover : "transparent"
            border.width: quiet.visualFocus ? Theme.focusRingWidth : 0
            border.color: Theme.focusRing
        }
        Accessible.role: Accessible.Button
        Accessible.name: text
    }

    RowLayout {
        visible: group.label.length > 0 || group.allNone
        Layout.fillWidth: true
        spacing: 4
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: group.label
            font: Theme.fontHeading
            color: Theme.textMuted
            elide: Text.ElideRight
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        QuietButton {
            objectName: group.objectName.length ? group.objectName + ".all" : ""
            visible: group.allNone
            //: Choose every option of a chip group
            text: qsTr("All")
            onClicked: group.setAll(true)
        }
        QuietButton {
            objectName: group.objectName.length ? group.objectName + ".none" : ""
            visible: group.allNone
            //: Choose none of a chip group's options
            text: qsTr("None")
            onClicked: group.setAll(false)
        }
    }

    Flow {
        id: flow
        // The chips side by side, as wide as they are, wrapping when the
        // group is narrower
        property real natural: 0
        function measure() {
            let w = 0
            for (let i = 0; i < chips.count; ++i) {
                const c = chips.itemAt(i)
                if (c) w += c.implicitWidth + (i > 0 ? group.chipSpacing : 0)
            }
            natural = w
        }
        Layout.fillWidth: true
        Layout.preferredWidth: natural
        spacing: group.chipSpacing
        Repeater {
            id: chips
            model: group.count
            onItemAdded: flow.measure()
            onItemRemoved: flow.measure()
            OptionChip {
                required property int index
                readonly property var entryData: group.entry(index)
                objectName: group.objectName.length ? group.objectName + ".chip." + index : ""
                text: entryData.text !== undefined ? entryData.text : ""
                toolTip: entryData.toolTip !== undefined ? entryData.toolTip : ""
                iconName: entryData.iconName !== undefined ? entryData.iconName : ""
                enabled: entryData.enabled !== false
                exclusive: group.exclusive
                checked: group.isChecked(index)
                onImplicitWidthChanged: flow.measure()
                onClicked: group.exclusive ? group.chosen(index) : group.toggled(index, !checked)
            }
        }
    }

    Accessible.role: Accessible.Grouping
    Accessible.name: label
}
