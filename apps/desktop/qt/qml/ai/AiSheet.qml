import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP MCP4 -- The frame of what AI agents ask the user: a dialog of the main
// window that an answer closes, never a click outside it. Escape denies, once
// the buttons take answers. It closes by itself when there is nothing to ask
// any more (answered elsewhere, not answered in time, the agent gone).
Dialog {
    id: sheet
    property string heading
    property var question           // what the buttons are for; it arms them anew
    property bool wanted: false
    property string prefix
    default property alias body: column.data
    readonly property alias armed: answers.armed
    signal answered(bool allow)

    modal: true
    focus: true
    closePolicy: Popup.NoAutoClose
    onWantedChanged: wanted ? open() : close()
    Component.onCompleted: if (wanted) open()

    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 320
    width: Math.min(460, hostWidth - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: Math.max(Theme.spacingLarge, (hostHeight - implicitHeight) / 3)
    padding: Theme.spacingLarge
    topPadding: Theme.spacingSmall

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.color: Theme.border
    }
    header: Item {
        implicitHeight: titleText.height
        Text {
            id: titleText
            width: parent.width
            leftPadding: Theme.spacingLarge
            rightPadding: Theme.spacingLarge
            topPadding: Theme.spacingLarge
            textFormat: Text.PlainText
            text: sheet.heading
            font: Theme.fontTitle
            color: Theme.text
            wrapMode: Text.Wrap
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
    }
    contentItem: Item {
        implicitHeight: column.height
        Accessible.role: Accessible.AlertMessage
        Accessible.name: sheet.heading
        Keys.onEscapePressed: if (answers.armed) sheet.answered(false)
        Column {
            id: column
            width: parent.width
            spacing: Theme.spacingSmall
        }
    }
    footer: Item {
        implicitHeight: answers.implicitHeight + 2 * Theme.spacing
        Keys.onEscapePressed: if (answers.armed) sheet.answered(false)
        AiAnswerButtons {
            id: answers
            x: parent.width - width - Theme.spacingLarge
            y: Theme.spacing
            prefix: sheet.prefix
            question: sheet.question
            takeFocus: sheet.opened
            onAnswered: allow => sheet.answered(allow)
        }
    }
}
