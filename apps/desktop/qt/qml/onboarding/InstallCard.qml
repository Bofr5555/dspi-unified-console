import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP17 -- The board step's card (the Mac's InstallStateCard): an icon,
// turning while the app or the board is at work, a heading, the detail and an
// optional button; a bar shows how much is written.
Rectangle {
    id: card
    property string glyph
    property color tint: Theme.accentText
    property bool busy: false
    property int glyphSize: 28
    property string heading
    property string detail
    property string actionText
    property bool actionPrimary: true
    // Written so far, 0..1; below 0 there is no bar
    property real progress: -1
    signal action()

    Layout.fillWidth: true
    implicitHeight: stack.implicitHeight + 2 * Theme.spacingLarge
    radius: Theme.radius
    color: Theme.panel
    border {
        width: 1
        color: Theme.separator
    }

    ColumnLayout {
        id: stack
        anchors {
            fill: parent
            margins: Theme.spacingLarge
        }
        spacing: Theme.spacing

        Icon {
            id: mark
            Layout.alignment: Qt.AlignHCenter
            name: card.busy ? "loader-circle" : card.glyph
            size: card.glyphSize
            color: card.tint
        }
        RotationAnimator {
            id: turning
            target: mark
            from: 0
            to: 360
            duration: 1200
            loops: Animation.Infinite
            running: card.busy && card.visible
            // Upright again once the work is done
            onStopped: mark.rotation = 0
        }
        Text {
            objectName: "GettingStarted.cardTitle"
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: card.heading
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            color: Theme.text
            font.pixelSize: Theme.fontTitle.pixelSize
            font.weight: Font.DemiBold
        }
        Text {
            objectName: "GettingStarted.cardText"
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: card.detail
            horizontalAlignment: Text.AlignHCenter
            wrapMode: Text.Wrap
            color: Theme.textMuted
            font: Theme.fontBody
        }
        Item {
            visible: card.progress >= 0
            Layout.alignment: Qt.AlignHCenter
            Layout.preferredWidth: Math.min(360, stack.width)
            implicitHeight: 6
            readonly property real share: Math.min(Math.max(card.progress, 0), 1)
            Rectangle {
                id: groove
                anchors.fill: parent
                radius: 3
                color: Theme.separator
            }
            Rectangle {
                height: groove.height
                width: Math.round(groove.width * parent.share)
                radius: groove.radius
                color: Theme.accent
            }
            Accessible.role: Accessible.ProgressBar
            Accessible.name: qsTr("%1 percent written").arg(Math.round(share * 100))
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
    Accessible.name: heading
}
