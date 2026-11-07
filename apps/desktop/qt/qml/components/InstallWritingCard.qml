import QtQuick 2.15

// The firmware write in progress: how far along, which board, which
// firmware, and that the drive vanishing at the end is the normal ending.
Rectangle {
    id: card
    property real progress: 0       // below 0: how far along isn't known
    property string boardName: ""
    property string version: ""
    // -- WP QD32 -- The wizard's come from the core
    property string title: "Writing firmware"
    property string note: "Near the end the board restarts itself and its drive disappears. That is normal - do not unplug it."

    radius: 8
    color: isMacOS ? MacColors.opacity(MacColors.controlBackground, 0.6) : Qt.rgba(1, 1, 1, 0.045)
    border.color: isMacOS ? MacColors.opacity(MacColors.gray, 0.2) : Qt.rgba(1, 1, 1, 0.07)

    Column {
        x: 14
        y: 14
        width: parent.width - 28
        spacing: 10
        Item {
            width: parent.width
            height: 18
            Text { text: card.title; font.pixelSize: 13; font.weight: Font.DemiBold; color: isMacOS ? MacColors.label : "white" }
            Text {
                anchors.right: parent.right
                visible: card.progress >= 0
                text: Math.round(card.progress * 100) + "%"
                font.pixelSize: 12
                font.weight: Font.Medium
                color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.55)
            }
        }
        Rectangle {
            width: parent.width
            height: 4
            radius: 2
            color: isMacOS ? MacColors.quaternaryLabel : Qt.rgba(1, 1, 1, 0.14)
            clip: true
            Rectangle {
                id: bar
                width: card.progress >= 0 ? parent.width * card.progress : parent.width / 3
                height: parent.height
                radius: 2
                color: isMacOS ? MacColors.accent : "#0a7cff"
                NumberAnimation on x {
                    running: card.progress < 0 && card.visible
                    loops: Animation.Infinite
                    from: -bar.width; to: card.width
                    duration: 1200
                }
                x: 0
            }
        }
        Column {
            spacing: 4
            LabelValueRow { visible: card.boardName !== ""; label: "Board"; value: card.boardName }
            LabelValueRow { label: "Firmware"; value: card.version }
        }
    }
    Row {
        x: 14
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 12
        spacing: 6
        Icon { name: "info"; size: 13; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45) }
        Text {
            width: card.width - 28 - 19
            wrapMode: Text.WordWrap
            text: card.note
            font.pixelSize: 11
            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
        }
    }
}
