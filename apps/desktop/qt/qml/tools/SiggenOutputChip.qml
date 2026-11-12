import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP9 -- One output in the generator's grid (the Mac's chip): tinted
// while selected, "ø" while inverted, a green edge on the output a walk is
// playing, dimmed when the matrix has the output off.
AbstractButton {
    id: chip
    property bool selected: false
    property bool inverted: false
    property bool walking: false
    property bool matrixEnabled: true
    property string shownName: text + (inverted ? " ø" : "")

    implicitHeight: 26
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    opacity: matrixEnabled ? 1 : 0.4

    background: Rectangle {
        radius: 6
        color: chip.selected ? Qt.alpha(Theme.accent, 0.18) : chip.hovered && chip.enabled ? Theme.controlHover : Theme.control
        border.width: chip.visualFocus || chip.walking ? 1.5 : 1
        border.color: chip.visualFocus ? Theme.focusRing : chip.walking ? Theme.success
                    : chip.selected ? Qt.alpha(Theme.accent, 0.6) : Theme.separator
    }

    contentItem: RowLayout {
        spacing: 4
        Text {
            Layout.fillWidth: true
            text: chip.text
            font: Qt.font({ pixelSize: Theme.fontSmall.pixelSize, weight: chip.selected ? Font.DemiBold : Font.Normal })
            color: chip.selected ? Theme.text : chip.enabled ? Theme.textMuted : Theme.textDisabled
            horizontalAlignment: Text.AlignHCenter
            elide: Text.ElideRight
        }
        Text {
            visible: chip.inverted
            text: "ø"
            font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontSmall.pixelSize, weight: Font.Bold })
            color: Theme.warning
        }
    }

    HelpTip {
        text: chip.matrixEnabled ? qsTr("Click: on → inverted → off")
                                 : qsTr("Output disabled in the matrix mixer - selected but silent until enabled")
        visible: chip.hovered
    }

    Accessible.role: Accessible.CheckBox
    Accessible.name: shownName
    Accessible.checkable: true
    Accessible.checked: selected
}
