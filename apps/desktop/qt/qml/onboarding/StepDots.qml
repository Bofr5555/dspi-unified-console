import QtQuick
import UConsole

// -- WP QP17 -- The wizard's progress (the Mac's StepDotStrip): a dot per
// step on a track, those reached and the current one in the accent colour,
// each step's name under its dot.
Item {
    id: dots
    property var steps: []
    property int at: 0
    property int slot: 60
    readonly property int count: steps.length
    readonly property real pitch: count > 0 ? width / count : 0

    implicitWidth: slot * Math.max(count, 1)
    implicitHeight: 34

    // The track, then the part already walked
    Rectangle {
        id: track
        x: dots.pitch / 2
        y: 5
        width: dots.pitch * Math.max(dots.count - 1, 0)
        height: 2
        radius: height / 2
        color: Theme.separator
    }
    Rectangle {
        x: track.x
        y: track.y
        height: track.height
        radius: track.radius
        width: dots.count > 1 ? track.width * dots.at / (dots.count - 1) : 0
        color: Theme.accent
    }

    Repeater {
        model: dots.count
        delegate: Column {
            id: step
            required property int index
            readonly property bool reached: index <= dots.at
            x: dots.pitch * index
            width: dots.pitch
            spacing: 8
            Rectangle {
                x: (step.width - width) / 2
                implicitWidth: 12
                implicitHeight: 12
                radius: implicitWidth / 2
                color: step.reached ? Theme.accent : Theme.panel
                border {
                    width: step.reached ? 0 : 2
                    color: Theme.border
                }
            }
            Text {
                width: step.width
                horizontalAlignment: Text.AlignHCenter
                text: dots.steps[step.index] || ""
                font: Theme.fontSmall
                color: step.index === dots.at ? Theme.text : Theme.textMuted
                elide: Text.ElideRight
            }
        }
    }

    Accessible.role: Accessible.ProgressBar
    Accessible.name: count ? qsTr("Step %1 of %2: %3").arg(at + 1).arg(count).arg(steps[at] || "") : ""
}
