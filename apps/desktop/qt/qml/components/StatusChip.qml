import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP DL0 fold -- A state shown, not chosen (DESIGN-LANGUAGE §13): a pill
// in the control tone, a 6 px dot in the state's colour, the words in
// fontSmall text. It takes no click; `toolTip` explains it on hover. Live's
// "Running", Overview's "Meters stale" and the like.
//
//     StatusChip {
//         objectName: "MeasureLive.status"
//         kind: mc.liveIssueText.length ? "warning" : running ? "success" : "idle"
//         text: mc.liveText
//         toolTip: qsTr("…")                 // optional
//     }
//
// kind: "success", "warning", "error", "info" or "idle" (muted).
Rectangle {
    id: chip
    property string kind: "idle"
    property string text
    property string toolTip

    implicitWidth: row.implicitWidth + 20
    implicitHeight: Math.max(22, row.implicitHeight + 6)
    radius: height / 2
    color: Theme.control

    readonly property color dot: kind === "success" ? Theme.success
                               : kind === "warning" ? Theme.warning
                               : kind === "error" ? Theme.danger
                               : kind === "info" ? Theme.accent
                               : Theme.textMuted

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.leftMargin: 10
        anchors.rightMargin: 10
        spacing: 6
        Rectangle {
            implicitWidth: 6
            implicitHeight: 6
            radius: 3
            color: chip.dot
        }
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: chip.text
            font: Theme.fontSmall
            color: Theme.text
            elide: Text.ElideRight
        }
    }

    HoverHandler { id: hover }
    HelpTip {
        text: chip.toolTip
        visible: hover.hovered && chip.toolTip.length > 0
    }

    Accessible.role: Accessible.StatusBar
    Accessible.name: text
    Accessible.description: toolTip
}
