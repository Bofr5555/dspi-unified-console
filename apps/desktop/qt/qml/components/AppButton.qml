import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A push button: bordered, or filled with the accent for the default action
// of a dialog or a bar (`primary`), or red for a destructive one
// (`destructive`). An optional icon sits before the text.
// -- WP DL0 -- Modern: no outline (the control tone is the edge), radius 6,
// hover and pressed in the control tones; keyboard focus is a 2 px ring.
// -- INTC (DL7 fold) -- `danger`: a quiet destructive action outside a
// confirmation (DESIGN-LANGUAGE §7: the red fill only inside the sheet that
// asks): the secondary button's fill, its icon and text in dangerText.
Button {
    id: button
    property bool primary: false
    property bool destructive: false
    property bool danger: false
    property string iconName
    property string toolTip

    implicitHeight: Theme.controlHeight
    leftPadding: 10
    rightPadding: 10
    topPadding: 2
    bottomPadding: 2
    font: Theme.fontBody
    hoverEnabled: true
    focusPolicy: Qt.StrongFocus

    readonly property color fill: primary ? Theme.accent : destructive ? Theme.danger : Theme.control
    readonly property color ink: primary || destructive ? Theme.textOnAccent : danger ? Theme.dangerText : Theme.text

    contentItem: Item {
        implicitWidth: label.implicitWidth
        implicitHeight: label.implicitHeight
        Row {
            id: label
            anchors.centerIn: parent
            spacing: 6
            Icon {
                visible: button.iconName.length > 0
                anchors.verticalCenter: parent.verticalCenter
                name: button.iconName
                size: 14
                color: button.enabled ? button.ink : Theme.textDisabled
            }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                textFormat: Text.PlainText
                text: button.text
                font: button.font
                color: button.enabled ? button.ink : Theme.textDisabled
            }
        }
    }
    background: Rectangle {
        implicitWidth: 72
        radius: Theme.radiusControl   // -- WP DL0 --
        readonly property bool plain: LayoutController.modern && !button.primary && !button.destructive
        color: plain ? (button.down ? Theme.controlPressed : button.hovered && button.enabled ? Theme.controlHover : Theme.control)
             : button.down ? Qt.darker(button.fill, 1.15) : button.hovered && button.enabled ? Qt.lighter(button.fill, 1.08) : button.fill
        opacity: button.enabled ? 1 : 0.6
        border.width: button.visualFocus ? Theme.focusRingWidth : (button.primary || button.destructive ? 0 : Theme.controlLineWidth)
        border.color: button.visualFocus ? Theme.focusRing : Theme.controlLine   // -- WP UB13 --
    }

    HelpTip {
        text: button.toolTip
        visible: button.hovered && button.toolTip.length > 0
    }

    Accessible.role: Accessible.Button
    Accessible.name: text.length ? text : toolTip
    Accessible.description: toolTip
}
