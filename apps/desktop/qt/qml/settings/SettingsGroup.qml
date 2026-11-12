import QtQuick
import QtQuick.Layouts
import UConsole

// One section of a Settings page (the Mac's Form Section): a header with its
// icon, the rows on a rounded panel, and an optional footnote under it.
ColumnLayout {
    id: group
    property string title
    property string iconName
    property string footer
    property color footerColor: Theme.textMuted
    default property alias rows: panel.data
    // Something beside the title (a refresh button)
    property alias trailing: trailingSlot.data

    Layout.fillWidth: true
    spacing: 6

    RowLayout {
        visible: group.title.length > 0
        Layout.fillWidth: true
        spacing: 6
        Icon {
            visible: group.iconName.length > 0
            name: group.iconName
            size: 14
            color: Theme.textMuted
        }
        Text {
            Layout.fillWidth: true
            text: group.title
            font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
            color: Theme.text
            elide: Text.ElideRight
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Item {
            id: trailingSlot
            implicitWidth: childrenRect.width
            implicitHeight: childrenRect.height
        }
    }

    Rectangle {
        Layout.fillWidth: true
        implicitHeight: panel.implicitHeight + 2 * Theme.spacing
        radius: Theme.radius
        color: Theme.panel
        border.width: 1
        border.color: Theme.separator
        ColumnLayout {
            id: panel
            x: Theme.spacing
            y: Theme.spacing
            width: parent.width - 2 * Theme.spacing
            spacing: Theme.spacing
        }
        Accessible.role: Accessible.Grouping
        Accessible.name: group.title
    }

    Text {
        visible: group.footer.length > 0
        Layout.fillWidth: true
        text: group.footer
        font: Theme.fontSmall
        color: group.footerColor
        wrapMode: Text.Wrap
    }
}
