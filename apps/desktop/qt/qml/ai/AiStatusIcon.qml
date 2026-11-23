import QtQuick
import UConsole

// -- WP MCP4 -- Classic's sign that AI agents are on: the icon beside the device
// button, with a dot for their state (AiController.state; also in words, in the
// tooltip). A click opens the AI agents window (what they do, Pause, Stop).
IconButton {
    id: icon
    objectName: "AiStatusIcon"
    visible: AiController.enabled
    iconName: "bot"
    // -- WP AIVIS -- what it does now
    toolTip: AiController.working && AiController.work.text ? AiController.work.agent + ": " + AiController.work.text : AiController.statusLine
    onClicked: AiController.openPanel()

    Rectangle {
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.margins: 2
        width: 8
        height: 8
        radius: 4
        border.width: 1
        border.color: Theme.sidebar
        color: AiController.state === "asking" || AiController.state === "error" ? Theme.warning
             : AiController.state === "active" ? Theme.connected
             : Theme.offline
    }

    // -- WP AIVIS -- turns while an agent's call or job runs
    AiWorkMark {
        objectName: "AiStatusIcon.working"
        anchors.centerIn: parent
        width: parent.width - 2
        height: parent.height - 2
        visible: AiController.working
        working: AiController.working
    }
}
