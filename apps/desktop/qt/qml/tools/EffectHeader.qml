import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- An effect window's header (the Mac's headerSection): the
// effect's icon, its name and what it is, and its on/off switch.
Rectangle {
    id: header
    property string iconName
    property string touchPath   // -- WP AIVIS -- the effect's journal paths ("loud.")
    property string title
    property string subtitle
    property bool on: false
    property bool switchEnabled: true
    signal toggled(bool on)

    implicitHeight: row.implicitHeight + 2 * Theme.spacing
    color: Theme.window

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.leftMargin: Theme.spacingLarge
        anchors.rightMargin: Theme.spacingLarge
        spacing: Theme.spacing

        Icon {
            name: header.iconName
            size: 22
            color: Theme.accentText
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            Text {
                text: header.title
                font: Theme.fontTitle
                color: Theme.text
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            Text {
                Layout.fillWidth: true
                text: header.subtitle
                font: Theme.fontCaption
                color: Theme.textMuted
                elide: Text.ElideRight
            }
        }
        AppSwitch {
            objectName: header.objectName.length ? header.objectName + ".switch" : ""
            text: ""
            checked: header.on
            enabled: header.switchEnabled
            onToggled: header.toggled(checked)
            Accessible.name: header.title
        }
    }

    // -- WP AIVIS -- an agent changed it just now
    AiTouched { objectName: header.objectName + ".touched"; path: header.touchPath.length > 0 ? header.touchPath : "-" }
}
