import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP7 -- An effect window's small graph (the Mac's CrossfeedCurveView,
// CompensationCurveView and PsybassSpectrumView): a rounded panel, the grid
// at 100 Hz, 1 kHz and 10 kHz and at the dB steps of `gridY`, the curves, a
// legend or a badge in the top right corner, and "Disabled" while the effect
// is off. Curves are [{points: [x, y, ...] (0..1, y from the bottom), color}].
// Items added as children are drawn over the grid (the psybass bands).
// -- WP DL1 -- In Modern the graph sits in a GraphFrame (DESIGN-LANGUAGE §12):
// no outline, the plot to the frame's edges, the axis labels the frame's
// (fontCaption, whole, inside the plot; the grid stays the canvas's, under the
// curves, so no line item sits under a label), the legend or the badge as
// the frame's tools at the top right, "Disabled" as its empty label. Classic
// keeps the boxed panel with its own grid and 9 px labels.
Item {
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
    readonly property bool modern: LayoutController.modern   // -- WP DL1 --
    // The canvas's grid follows the scheme and the tone
    readonly property color gridColor: Theme.graphGrid
    onGridColorChanged: curves.requestPaint()

    implicitHeight: 160

    readonly property var freqMarks: [
        { hz: 100, text: "100" },
        { hz: 1000, text: "1k" },
        { hz: 10000, text: "10k" }
    ]

    // -- WP DL1 -- Modern: the frame, its ticks from the marks and the dB steps
    GraphFrame {
        id: frame
        objectName: graph.objectName.length ? graph.objectName + ".frame" : ""
        visible: graph.modern
        anchors.fill: parent
        grid: false
        empty: !graph.active
        emptyText: qsTr("Disabled")
        xTicks: graph.freqMarks.map(m => ({ at: EffectsController.freqX(m.hz), text: m.text }))
        yTicks: graph.gridDb.map((db, i) => ({ at: 1 - graph.gridY[i], text: String(db) }))

        // The plot's room in the frame: all of it, less the psybass sketch's strip
        Item {
            id: modernRoom
            anchors.fill: parent
            anchors.bottomMargin: graph.bottomInset
        }
        tools: [
            Rectangle {
                visible: graph.active && graph.badge.length > 0
                width: badgeText.implicitWidth + 12
                height: badgeText.implicitHeight + 6
                radius: height / 2
                color: Theme.control
                Text {
                    id: badgeText
                    anchors.centerIn: parent
                    textFormat: Text.PlainText
                    text: graph.badge
                    font: Theme.fontCaption
                    color: Theme.textMuted
                }
            },
            Row {
                visible: graph.active && graph.badge.length === 0 && graph.legend.length > 0
                spacing: Theme.spacingSmall
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
                            textFormat: Text.PlainText
                            text: modelData.label
                            font: Theme.fontCaption
                            color: Theme.textMuted
                        }
                    }
                }
            }
        ]
    }

    // Classic: the boxed panel
    Rectangle {
        id: box
        visible: !graph.modern
        anchors.fill: parent
        radius: Theme.radius
        color: Theme.panel
        border.width: 1
        border.color: Theme.separator
        clip: true

        Item {
            id: classicRoom
            anchors.fill: parent
            anchors.margins: 8
            anchors.bottomMargin: 8 + graph.bottomInset
        }

        // Frequency labels along the bottom
        Repeater {
            model: graph.freqMarks
            Text {
                required property var modelData
                x: classicRoom.x + EffectsController.freqX(modelData.hz) * classicRoom.width - width / 2
                y: box.height - height - 1
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
    }

    // The plot: in the frame's room (Modern) or the box's (Classic)
    Item {
        id: plot
        parent: graph.modern ? modernRoom : classicRoom
        anchors.fill: parent

        // Classic's grid and dB labels; Modern's are the frame's
        Repeater {
            model: graph.modern ? [] : graph.gridY
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
            model: graph.modern ? [] : graph.freqMarks
            Rectangle {
                required property var modelData
                x: EffectsController.freqX(modelData.hz) * plot.width
                width: 1
                height: plot.height
                color: Theme.graphGrid
                opacity: 0.6
            }
        }
        Repeater {
            model: graph.modern ? 0 : graph.gridDb.length
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
                // -- WP DL1 -- Modern: the grid at the marks and the dB steps
                if (graph.modern) {
                    ctx.strokeStyle = Theme.graphGrid
                    ctx.lineWidth = 1
                    ctx.beginPath()
                    for (const m of graph.freqMarks) {
                        const x = Math.round(EffectsController.freqX(m.hz) * width) + 0.5
                        ctx.moveTo(x, 0)
                        ctx.lineTo(x, height)
                    }
                    for (const g of graph.gridY) {
                        const y = Math.round(height - g * height) + 0.5
                        if (y <= 1 || y >= height - 1) continue
                        ctx.moveTo(0, y)
                        ctx.lineTo(width, y)
                    }
                    ctx.stroke()
                }
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
                function onModernChanged() { curves.requestPaint() }
                function onGridYChanged() { curves.requestPaint() }
            }
            onWidthChanged: requestPaint()
            onHeightChanged: requestPaint()
        }
    }

    Accessible.role: Accessible.Chart
    Accessible.name: title
}
