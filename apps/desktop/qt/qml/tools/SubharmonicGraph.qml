import QtQuick
import UConsole

// -- WP QP8 -- The subharmonic synthesizer's band graph (the Mac's
// SubharmBandView), on a real dB scale from 16 to 250 Hz: the three program
// bands the dividers listen to, the subs they make an octave below, the LF
// boost bell, the ceiling the sub is held under. `graph` is
// ToolsController.subharmGraph(); its points are 0..1 across the plot, which
// sits between the annotation strip (the brackets and the octave arrows) and
// the frequency labels.
// -- WP DL1 -- In Modern the plot sits in a GraphFrame (DESIGN-LANGUAGE §12):
// no outline, the panel tone, the frequency and level labels the frame's
// (fontCaption, whole, inside the plot), "Disabled" its empty label; the
// annotations in the plot a pixel larger. Classic keeps its box and labels.
Item {
    id: frame
    property var graph: ({})
    property bool active: true
    // The three subs' colours
    readonly property var blockColors: [Theme.accent, Theme.warning, Theme.danger]
    readonly property real annotation: 38
    readonly property bool modern: LayoutController.modern   // -- WP DL1 --
    // The strip under the plot for the frequency labels: the frame's are taller
    readonly property real axis: modern ? 22 : 13
    // A label's size: a pixel more in Modern
    function px(size) { return modern ? size + 1 : size }

    onGraphChanged: canvas.requestPaint()
    onActiveChanged: canvas.requestPaint()
    onModernChanged: canvas.requestPaint()
    // The colours are the theme's: a scheme change repaints
    readonly property bool dark: Theme.dark
    onDarkChanged: canvas.requestPaint()

    // -- WP DL1 -- Modern: the frame, its ticks where the canvas puts them
    GraphFrame {
        id: graphFrame
        objectName: frame.objectName.length ? frame.objectName + ".frame" : ""
        visible: frame.modern
        anchors.fill: parent
        grid: false
        empty: !frame.active
        emptyText: qsTr("Disabled")
        readonly property var g: frame.graph || ({})
        xTicks: g.gridX ? g.gridX.map((x, i) => ({ at: x, text: String(g.freqs[i]) })) : []
        yTicks: g.gridY && canvas.height > 0
                ? g.gridY.map((y, i) => ({ at: canvas.yAt(y) / canvas.height, text: g.gridDb[i] > 0 ? "+" + g.gridDb[i] : String(g.gridDb[i]) }))
                : []
        Item {
            id: modernRoom
            anchors.fill: parent
        }
    }

    // Classic: the boxed plot
    Rectangle {
        id: box
        visible: !frame.modern
        anchors.fill: parent
        radius: 8
        color: Theme.control
        border.width: 1
        border.color: Theme.separator
        Item {
            id: classicRoom
            anchors.fill: parent
            anchors.margins: 8
        }
    }

    Canvas {
        id: canvas
        objectName: frame.objectName.length ? frame.objectName + ".canvas" : ""
        parent: frame.modern ? modernRoom : classicRoom
        anchors.fill: parent
        onWidthChanged: requestPaint()
        onHeightChanged: requestPaint()

        function yAt(t) { return frame.annotation + t * (height - frame.annotation - frame.axis) }
        function label(ctx, text, x, y, color, size, bold, backed) {
            size = frame.px(size)
            ctx.font = (bold ? "bold " : "") + size + "px \"" + (frame.modern ? Theme.fontCaption.family : Theme.fontMono.family) + "\""
            const w = ctx.measureText(text).width
            if (backed) {
                ctx.globalAlpha = 0.85
                ctx.fillStyle = frame.modern ? Theme.panel : Theme.control
                ctx.fillRect(x - w / 2 - 2, y - size / 2 - 1, w + 4, size + 2)
            }
            ctx.globalAlpha = 1
            ctx.fillStyle = color
            ctx.textAlign = "center"
            ctx.textBaseline = "middle"
            ctx.fillText(text, x, y)
        }
        function line(ctx, x0, y0, x1, y1) {
            ctx.beginPath()
            ctx.moveTo(x0, y0)
            ctx.lineTo(x1, y1)
            ctx.stroke()
        }

        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            const g = frame.graph
            if (!g || !g.gridX) return
            const w = width, h = height
            const base = h - frame.axis
            ctx.lineCap = "round"
            ctx.lineJoin = "round"

            // The grid: frequency verticals, quiet level lines, the 0 dB reference
            ctx.strokeStyle = Theme.graphGrid
            ctx.lineWidth = 0.5
            ctx.globalAlpha = 0.6
            for (let i = 0; i < g.gridX.length; ++i) line(ctx, g.gridX[i] * w, frame.annotation, g.gridX[i] * w, base)
            for (let i = 0; i < g.gridY.length; ++i)
                if (g.gridDb[i] !== 0) line(ctx, 0, yAt(g.gridY[i]), w, yAt(g.gridY[i]))
            line(ctx, 0, base, w, base)
            ctx.globalAlpha = 1
            line(ctx, 0, yAt(g.zeroY), w, yAt(g.zeroY))
            // Level labels down the left edge, where nothing is drawn below 24 Hz
            // (-- WP DL1 -- Modern: the frame's)
            for (let i = 0; i < (frame.modern ? 0 : g.gridDb.length); ++i) {
                const db = g.gridDb[i]
                label(ctx, db > 0 ? "+" + db : String(db), 9, yAt(g.gridY[i]) - 5, Theme.textMuted, 7, false, false)
            }

            if (!frame.active) {
                if (!frame.modern) label(ctx, qsTr("Disabled"), w / 2, (frame.annotation + base) / 2, Theme.textMuted, 11, true, false)
            } else {
                // The program bands, behind the subs, with a bracket and their range above
                for (let i = 0; i < g.blocks.length; ++i) {
                    const b = g.blocks[i]
                    const x0 = b.srcX0 * w, x1 = b.srcX1 * w
                    ctx.globalAlpha = b.off ? 0.04 : 0.10
                    ctx.fillStyle = Theme.text
                    ctx.fillRect(x0, frame.annotation, x1 - x0, base - frame.annotation)
                    ctx.globalAlpha = b.off ? 0.25 : 0.5
                    ctx.strokeStyle = Theme.textMuted
                    ctx.lineWidth = 1
                    ctx.beginPath()
                    ctx.moveTo(x0 + 1, 9)
                    ctx.lineTo(x0 + 1, 5)
                    ctx.lineTo(x1 - 1, 5)
                    ctx.lineTo(x1 - 1, 9)
                    ctx.stroke()
                    ctx.globalAlpha = 1
                    label(ctx, b.source, (x0 + x1) / 2, 5, b.off ? Theme.textDisabled : Theme.textMuted, 7, true, true)
                }

                // The LF boost over the dry path, only while it does something
                if (g.bell.length > 0) {
                    ctx.strokeStyle = Theme.success
                    ctx.lineWidth = 1.4
                    ctx.globalAlpha = 0.8
                    ctx.beginPath()
                    for (let i = 0; i < g.bell.length; i += 2) {
                        if (i === 0) ctx.moveTo(g.bell[i] * w, yAt(g.bell[i + 1]))
                        else ctx.lineTo(g.bell[i] * w, yAt(g.bell[i + 1]))
                    }
                    ctx.stroke()
                    ctx.globalAlpha = 1
                    label(ctx, qsTr("LF boost"), g.bellX * w, Math.max(frame.annotation + 5, yAt(g.bellLabelY) - 8),
                          Theme.success, 7, true, true)
                }

                // The subs, their tops following the bell, and the octave hop to each
                for (let i = 0; i < g.blocks.length; ++i) {
                    const b = g.blocks[i]
                    if (b.off) continue
                    const color = frame.blockColors[i]
                    const x0 = b.x0 * w, x1 = b.x1 * w
                    ctx.beginPath()
                    ctx.moveTo(x0, base)
                    for (let k = 0; k < b.top.length; k += 2) ctx.lineTo(b.top[k] * w, yAt(b.top[k + 1]))
                    ctx.lineTo(x1, base)
                    ctx.closePath()
                    ctx.globalAlpha = 0.4
                    ctx.fillStyle = color
                    ctx.fill()
                    ctx.globalAlpha = 0.9
                    ctx.strokeStyle = color
                    ctx.lineWidth = 1
                    ctx.stroke()
                    const midTop = yAt(b.midY)
                    const inside = base - midTop > 18
                    label(ctx, b.label, (x0 + x1) / 2, inside ? midTop + 8 : midTop - 6,
                          inside ? Theme.textOnAccent : Theme.text, 7, true, false)

                    const ax0 = b.fromX * w, ax1 = b.toX * w, ay = b.arrowRow
                    ctx.globalAlpha = 0.3
                    ctx.strokeStyle = Theme.text
                    ctx.setLineDash([2, 2])
                    line(ctx, ax0, ay, ax1 + 3, ay)
                    ctx.setLineDash([])
                    ctx.globalAlpha = 0.45
                    ctx.beginPath()
                    ctx.moveTo(ax1 + 4, ay - 3)
                    ctx.lineTo(ax1, ay)
                    ctx.lineTo(ax1 + 4, ay + 3)
                    ctx.stroke()
                    ctx.globalAlpha = 1
                    label(ctx, "÷2", (ax0 + ax1) / 2, ay, Theme.textMuted, 6, true, true)
                }

                // Where the ceiling starts holding the sub down, across the sub range only
                if (g.ceilingOn) {
                    const y = yAt(g.ceilingY), x1 = g.ceilingX * w
                    ctx.strokeStyle = Theme.danger
                    ctx.lineWidth = 1
                    ctx.globalAlpha = 0.65
                    ctx.setLineDash([3, 2])
                    line(ctx, 0, y, x1, y)
                    ctx.setLineDash([])
                    ctx.globalAlpha = 1
                    label(ctx, qsTr("ceiling"), x1 + 18, y, Theme.dangerText, 7, true, true)
                }
                if (g.allOff)
                    label(ctx, qsTr("All bands off"), w / 2, (frame.annotation + base) / 2, Theme.textMuted, 10, true, false)
            }

            for (let i = 0; i < (frame.modern ? 0 : g.gridX.length); ++i)
                label(ctx, String(g.freqs[i]), g.gridX[i] * w, h - 5, Theme.textMuted, 7, false, false)
        }
    }

    Accessible.role: Accessible.Graphic
    Accessible.name: qsTr("Subharmonic bands")
}
