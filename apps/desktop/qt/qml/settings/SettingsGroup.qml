import QtQuick
import QtQuick.Layouts
import UConsole

// One section of a Settings page (the Mac's Form Section): a header with its
// icon, the rows on a rounded panel, and an optional footnote under it.
// -- WP DL7 -- No outline in Modern (Theme.lineWidth). `flat` drops the panel:
// a section (its heading, then the rows on the page), for a page whose one
// group would otherwise frame everything (DESIGN-LANGUAGE §8).
ColumnLayout {
    id: group
    property string title
    property string iconName
    property string footer
    property color footerColor: Theme.textMuted
    default property alias rows: panel.data
    // Something beside the title (a refresh button)
    property alias trailing: trailingSlot.data
    property bool flat: false   // -- WP DL7 --

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
            textFormat: Text.PlainText
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
        // -- WP DL7 -- flat: no panel, no inset
        readonly property int inset: group.flat ? 0 : Theme.spacing
        implicitHeight: panel.implicitHeight + 2 * inset
        radius: Theme.radius
        color: group.flat ? "transparent" : Theme.panel
        border.width: group.flat ? 0 : Theme.lineWidth
        border.color: Theme.separator
        ColumnLayout {
            id: panel
            x: parent.inset
            y: parent.inset
            width: parent.width - 2 * parent.inset
            spacing: Theme.spacing
        }
        Accessible.role: Accessible.Grouping
        Accessible.name: group.title
    }

    Text {
        visible: group.footer.length > 0
        Layout.fillWidth: true
        textFormat: Text.PlainText
        text: group.footer
        font: Theme.fontSmall
        color: group.footerColor
        wrapMode: Text.Wrap
    }
}
