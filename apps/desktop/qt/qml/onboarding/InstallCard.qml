import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP17 -- The board step's card (the Mac's InstallStateCard): an icon,
// spinning while the app or the board is at work, a title, the message and an
// optional action button; or a progress bar while firmware is written.
Rectangle {
    id: card
    property string iconName
    property color tint: Theme.accentText
    property bool spinning: false
    property int iconSize: 28
    property string title
    property string message
    property string actionText
    property bool actionPrimary: true
    // 0..1 shows a bar under the message; below 0 none
    property real fraction: -1
    signal action()

    Layout.fillWidth: true
    implicitHeight: column.implicitHeight + 2 * Theme.spacingLarge
    radius: Theme.radius
    color: Theme.panel
    border.width: 1
    border.color: Theme.separator

    ColumnLayout {
        id: column
        anchors.fill: parent
        anchors.margins: Theme.spacingLarge
        spacing: Theme.spacing

        Icon {
            id: glyph
            Layout.alignment: Qt.AlignHCenter
            name: card.spinning ? "loader-circle" : card.iconName
            size: card.iconSize
            color: card.tint
        }
        RotationAnimator {
            target: glyph
            running: card.spinning && card.visible
            from: 0
            to: 360
            duration: 1200
            loops: Animation.Infinite
            // Back upright when it stops
            onRunningChanged: if (!running) glyph.rotation = 0
        }
        Text {
            objectName: "GettingStarted.cardTitle"
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: card.title
            font: Qt.font({ pixelSize: 15, weight: Font.DemiBold })
            color: Theme.text
            wrapMode: Text.Wrap
        }
        Text {
            objectName: "GettingStarted.cardText"
            Layout.fillWidth: true
            horizontalAlignment: Text.AlignHCenter
            text: card.message
            font: Theme.fontBody
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        Rectangle {
            visible: card.fraction >= 0
            Layout.fillWidth: true
            Layout.maximumWidth: 360
            Layout.alignment: Qt.AlignHCenter
            implicitHeight: 6
            radius: 3
            color: Theme.separator
            Rectangle {
                width: parent.width * Math.max(0, Math.min(1, card.fraction))
                height: parent.height
                radius: 3
                color: Theme.accent
            }
            Accessible.role: Accessible.ProgressBar
            Accessible.name: qsTr("%1 percent written").arg(Math.round(card.fraction * 100))
        }
        AppButton {
            objectName: "GettingStarted.cardAction"
            visible: card.actionText.length > 0
            Layout.alignment: Qt.AlignHCenter
            text: card.actionText
            primary: card.actionPrimary
            onClicked: card.action()
        }
    }

    Accessible.role: Accessible.Grouping
    Accessible.name: title
}
