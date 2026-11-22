import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The row over a page: the section's title and its tabs, and the
// page's "Open in window". The tabs are underlined beside the title with the
// rail (variant B), pills with the sections across the top (variant A).
Item {
    id: bar
    objectName: "PageTabs"
    property string section: ""
    property string tab: ""
    property bool pills: false
    readonly property var entry: SectionTable.tab(section, tab)
    readonly property var entries: SectionTable.tabs(section).filter(t => !t.quiet || t.tab === tab)
    readonly property string action: entry && entry.row ? entry.row.action : ""
    signal chosen(string tab)

    implicitHeight: 38

    RowLayout {
        anchors.fill: parent
        anchors.leftMargin: Theme.spacing
        anchors.rightMargin: Theme.spacingSmall
        spacing: Theme.spacing

        Text {
            objectName: "PageTabs.title"
            visible: !bar.pills
            text: { const s = SectionTable.section(bar.section); return s ? s.title : "" }
            font: Theme.fontTitle
            color: Theme.text
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Flickable {
            id: flick
            Layout.fillWidth: true
            Layout.fillHeight: true
            contentWidth: tabs.implicitWidth
            contentHeight: height
            boundsBehavior: Flickable.StopAtBounds
            interactive: contentWidth > width
            clip: true
            // The current tab stays in sight
            function reveal(item) {
                if (item.x < contentX) contentX = item.x
                else if (item.x + item.width > contentX + width) contentX = item.x + item.width - width
            }
            Row {
                id: tabs
                // Tabs for a section with parts; a section of one page needs none
                visible: bar.entries.length > 1
                height: parent.height
                spacing: bar.pills ? 4 : 2
                Accessible.role: Accessible.PageTabList
                Accessible.name: qsTr("Pages")
                Repeater {
                    model: bar.entries
                    delegate: AbstractButton {
                        id: button
                        required property var modelData
                        objectName: "PageTabs." + modelData.tab
                        checked: modelData.tab === bar.tab
                        text: modelData.title
                        hoverEnabled: true
                        focusPolicy: Qt.TabFocus
                        leftPadding: 12
                        rightPadding: 12
                        height: bar.pills ? 26 : tabs.height
                        anchors.verticalCenter: parent ? parent.verticalCenter : undefined
                        onCheckedChanged: if (checked) flick.reveal(button)
                        onVisualFocusChanged: if (visualFocus) flick.reveal(button)
                        contentItem: Text {
                            text: button.text
                            font: Theme.fontBody
                            color: button.checked ? Theme.text : Theme.textMuted
                            verticalAlignment: Text.AlignVCenter
                        }
                        background: Rectangle {
                            radius: Theme.radiusSmall
                            color: bar.pills && button.checked ? Theme.control : button.hovered ? Theme.controlHover : "transparent"
                            border.width: button.visualFocus ? 2 : 0
                            border.color: Theme.focusRing
                            Rectangle {
                                visible: !bar.pills && button.checked
                                anchors.bottom: parent.bottom
                                width: parent.width
                                height: 2
                                color: Theme.accent
                            }
                        }
                        onClicked: bar.chosen(modelData.tab)
                        Accessible.role: Accessible.PageTab
                        Accessible.name: text
                        Accessible.selected: checked
                        Accessible.onPressAction: button.clicked()
                    }
                }
            }
        }
        IconButton {
            objectName: "PageTabs.openWindow"
            visible: bar.action.length > 0
            enabled: Shell.allows(bar.action) && Shell.shownWindows.indexOf(bar.action) < 0
            iconName: "app-window"
            toolTip: qsTr("Open in Window")
            onClicked: Shell.openWindow(bar.action)
        }
    }
}
