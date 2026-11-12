import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- An effect window's small graph (the Mac's CrossfeedCurveView,
// CompensationCurveView and PsybassSpectrumView): a rounded panel, the grid
// at 100 Hz, 1 kHz and 10 kHz and at the dB steps of `gridY`, the curves, a
// legend or a badge in the top right corner, and "Disabled" while the effect
// is off. Curves are [{points: [x, y, ...] (0..1, y from the bottom), color}].
// Items added as children are drawn over the grid (the psybass bands).
Rectangle {
    id: graph
    property string title
    property var curves: []
    property var gridDb: []
    property var gridY: []
    property bool active: true
    // [{color, label}], or a badge text
    property var legend: []
    property string badge
    // Leave room under the plot for the frequency labels (the psybass sketch)
    property int bottomInset: 0
    default property alias overlay: plot.data
    readonly property alias plotWidth: plot.width
    readonly property alias plotHeight: plot.height

    radius: Theme.radius
    color: Theme.panel
    border.width: 1
    border.color: Theme.separator
    implicitHeight: 160
    clip: true

    readonly property var freqMarks: [
        { hz: 100, text: "100" },
        { hz: 1000, text: "1k" },
        { hz: 10000, text: "10k" }
    ]

    Item {
        id: plot
        anchors.fill: parent
        anchors.margins: 8
        anchors.bottomMargin: 8 + graph.bottomInset

        // Grid
        Repeater {
            model: graph.gridY
            Rectangle {
                required property real modelData
                width: plot.width
                height: 1
                y: plot.height - modelData * plot.height
                color: Theme.graphGrid
                opacity: 0.6
            }
        }
        Repeater {
            model: graph.freqMarks
            Rectangle {
                required property var modelData
                x: EffectsController.freqX(modelData.hz) * plot.width
                width: 1
                height: plot.height
                color: Theme.graphGrid
                opacity: 0.6
            }
        }
        // dB labels on the left
        Repeater {
            model: graph.gridDb.length
            Text {
                required property int index
                x: 2
                y: plot.height - graph.gridY[index] * plot.height - height / 2
                text: String(graph.gridDb[index])
                font.family: Theme.fontMono.family
                font.pixelSize: 9
                color: Theme.textMuted
            }
        }
        // Curves (a Canvas: the software renderer draws a Shape outside its clip)
        Canvas {
            id: curves
            anchors.fill: parent
            renderStrategy: Canvas.Cooperative
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                if (!graph.active) return
                ctx.lineWidth = 2
                ctx.lineCap = "round"
                ctx.lineJoin = "round"
                for (const c of graph.curves) {
                    const pts = c.points || []
                    if (pts.length < 4) continue
                    ctx.strokeStyle = c.color
                    ctx.beginPath()
                    ctx.moveTo(pts[0] * width, height - pts[1] * height)
                    for (let i = 2; i + 1 < pts.length; i += 2) ctx.lineTo(pts[i] * width, height - pts[i + 1] * height)
                    ctx.stroke()
                }
            }
            Connections {
                target: graph
                function onCurvesChanged() { curves.requestPaint() }
                function onActiveChanged() { curves.requestPaint() }
            }
            onWidthChanged: requestPaint()
            onHeightChanged: requestPaint()
        }
    }
    // Frequency labels along the bottom
    Repeater {
        model: graph.freqMarks
        Text {
            required property var modelData
            x: plot.x + EffectsController.freqX(modelData.hz) * plot.width - width / 2
            y: graph.height - height - 1
            text: modelData.text
            font.family: Theme.fontMono.family
            font.pixelSize: 9
            color: Theme.textMuted
        }
    }
    // Legend or badge
    Rectangle {
        visible: graph.active && (graph.legend.length > 0 || graph.badge.length > 0)
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 8
        width: key.implicitWidth + 12
        height: key.implicitHeight + 6
        radius: Theme.radiusSmall
        color: graph.badge.length ? Theme.accent : Theme.window
        opacity: 0.9
        Column {
            id: key
            anchors.centerIn: parent
            spacing: 2
            Text {
                visible: graph.badge.length > 0
                text: graph.badge
                font.pixelSize: 9
                font.weight: Font.DemiBold
                color: Theme.textOnAccent
            }
            Repeater {
                model: graph.badge.length ? [] : graph.legend
                Row {
                    required property var modelData
                    spacing: 4
                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 10
                        height: modelData.bar ? 6 : 2
                        radius: 1
                        color: modelData.color
                    }
                    Text {
                        text: modelData.label
                        font.pixelSize: 9
                        font.weight: Font.DemiBold
                        color: Theme.text
                    }
                }
            }
        }
    }
    Text {
        visible: !graph.active
        anchors.centerIn: parent
        text: qsTr("Disabled")
        font: Theme.fontSmall
        color: Theme.textMuted
    }

    Accessible.role: Accessible.Chart
    Accessible.name: title
}
