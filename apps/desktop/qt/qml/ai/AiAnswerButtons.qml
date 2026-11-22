import QtQuick
import UConsole

// -- WP MCP4 -- Deny and Allow. Neither takes a click or a key in the first
// moments of a question (`question` changed): what the person was doing
// when it came up must not answer it, nor one answer the next question too.
// Deny has the focus.
Row {
    id: row
    property var question
    property string prefix
    readonly property bool armed: !arming.running
    signal answered(bool allow)
    spacing: Theme.spacingSmall

    onQuestionChanged: arming.restart()
    Component.onCompleted: arming.restart()
    Timer {
        id: arming
        interval: 700
        onTriggered: if (row.takeFocus) deny.forceActiveFocus()
    }
    property bool takeFocus: false

    AppButton {
        id: deny
        objectName: row.prefix + ".deny"
        text: qsTr("Deny")
        enabled: row.armed
        onClicked: row.answered(false)
    }
    AppButton {
        objectName: row.prefix + ".allow"
        text: qsTr("Allow")
        primary: true
        enabled: row.armed
        onClicked: row.answered(true)
    }
}
