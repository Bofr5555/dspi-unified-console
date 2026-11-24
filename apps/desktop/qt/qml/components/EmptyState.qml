import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP DL0 -- What a page shows when it has nothing to show (DESIGN-LANGUAGE
// §16), instead of a bare sentence, a warning-coloured one or an empty
// skeleton: centred in the room it gets, a 32 px icon in textMuted, a title
// in fontTitle, one sentence in fontBody muted, and at most one action (a
// primary button when it is the next step, a secondary one otherwise).
//
// Give it the area to centre in (anchors.fill, or Layout.fillWidth and
// Layout.fillHeight); its implicit size is the content's.
//
//     EmptyState {
//         objectName: "Snapshots.empty"      // its button is "Snapshots.empty.action"
//         anchors.fill: parent
//         iconName: "camera"                 // a Lucide name
//         title: qsTr("No snapshots yet")
//         text: qsTr("Capture the device's settings to compare them later.")
//         actionText: qsTr("Capture…")       // empty: no button
//         actionPrimary: true
//         onAction: snapshots.capture()
//     }
Item {
    id: empty
    property string iconName: "circle-dashed"
    property string title
    property string text
    property string actionText
    property bool actionPrimary: false
    property bool actionEnabled: true
    signal action()

    implicitWidth: column.implicitWidth + 2 * Theme.spacingLarge
    implicitHeight: column.implicitHeight + 2 * Theme.spacingLarge

    ColumnLayout {
        id: column
        anchors.centerIn: parent
        width: Math.min(implicitWidth, Math.max(0, empty.width - 2 * Theme.spacingLarge))
        spacing: Theme.spacingSmall

        Icon {
            Layout.alignment: Qt.AlignHCenter
            Layout.bottomMargin: 4
            visible: empty.iconName.length > 0
            name: empty.iconName
            size: 32
            color: Theme.textMuted
        }
        Text {
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
            Layout.maximumWidth: 420
            visible: text.length > 0
            textFormat: Text.PlainText
            text: empty.title
            font: Theme.fontTitle
            color: Theme.text
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Text {
            Layout.alignment: Qt.AlignHCenter
            Layout.fillWidth: true
            Layout.maximumWidth: 420
            visible: text.length > 0
            textFormat: Text.PlainText
            text: empty.text
            font: Theme.fontBody
            color: Theme.textMuted
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            Accessible.role: Accessible.StaticText
            Accessible.name: text
        }
        AppButton {
            objectName: empty.objectName.length ? empty.objectName + ".action" : ""
            Layout.alignment: Qt.AlignHCenter
            Layout.topMargin: 4
            visible: empty.actionText.length > 0
            text: empty.actionText
            primary: empty.actionPrimary
            enabled: empty.actionEnabled
            onClicked: empty.action()
        }
    }

    Accessible.role: Accessible.Grouping
    Accessible.name: title
}
