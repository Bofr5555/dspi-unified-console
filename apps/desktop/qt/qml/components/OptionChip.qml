import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP DL0 -- One option that can be picked (DESIGN-LANGUAGE §13): a chip in
// the control tone, 24 high, radius 12, its label in fontSmall; chosen
// (`checked`) it takes the selection tone and accent text, never an accent
// fill. Keyboard focus is the 2 px focus ring; Space clicks it.
//
// The owner keeps the state: bind `checked` to the device's value and write
// on `clicked` (the chip does not flip itself unless `checkable` is set).
// A row of them is a ChipGroup; an overlay or mode toggle is a ToggleChip.
//
//     OptionChip {
//         objectName: "Crossfeed.output.3"
//         text: "4"
//         toolTip: qsTr("Output 4")      // also its accessible name when set
//         checked: panel.outputOn(3)
//         onClicked: panel.setOutput(3, !checked)
//     }
//
// Properties: text, checked, checkable (false), iconName (a Lucide name
// before the label, optional), toolTip, exclusive (false: a radio button to
// assistive technology when true), chipRadius (Theme.radiusChip).
AbstractButton {
    id: chip
    property string iconName
    property string toolTip
    property bool exclusive: false
    property int chipRadius: Theme.radiusChip

    implicitHeight: Theme.chipHeight   // -- WP UB13 --
    implicitWidth: Math.max(implicitHeight, contentItem.implicitWidth + leftPadding + rightPadding)
    leftPadding: 10
    rightPadding: 10
    topPadding: 0
    bottomPadding: 0
    font: Theme.fontSmall
    hoverEnabled: true
    focusPolicy: Qt.TabFocus

    readonly property color ink: !enabled ? Theme.textDisabled : checked ? Theme.accentText : Theme.text

    contentItem: Item {
        implicitWidth: label.implicitWidth
        implicitHeight: label.implicitHeight
        Row {
            id: label
            anchors.centerIn: parent
            spacing: 4
            Icon {
                visible: chip.iconName.length > 0
                anchors.verticalCenter: parent.verticalCenter
                name: chip.iconName
                size: 14
                color: chip.ink
            }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                textFormat: Text.PlainText
                text: chip.text
                font: chip.font
                color: chip.ink
            }
        }
    }
    background: Rectangle {
        radius: Math.min(chip.chipRadius, height / 2)
        color: chip.checked ? Theme.selection
             : chip.down ? Theme.controlPressed
             : chip.hovered && chip.enabled ? Theme.controlHover : Theme.control
        opacity: chip.enabled ? 1 : 0.6
        border.width: chip.visualFocus ? Theme.focusRingWidth : Theme.chipLineWidth   // -- WP UB13 --
        border.color: chip.visualFocus ? Theme.focusRing : Theme.controlOutline
    }

    HelpTip {
        text: chip.toolTip
        visible: chip.hovered && chip.toolTip.length > 0
    }

    Accessible.role: exclusive ? Accessible.RadioButton : Accessible.CheckBox
    Accessible.name: toolTip.length ? toolTip : text
    Accessible.checkable: true
    Accessible.checked: checked
}
