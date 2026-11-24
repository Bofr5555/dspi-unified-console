import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP DL0 -- An inline notice in a page where a condition blocks or limits
// an action (DESIGN-LANGUAGE §14), in place of a bare orange sentence: at
// least 32 high, Theme.bannerFill(kind), radius 6, an icon in
// Theme.bannerEdge(kind), one sentence in Theme.text (wrapping when narrow),
// and an optional action at the right as a secondary button. One notice per
// place; help text under a control stays fontSmall muted, not a notice.
//
// kind: "info" (default), "warning" or "error".
//
//     NoticeRow {
//         objectName: "Measure.notice"       // its button is "Measure.notice.action"
//         Layout.fillWidth: true
//         kind: "warning"
//         text: qsTr("Choose a microphone to measure.")
//         actionText: qsTr("Choose…")        // empty: no button
//         onAction: measure.pickMicrophone()
//         // more buttons can go in as children; they sit after the action
//     }
Rectangle {
    id: notice
    property string kind: "info"
    property string text
    property string iconName: kind === "error" ? "circle-alert" : kind === "warning" ? "triangle-alert" : "info"
    property string actionText
    property bool actionEnabled: true
    property int textFormat: Text.PlainText
    default property alias actions: extra.data
    signal action()

    implicitWidth: row.implicitWidth + 2 * Theme.spacing
    implicitHeight: Math.max(32, row.implicitHeight + 2 * 6)
    radius: Theme.radiusControl
    color: Theme.bannerFill(kind)

    RowLayout {
        id: row
        anchors.fill: parent
        anchors.leftMargin: Theme.spacing
        anchors.rightMargin: 6
        anchors.topMargin: 6
        anchors.bottomMargin: 6
        spacing: Theme.spacingSmall
        Icon {
            Layout.alignment: message.lineCount > 1 ? Qt.AlignTop : Qt.AlignVCenter
            name: notice.iconName
            color: Theme.bannerEdge(notice.kind)
        }
        Text {
            id: message
            Layout.fillWidth: true
            Layout.alignment: Qt.AlignVCenter
            textFormat: notice.textFormat
            text: notice.text
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
            onLinkActivated: link => Qt.openUrlExternally(link)
        }
        AppButton {
            objectName: notice.objectName.length ? notice.objectName + ".action" : ""
            visible: notice.actionText.length > 0
            text: notice.actionText
            enabled: notice.actionEnabled
            onClicked: notice.action()
        }
        RowLayout {
            id: extra
            spacing: Theme.spacingSmall
            visible: children.length > 0
        }
    }

    Accessible.role: kind === "info" ? Accessible.StaticText : Accessible.AlertMessage
    Accessible.name: text
}
