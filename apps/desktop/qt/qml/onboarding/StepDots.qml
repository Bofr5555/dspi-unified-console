import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP17 -- The wizard's progress (the Mac's StepDotStrip): a dot per
// step on a line, the steps done and the current one in the accent colour,
// each with its label under it.
Item {
    id: strip
    property var labels: []
    property int current: 0
    property int dotWidth: 60

    implicitHeight: 34
    implicitWidth: Math.max(1, labels.length) * dotWidth

    // The line between the first and the last dot
    Rectangle {
        x: strip.dotWidth / 2
        y: 5
        width: Math.max(0, strip.width - strip.dotWidth)
        height: 2
        radius: 1
        color: Theme.separator
    }
    Rectangle {
        x: strip.dotWidth / 2
        y: 5
        width: strip.labels.length > 1 ? (strip.width - strip.dotWidth) * strip.current / (strip.labels.length - 1) : 0
        height: 2
        radius: 1
        color: Theme.accent
    }
    Row {
        anchors.fill: parent
        Repeater {
            model: strip.labels
            delegate: Item {
                required property string modelData
                required property int index
                width: strip.labels.length ? strip.width / strip.labels.length : 0
                height: strip.height
                Rectangle {
                    anchors.horizontalCenter: parent.horizontalCenter
                    y: 0
                    width: 12
                    height: 12
                    radius: 6
                    color: index <= strip.current ? Theme.accent : Theme.panel
                    border.width: index <= strip.current ? 0 : 2
                    border.color: Theme.border
                }
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    anchors.bottom: parent.bottom
                    text: modelData
                    font: Theme.fontSmall
                    color: index === strip.current ? Theme.text : Theme.textMuted
                }
            }
        }
    }

    Accessible.role: Accessible.ProgressBar
    Accessible.name: labels.length ? qsTr("Step %1 of %2: %3").arg(current + 1).arg(labels.length).arg(labels[current] || "") : ""
}
