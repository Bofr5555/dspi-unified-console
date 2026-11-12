import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- A section title of an effect window ("PARAMETERS", "OUTPUTS")
// with an optional control at its right (a Presets menu).
RowLayout {
    id: section
    property string text
    default property alias trailing: tail.data

    Layout.fillWidth: true
    spacing: Theme.spacingSmall
    SectionHeading {
        Layout.fillWidth: true
        text: section.text
    }
    Row {
        id: tail
        spacing: Theme.spacingSmall
    }
}
