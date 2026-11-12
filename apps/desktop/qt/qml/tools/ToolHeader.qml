import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- The header of the subharmonic, tube and upmixer windows (the
// Mac's headerSection): the icon, the name and what it does, any controls the
// window adds (`trailing`: Solo, Basic / Advanced), and the on/off switch.
Rectangle {
    id: header
    property string iconName
    property string title
    property string subtitle
    property bool on: false
    property bool switchEnabled: true
    default property alias trailing: extras.data
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
        RowLayout {
            id: extras
            spacing: Theme.spacing
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
}
