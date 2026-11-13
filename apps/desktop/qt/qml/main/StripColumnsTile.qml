import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP QP9 -- A layout choice in the bar strip's popover (the Mac's tile): a
// small picture of two rows of `columns` cells over the number.
AbstractButton {
    id: tile
    property int columns: 1
    property bool chosen: false
    readonly property color ink: chosen ? Theme.accentText : Theme.textMuted

    implicitHeight: 38
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    text: String(columns)

    background: Rectangle {
        color: tile.chosen ? Qt.alpha(Theme.accent, 0.16) : (tile.hovered ? Theme.controlHover : Theme.control)
        border.color: tile.visualFocus ? Theme.focusRing : (tile.chosen ? Qt.alpha(Theme.accent, 0.7) : Theme.separator)
        border.width: tile.visualFocus ? 2 : 1
        radius: 6
    }
    contentItem: Item {
        Grid {
            id: picture
            y: 5
            anchors.horizontalCenter: parent.horizontalCenter
            columns: tile.columns
            spacing: 1
            Repeater {
                model: 2 * tile.columns
                Rectangle {
                    width: (22 - (tile.columns - 1)) / tile.columns
                    height: 6
                    radius: 1
                    color: tile.ink
                    opacity: 0.7
                }
            }
        }
        Text {
            anchors { top: picture.bottom; topMargin: 2; horizontalCenter: parent.horizontalCenter }
            text: tile.text
            color: tile.ink
            font.pixelSize: 9
            font.bold: tile.chosen
        }
    }
    HelpTip {
        visible: tile.hovered
        text: tile.columns === 1 ? qsTr("Stack the channels in one column")
                                 : qsTr("Lay the channels out in up to %1 columns").arg(tile.columns)
    }
    Accessible.role: Accessible.RadioButton
    Accessible.name: text
    Accessible.checked: chosen
}
