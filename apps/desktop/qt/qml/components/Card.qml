import QtQuick
import QtQuick.Layouts
import UConsole

// A rounded panel with an optional title, for grouped controls (the Mac's
// cards and grouped forms). Children go in a column.
// -- WP DL0 -- No outline in Modern: the panel tone on the window is the edge.
Rectangle {
    id: card
    property string title
    default property alias content: column.data
    property int padding: Theme.spacing

    implicitWidth: column.implicitWidth + 2 * padding
    implicitHeight: column.implicitHeight + 2 * padding
    radius: Theme.radius
    color: Theme.panel
    border.width: Theme.lineWidth   // -- WP DL0 --
    border.color: Theme.separator

    ColumnLayout {
        id: column
        anchors.fill: parent
        anchors.margins: card.padding
        spacing: Theme.spacingSmall
        Text {
            visible: card.title.length > 0
            textFormat: Text.PlainText
            text: card.title
            font: Theme.fontHeading
            color: Theme.textMuted
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
    }

    Accessible.role: Accessible.Grouping
    Accessible.name: title
}
