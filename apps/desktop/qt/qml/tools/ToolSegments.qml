import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- A segmented choice (the Mac's segmented Picker and the
// limiter's link-group segments): one segment per entry of `model`, the
// `current` one filled with the accent. `chosen(index)` fires on a click;
// the owner writes the value and `current` follows the device.
Rectangle {
    id: segments
    property var model: []
    property int current: -1
    property string label
    signal chosen(int index)

    implicitHeight: Theme.controlHeight
    implicitWidth: row.implicitWidth + 2
    radius: Theme.radiusSmall
    color: Theme.control
    border.width: 1
    border.color: Theme.border
    opacity: enabled ? 1 : 0.6

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.margins: 1
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
                    text: segment.text
                    font: Theme.fontSmall
                    color: segment.chosenNow ? Theme.textOnAccent : segment.enabled ? Theme.text : Theme.textDisabled
                    horizontalAlignment: Text.AlignHCenter
                    verticalAlignment: Text.AlignVCenter
                    elide: Text.ElideRight
                }
                background: Rectangle {
                    radius: Theme.radiusSmall
                    color: segment.chosenNow ? Theme.accent
                                             : segment.hovered && segment.enabled ? Theme.controlHover : "transparent"
                    border.width: segment.visualFocus ? 2 : 0
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

    Accessible.role: Accessible.Grouping
    Accessible.name: label
}
