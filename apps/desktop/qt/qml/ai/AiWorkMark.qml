import QtQuick
import UConsole

// -- WP AIVIS -- A turning ring: an AI agent's call or job runs (AiController's
// `working`). At rest a dot; under a pause it does not turn.
Item {
    id: mark
    property bool working: false
    property color color: Theme.accentText
    implicitWidth: 14
    implicitHeight: 14
    Accessible.ignored: true

    Rectangle {
        anchors.centerIn: parent
        width: 6
        height: 6
        radius: 3
        color: mark.color
        opacity: mark.working ? 1 : 0.6
    }
    Item {
        id: ring
        anchors.fill: parent
        visible: mark.working
        Repeater {
            model: 8
            delegate: Rectangle {
                required property int index
                readonly property real turn: index * Math.PI / 4
                x: ring.width / 2 + Math.cos(turn) * (ring.width / 2 - 1) - 1
                y: ring.height / 2 + Math.sin(turn) * (ring.height / 2 - 1) - 1
                width: 2
                height: 2
                radius: 1
                color: mark.color
                opacity: (index + 1) / 8
            }
        }
        RotationAnimator on rotation {
            running: mark.working && mark.visible
            from: 0
            to: 360
            duration: 1200
            loops: Animation.Infinite
        }
    }
}
