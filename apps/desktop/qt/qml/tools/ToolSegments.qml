import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- A segmented choice (the Mac's segmented Picker and the
// limiter's link-group segments): one segment per entry of `model`, the
// `current` one marked. `chosen(index)` fires on a click; the owner writes
// the value and `current` follows the device.
//
// -- WP DL0 -- Modern (DESIGN-LANGUAGE §7): one bar in the control tone with
// no outline, the chosen segment in the selection tone with accent text
// (never an accent fill), and the bar as wide as its segments
// (`contentSized`) even where the owner's layout gives it the whole row: it
// sits at the left of the room it gets. Classic keeps its bordered bar, the
// accent-filled segment and the full width.
//
//     ToolSegments {
//         objectName: "Tube.mode"            // segments are "Tube.mode.0", ".1", ...
//         label: qsTr("Mode")                // the group's accessible name
//         model: [qsTr("Basic"), qsTr("Advanced")]
//         current: panel.advanced ? 1 : 0
//         onChosen: index => panel.setAdvanced(index === 1)
//     }
Item {
    id: segments
    property var model: []
    property int current: -1
    property string label
    // -- WP DL0 -- the bar keeps to its segments' width (Modern's default)
    property bool contentSized: LayoutController.modern
    signal chosen(int index)

    implicitHeight: Theme.controlHeight
    implicitWidth: row.implicitWidth + 2 * bar.inset
    opacity: enabled ? 1 : 0.6

    Rectangle {
        id: bar
        readonly property int inset: LayoutController.modern ? 2 : 1
        width: segments.contentSized ? Math.min(segments.width, segments.implicitWidth) : segments.width
        height: segments.height
        radius: Theme.radiusControl
        color: Theme.control
        border.width: Theme.lineWidth
        border.color: Theme.border

        RowLayout {
            id: row
            anchors.fill: parent
            anchors.margins: bar.inset
            spacing: 0
            Repeater {
                model: segments.model
                AbstractButton {
                    id: segment
                    required property int index
                    required property var modelData
                    readonly property bool chosenNow: index === segments.current
                    objectName: segments.objectName.length ? segments.objectName + "." + index : ""
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    implicitWidth: label.implicitWidth + 16
                    text: modelData
                    hoverEnabled: true
                    focusPolicy: Qt.TabFocus
                    contentItem: Text {
                        id: label
                        textFormat: Text.PlainText
                        text: segment.text
                        font: Theme.fontSmall
                        color: !segment.enabled ? Theme.textDisabled
                             : segment.chosenNow ? (LayoutController.modern ? Theme.accentText : Theme.textOnAccent)
                             : Theme.text
                        horizontalAlignment: Text.AlignHCenter
                        verticalAlignment: Text.AlignVCenter
                        elide: Text.ElideRight
                    }
                    background: Rectangle {
                        radius: LayoutController.modern ? Theme.radiusControl - bar.inset : Theme.radiusSmall
                        color: segment.chosenNow ? (LayoutController.modern ? Theme.selection : Theme.accent)
                             : segment.hovered && segment.enabled ? Theme.controlHover : "transparent"
                        border.width: segment.visualFocus ? Theme.focusRingWidth : 0
                        border.color: Theme.focusRing
                    }
                    onClicked: segments.chosen(index)
                    Accessible.role: Accessible.RadioButton
                    Accessible.name: text
                    Accessible.checkable: true
                    Accessible.checked: chosenNow
                }
            }
        }
    }

    Accessible.role: Accessible.Grouping
    Accessible.name: label
}
