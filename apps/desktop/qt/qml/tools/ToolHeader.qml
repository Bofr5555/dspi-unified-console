import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP8 -- The header of the subharmonic, tube and upmixer windows (the
// Mac's headerSection): the icon, the name and what it does, any controls the
// window adds (`trailing`: Solo, Basic / Advanced), and the on/off switch.
//
// -- WP DL0 -- In Modern it is a page's intro row (DESIGN-LANGUAGE §6): no
// background, Theme.introRowHeight (44) high, a 20 px icon in accentText, the
// name in fontTitle, the description in fontSmall muted, the switch (or, with
// `showSwitch: false`, the primary action put in `trailing`) at the right.
// The page leaves Theme.spacingSection (24) under it. Classic keeps the
// window header.
//
//     ToolHeader {
//         objectName: "TubeWindow.header"    // the switch is "TubeWindow.header.switch"
//         Layout.fillWidth: true
//         iconName: "tube"
//         title: qsTr("Tube Modeller")
//         subtitle: qsTr("Valve-style harmonic colour ...")
//         on: panel.on
//         switchEnabled: panel.writable
//         onToggled: on => panel.setOn(on)
//         AppButton { text: qsTr("Solo") }   // trailing, before the switch
//     }
Rectangle {
    id: header
    property string iconName
    property string touchPath   // -- WP AIVIS -- the effect's journal paths ("loud.")
    property string title
    property string subtitle
    property bool on: false
    property bool switchEnabled: true
    property bool showSwitch: true   // -- WP DL0 -- false: an intro row with no on/off
    default property alias trailing: extras.data
    signal toggled(bool on)

    implicitHeight: LayoutController.modern ? Math.max(Theme.introRowHeight, row.implicitHeight)   // -- WP DL0 --
                                            : row.implicitHeight + 2 * Theme.spacing
    color: LayoutController.modern ? "transparent" : Theme.window

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.leftMargin: Theme.spacingLarge
        anchors.rightMargin: Theme.spacingLarge
        spacing: Theme.spacing

        Icon {
            name: header.iconName
            size: LayoutController.modern ? 20 : 22   // -- WP DL0 --
            color: Theme.accentText
        }
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            Text {
                textFormat: Text.PlainText
                text: header.title
                font: Theme.fontTitle
                color: Theme.text
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: header.subtitle
                font: LayoutController.modern ? Theme.fontSmall : Theme.fontCaption   // -- WP DL0 --
                color: Theme.textMuted
                elide: Text.ElideRight
                visible: text.length > 0
            }
        }
        RowLayout {
            id: extras
            spacing: Theme.spacing
        }
        AppSwitch {
            objectName: header.objectName.length ? header.objectName + ".switch" : ""
            text: ""
            visible: header.showSwitch   // -- WP DL0 --
            checked: header.on
            enabled: header.switchEnabled
            onToggled: header.toggled(checked)
            Accessible.name: header.title
        }
    }

    // -- WP AIVIS -- an agent changed it just now
    AiTouched { objectName: header.objectName + ".touched"; path: header.touchPath.length > 0 ? header.touchPath : "-" }
}
