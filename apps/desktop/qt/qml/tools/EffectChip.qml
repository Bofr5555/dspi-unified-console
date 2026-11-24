import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- One output, pair or channel of an effect's mask (the Mac's
// channel chips): filled with the accent while it is in the mask, its name
// in the tooltip.
// -- WP DL1 -- In Modern an option chip (DESIGN-LANGUAGE §13, OptionChip's
// look): the control tone, radius 12, 24 high, at least as wide as its
// label (EffectChipRow makes a row's chips one width); in the mask it takes
// the selection tone and accent text, never an accent fill.
// It keeps its own name and `on`, which the smoke sections drive, so the
// panels lay these out themselves (EffectChipRow) instead of a ChipGroup.
AbstractButton {
    id: chip
    property bool on: false
    property string toolTip
    readonly property bool modern: LayoutController.modern   // -- WP DL1 --

    // -- WP DL1 -- In an EffectChipRow in Modern: the row's widest chip's width;
    // elsewhere (a column under the row) it fills what holds it
    readonly property real rowWidest: parent && parent.widest !== undefined ? parent.widest : 0
    Layout.fillWidth: !modern || rowWidest === 0
    Layout.preferredWidth: modern && rowWidest > 0 ? rowWidest : -1
    Layout.minimumWidth: modern ? implicitWidth : 28
    implicitHeight: modern ? Theme.controlHeight : 26
    implicitWidth: modern ? Math.max(implicitHeight, implicitContentWidth + 20)
                          : Math.max(implicitBackgroundWidth + leftInset + rightInset, implicitContentWidth + leftPadding + rightPadding)
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    checkable: false

    contentItem: Text {
        textFormat: Text.PlainText
        text: chip.text
        font: Theme.fontSmall
        color: !chip.modern ? (chip.on ? Theme.textOnAccent : chip.enabled ? Theme.textMuted : Theme.textDisabled)
             : !chip.enabled ? Theme.textDisabled : chip.on ? Theme.accentText : Theme.text
        horizontalAlignment: Text.AlignHCenter
        verticalAlignment: Text.AlignVCenter
        elide: Text.ElideRight
    }
    background: Rectangle {
        radius: chip.modern ? Math.min(Theme.radiusChip, height / 2) : 6
        color: chip.on ? (chip.modern ? Theme.selection : Theme.accent)
             : chip.modern && chip.down ? Theme.controlPressed
             : chip.hovered && chip.enabled ? Theme.controlHover : Theme.control
        opacity: chip.enabled ? 1 : 0.6
        border.width: chip.visualFocus ? 2 : chip.on || chip.modern ? 0 : 1
        border.color: chip.visualFocus ? Theme.focusRing : Theme.border
        Behavior on color { ColorAnimation { duration: 120 } }
    }

    HelpTip {
        text: chip.toolTip
        visible: chip.hovered && chip.toolTip.length > 0
    }

    Accessible.role: Accessible.CheckBox
    Accessible.name: toolTip.length ? text + ", " + toolTip : text
    Accessible.checkable: true
    Accessible.checked: on
}
