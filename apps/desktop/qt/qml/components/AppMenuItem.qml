import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// A menu row: its text, and its shortcut on the right.
MenuItem {
    id: item
    implicitHeight: 24
    font: Theme.fontBody

    readonly property string shortcutText: action && action.shortcut ? shortcutLabel(action.shortcut) : ""
    function shortcutLabel(sequence) {
        // Action.shortcut is a string or a StandardKey; strings are what the table uses
        return typeof sequence === "string" ? sequence : ""
    }

    contentItem: RowLayout {
        spacing: Theme.spacing
        Text {
            Layout.fillWidth: true
            leftPadding: item.checkable ? 18 : 6
            text: item.text
            font: item.font
            color: item.enabled ? Theme.text : Theme.textDisabled
            elide: Text.ElideRight
        }
        Text {
            visible: item.shortcutText.length > 0
            rightPadding: 6
            text: item.shortcutText
            font: Theme.fontSmall
            color: Theme.textMuted
        }
    }
    indicator: Text {
        visible: item.checked
        x: 6
        anchors.verticalCenter: parent.verticalCenter
        text: "✓"
        color: Theme.text
        font: item.font
    }
    arrow: Icon {
        visible: item.subMenu
        x: item.width - width - 6
        anchors.verticalCenter: parent.verticalCenter
        name: "chevron-right"
        size: 12
        color: Theme.textMuted
    }
    background: Rectangle {
        color: item.highlighted ? Theme.selection : "transparent"
        radius: Theme.radiusSmall
    }
}
