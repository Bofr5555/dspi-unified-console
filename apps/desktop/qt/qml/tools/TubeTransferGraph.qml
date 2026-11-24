import QtQuick
import UConsole

// -- WP QP8 -- The tube's transfer curve (the Mac's TubeTransferView): output
// against input over one full-scale swing, the straight line a clean stage
// would draw, and the inputs the stage clips shaded. `graph` is
// ToolsController.tubeGraph(); x and y are 0..1 across the plot. The static
// curve only: sag, the DC blocker and the output stage have memory.
// -- WP DL1 -- In Modern the plot sits in a GraphFrame (DESIGN-LANGUAGE §12):
// no outline, the panel tone, "Disabled" the frame's empty label, and the
// axis names in fontCaption kept whole inside the plot. Classic keeps its box
// and 7 px labels.
Item {
    id: frame
    property var graph: ({})
    property bool active: true
    readonly property bool modern: LayoutController.modern   // -- WP DL1 --

    onGraphChanged: canvas.requestPaint()
    onActiveChanged: canvas.requestPaint()
    onModernChanged: canvas.requestPaint()
    // The colours are the theme's: a scheme change repaints
    readonly property bool dark: Theme.dark
    onDarkChanged: canvas.requestPaint()

    // -- WP DL1 -- Modern: the frame; the canvas draws its own grid
    GraphFrame {
        objectName: frame.objectName.length ? frame.objectName + ".frame" : ""
        visible: frame.modern
        anchors.fill: parent
        grid: false
        empty: !frame.active
        emptyText: qsTr("Disabled")
        Item {
            id: modernRoom
            anchors.fill: parent
        }
    }

    // Classic: the boxed plot
    Rectangle {
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

        function line(ctx, x0, y0, x1, y1) {
            ctx.beginPath()
            ctx.moveTo(x0, y0)
            ctx.lineTo(x1, y1)
            ctx.stroke()
        }
        function label(ctx, text, x, y, color, bold) {
            ctx.font = (bold ? "bold " : "") + "7px \"" + Theme.fontMono.family + "\""
            ctx.fillStyle = color
            ctx.textAlign = "center"
            ctx.textBaseline = "middle"
            ctx.fillText(text, x, y)
        }
        // -- WP DL1 -- Modern: an axis name in fontCaption, whole: `align`
        // "left" or "right" puts it 6 px inside that edge, else centred on x
        function caption(ctx, text, x, y, color, align) {
            const f = Theme.fontCaption
            ctx.font = f.pixelSize + "px \"" + f.family + "\""
            ctx.fillStyle = color
            ctx.textBaseline = "middle"
            const w = ctx.measureText(text).width
            const at = align === "left" ? 6 : align === "right" ? width - 6 - w : Math.max(6, Math.min(x - w / 2, width - 6 - w))
            ctx.textAlign = "left"
            ctx.fillText(text, at, Math.max(6 + f.pixelSize / 2, Math.min(y, height - 6 - f.pixelSize / 2)))
        }

        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            const g = frame.graph
            if (!g || !g.curve) return
            const w = width, h = height
            ctx.lineCap = "round"
            ctx.lineJoin = "round"

            // The grid, full scale out (dashed), and the two axes
            ctx.strokeStyle = Theme.graphGrid
            ctx.lineWidth = 0.5
            ctx.globalAlpha = 0.6
            for (const x of [0, 0.25, 0.75, 1]) line(ctx, x * w, 0, x * w, h)
            for (const y of g.half) line(ctx, 0, y * h, w, y * h)
            ctx.globalAlpha = 0.5
            ctx.strokeStyle = Theme.danger
            ctx.setLineDash([3, 2])
            line(ctx, 0, g.fullScaleTop * h, w, g.fullScaleTop * h)
            line(ctx, 0, g.fullScaleBottom * h, w, g.fullScaleBottom * h)
            ctx.setLineDash([])
            ctx.globalAlpha = 1
            ctx.strokeStyle = Theme.graphGrid
            line(ctx, 0.5 * w, 0, 0.5 * w, h)
            line(ctx, 0, g.zeroY * h, w, g.zeroY * h)

            if (!frame.active) {
                if (frame.modern) return   // the frame's empty label
                ctx.font = "bold 11px \"" + Theme.fontBody.family + "\""
                ctx.fillStyle = Theme.textMuted
                ctx.textAlign = "center"
                ctx.textBaseline = "middle"
                ctx.fillText(qsTr("Disabled"), w / 2, h / 2)
            } else {
                // Where the stage clips: from each knee outwards
                ctx.fillStyle = Theme.warning
                ctx.globalAlpha = 0.12
                if (!isNaN(g.clipPos)) ctx.fillRect(g.clipPos * w, 0, w - g.clipPos * w, h)
                if (!isNaN(g.clipNeg)) ctx.fillRect(0, 0, g.clipNeg * w, h)
                // The clean stage's straight line
                ctx.globalAlpha = 0.3
                ctx.strokeStyle = Theme.text
                ctx.lineWidth = 1
                ctx.setLineDash([3, 3])
                line(ctx, 0, g.lineFrom * h, w, g.lineTo * h)
                ctx.setLineDash([])
                // The curve
                ctx.globalAlpha = 1
                ctx.strokeStyle = Theme.accent
                ctx.lineWidth = 1.6
                ctx.beginPath()
                for (let i = 0; i < g.curve.length; i += 2) {
                    if (i === 0) ctx.moveTo(g.curve[i] * w, g.curve[i + 1] * h)
                    else ctx.lineTo(g.curve[i] * w, g.curve[i + 1] * h)
                }
                ctx.stroke()
            }
            if (frame.modern) {
                caption(ctx, qsTr("in"), w, g.zeroY * h + 9, Theme.textMuted, "right")
                caption(ctx, qsTr("out"), w / 2 + 12, 0, Theme.textMuted, "")
                caption(ctx, qsTr("0 dBFS"), 0, g.fullScaleTop * h - 8, Theme.dangerText, "left")
                return
            }
            label(ctx, qsTr("in"), w - 8, g.zeroY * h + 7, Theme.textMuted, true)
            label(ctx, qsTr("out"), w / 2 + 10, 6, Theme.textMuted, true)
            label(ctx, qsTr("0 dBFS"), 18, g.fullScaleTop * h - 6, Theme.dangerText, false)
        }
    }

    Accessible.role: Accessible.Graphic
    Accessible.name: qsTr("Transfer curve")
}
