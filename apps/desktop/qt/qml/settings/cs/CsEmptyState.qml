import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP12 -- A list with nothing in it yet (the Mac's empty states): a hint
// of what can go in it, a title, a line, and the button that adds one.
ColumnLayout {
    id: empty
    property var badgeTypes: []
    property string iconName
    property string title
    property string text
    default property alias action: actionSlot.data

    Layout.fillWidth: true
    spacing: 14

    Row {
        Layout.alignment: Qt.AlignHCenter
        spacing: 10
        visible: empty.badgeTypes.length > 0
        Repeater {
            model: empty.badgeTypes
            delegate: CsBadge {
                required property var modelData
                type: modelData
                size: 28
            }
        }
    }
    Icon {
        Layout.alignment: Qt.AlignHCenter
        visible: empty.badgeTypes.length === 0 && empty.iconName.length > 0
        name: empty.iconName
        size: 28
        color: Theme.textMuted
    }
    ColumnLayout {
        Layout.alignment: Qt.AlignHCenter
        spacing: 3
        Text {
            Layout.alignment: Qt.AlignHCenter
            text: empty.title
            font: Theme.fontTitle
            color: Theme.text
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Text {
            Layout.alignment: Qt.AlignHCenter
            Layout.maximumWidth: 340
            text: empty.text
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
            horizontalAlignment: Qt.AlignHCenter
        }
    }
    Item {
        id: actionSlot
        Layout.alignment: Qt.AlignHCenter
        implicitWidth: childrenRect.width
        implicitHeight: childrenRect.height
    }
}
