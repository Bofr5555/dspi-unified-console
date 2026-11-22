import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY4 -- The question before an output's enable switches the other side
// of Core 1 off (ChannelController.core1Question), for the page that shows.
QuestionDialog {
    id: question
    property bool active: false
    property int output: -1
    ofDevice: true
    parent: Overlay.overlay
    onAnswered: answer => { if (answer === "accept") ChannelController.confirmOutputEnable(question.output) }

    readonly property Connections asked: Connections {
        target: question.active ? ChannelController : null
        function onCore1Question(output, title, text, acceptText) {
            question.output = output
            question.title = title
            question.text = text
            question.acceptText = acceptText
            question.open()
        }
    }
}
