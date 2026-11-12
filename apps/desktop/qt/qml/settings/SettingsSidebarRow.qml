import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// One row of the Settings page list: a group heading, or a page with its
// tinted badge (the Mac's SettingsBadge: a rounded square in the page's tint,
// the icon in white). `row` is one entry of SettingsController.sidebarRows.
Item {
    id: item
    property var row: ({})
    readonly property bool isHeading: row.heading !== undefined
    readonly property bool current: !isHeading && row.id === SettingsController.currentPage

    objectName: isHeading ? "Settings.heading" : "Settings.row." + row.id
    implicitHeight: isHeading ? heading.implicitHeight : Theme.rowHeight

    Text {
        id: heading
        visible: item.isHeading
        width: parent.width
        text: item.isHeading ? row.heading : ""
        font: Theme.fontHeading
        color: Theme.textMuted
        topPadding: Theme.spacingSmall
        bottomPadding: 2
        leftPadding: 6
        Accessible.role: Accessible.Heading
        Accessible.name: text
        Accessible.ignored: !item.isHeading
    }

    AbstractButton {
        id: button
        objectName: item.isHeading ? "" : "Settings.button." + row.id
        visible: !item.isHeading
        anchors.fill: parent
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        onClicked: SettingsController.open(row.id)

        background: Rectangle {
            radius: Theme.radiusSmall
            color: item.current ? Theme.selection : button.hovered ? Theme.controlHover : "transparent"
            border.width: button.visualFocus ? 2 : 0
            border.color: Theme.focusRing
        }
        contentItem: RowLayout {
            spacing: 6
            Rectangle {
                Layout.leftMargin: 6
                implicitWidth: 18
                implicitHeight: 18
                radius: Theme.radiusSmall
                color: item.isHeading ? "transparent" : Theme.settingsTint(row.id)
                Icon {
                    anchors.centerIn: parent
                    name: item.isHeading ? "" : row.icon
                    size: 12
                    color: Theme.textOnAccent
                }
            }
            Text {
                Layout.fillWidth: true
                text: item.isHeading ? "" : row.title
                font: Theme.fontBody
                color: Theme.text
                elide: Text.ElideRight
            }
        }
        Accessible.role: Accessible.ListItem
        Accessible.name: item.isHeading ? "" : row.title
        Accessible.selected: item.current
    }
}
