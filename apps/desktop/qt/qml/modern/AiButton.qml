import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY8 -- The top bar's AI button (MODLAY 10; the owner's: always visible,
// labelled "AI"): the agents' state as a mark, in words in the tooltip, and the
// popover under it. The mark: off an empty grey ring; on and waiting for an agent
// an amber ring, which pulses a few times as it begins; paused a grey dot; an agent
// connected a green dot on a highlighted button; something to answer or a start
// that failed an amber dot.
AbstractButton {
    id: button
    // Another object with AiController's properties, in the smoke driver
    property QtObject ai: AiController
    readonly property string face: {
        switch (ai.state) {
        case "off": return "off"
        case "active": return "connected"
        case "paused": return "paused"
        case "asking":
        case "error": return "attention"
        }
        return "waiting"   // on, starting
    }
    readonly property alias popover: popover

    text: "AI"
    implicitHeight: 28
    implicitWidth: row.implicitWidth + 18
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    onClicked: popover.opened ? popover.close() : popover.open()

    contentItem: Item {
        Row {
            id: row
            anchors.centerIn: parent
            spacing: 6
            Rectangle {
                id: mark
                objectName: "AiButton.mark"
                readonly property bool ring: button.face === "off" || button.face === "waiting"
                anchors.verticalCenter: parent.verticalCenter
                width: 8
                height: 8
                radius: 4
                color: ring ? "transparent" : button.face === "connected" ? Theme.connected
                     : button.face === "attention" ? Theme.warning : Theme.offline
                border.width: ring ? 1.5 : 0
                border.color: button.face === "off" ? Theme.textMuted : Theme.warning
                // Under five seconds in all, so that nothing moves for good (WCAG 2.2.2)
                SequentialAnimation on opacity {
                    running: button.face === "waiting" && button.visible
                    loops: 3
                    NumberAnimation { to: 0.35; duration: 750; easing.type: Easing.InOutSine }
                    NumberAnimation { to: 1; duration: 750; easing.type: Easing.InOutSine }
                    onStopped: mark.opacity = 1
                }
            }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: button.text
                font: Theme.fontBody
                color: Theme.text
            }
        }
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        color: button.face === "connected" ? Theme.selection
             : button.hovered || popover.opened ? Theme.controlHover : Theme.control
        border.width: button.visualFocus ? 2 : button.face === "connected" ? 1 : 0
        border.color: button.visualFocus ? Theme.focusRing : Theme.success
    }

    Accessible.role: Accessible.Button
    Accessible.name: text
    Accessible.description: ai.statusLine
    Accessible.onPressAction: button.clicked()
    HelpTip {
        text: button.ai.statusLine
        visible: button.hovered && !popover.opened
    }

    AiPopover {
        id: popover
        objectName: "AiPopover"
        ai: button.ai
        x: button.width - width
        y: button.height + 4
        width: Math.min(380, button.Window.width - 2 * Theme.spacingLarge)
        maxHeight: button.Window.height - 48 - 2 * Theme.spacingLarge
    }
}
