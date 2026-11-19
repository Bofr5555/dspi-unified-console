import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP P5QUI -- A measurement's graph, in the effect windows' graph style
// (EffectGraph: a rounded panel, the decade grid, a Canvas, the legend): the
// curves the core computed, 20 Hz to 20 kHz on a log axis, the dB range the
// controller chose. Measured curves are solid (untrusted points faint), the
// target dashed, the predicted curve dotted, a held RTA spectrum thin; a
// spread across positions is a band. Nothing is computed here but the
// mapping to pixels.
//
// `graph` is MeasureController.graph()'s map: { series: [{ kind, label,
// colour, points: [x (0 to 1), dB, trusted (0 or 1), ...], spread? }], yMin,
// yMax, unit }. A click picks a frequency (picked), a drag a range (spanned).
Rectangle {
    id: root
    property string title
    property var graph: ({})
    // A frequency or a range to mark ({ f1, f2 }), or null
    property var mark: null
    property bool interactive: false
    signal picked(real hz)
    signal spanned(real f1, real f2)

    readonly property var series: graph && graph.series ? graph.series : []
    readonly property real yMin: graph && graph.yMin !== undefined ? graph.yMin : -30
    readonly property real yMax: graph && graph.yMax !== undefined ? graph.yMax : 6
    readonly property string unit: graph && graph.unit ? graph.unit : ""
    // What the last paint drew (the smoke driver reads these)
    property int drawnSeries: 0
    property var drawnKinds: []

    function hzOf(x) { return 20 * Math.pow(1000, Math.max(0, Math.min(1, x / plot.width))) }
    function xOfHz(hz) { return Math.log(hz / 20) / Math.log(1000) * plot.width }
    function colourOf(s) {
        if (s.kind === "target") return Theme.text
        if (s.kind === "predicted") return Theme.accentText
        if (s.kind === "held") return Theme.textMuted
        if (s.kind === "compare") return Theme.warning
        return ChannelPalette.outputs[(s.colour || 0) % ChannelPalette.outputs.length]
    }

    radius: Theme.radius
    color: Theme.panel
    border.width: 1
    border.color: Theme.separator
    implicitHeight: 260
    clip: true

    readonly property var freqMarks: [
        { hz: 20, text: "20" }, { hz: 50, text: "50" }, { hz: 100, text: "100" }, { hz: 200, text: "200" },
        { hz: 500, text: "500" }, { hz: 1000, text: "1k" }, { hz: 2000, text: "2k" }, { hz: 5000, text: "5k" },
        { hz: 10000, text: "10k" }, { hz: 20000, text: "20k" }
    ]
    readonly property var dbMarks: {
        const out = []
        const step = (yMax - yMin) > 60 ? 12 : 6
        for (let v = Math.ceil(yMin / step) * step; v <= yMax; v += step) out.push(v)
        return out
    }

    Item {
        id: plot
        anchors.fill: parent
        anchors.leftMargin: 34
        anchors.rightMargin: 10
        anchors.topMargin: 10
        anchors.bottomMargin: 20

        function yOf(db) { return (root.yMax - db) / Math.max(1e-6, root.yMax - root.yMin) * height }

        Repeater {
            model: root.dbMarks
            Rectangle {
                required property real modelData
                width: plot.width
                height: 1
                y: plot.yOf(modelData)
                color: Theme.graphGrid
                opacity: modelData === 0 ? 1.0 : 0.6
            }
        }
        Repeater {
            model: root.freqMarks
            Rectangle {
                required property var modelData
                x: root.xOfHz(modelData.hz)
                width: 1
                height: plot.height
                color: Theme.graphGrid
                opacity: 0.6
            }
        }
        // The marked frequency or range
        Rectangle {
            visible: root.mark !== null && root.mark !== undefined && root.mark.f1 > 0
            x: visible ? root.xOfHz(root.mark.f1) : 0
            width: visible && root.mark.f2 > root.mark.f1 ? Math.max(1, root.xOfHz(root.mark.f2) - x) : 1
            height: plot.height
            color: Theme.accent
            opacity: width > 1 ? 0.15 : 0.8
        }
        Canvas {
            id: canvas
            objectName: root.objectName + ".canvas"
            anchors.fill: parent
            renderStrategy: Canvas.Cooperative
            onPaint: {
                const ctx = getContext("2d")
                ctx.reset()
                ctx.lineCap = "round"
                ctx.lineJoin = "round"
                const w = width, h = height
                const yOf = db => (root.yMax - db) / Math.max(1e-6, root.yMax - root.yMin) * h
                let drawn = 0
                const kinds = []
                for (const s of root.series) {
                    const p = s.points || []
                    if (p.length < 6) continue
                    const colour = root.colourOf(s)
                    // The spread band under the curve
                    const sp = s.spread || []
                    if (sp.length >= 6) {
                        ctx.fillStyle = colour
                        ctx.globalAlpha = 0.15
                        ctx.beginPath()
                        ctx.moveTo(sp[0] * w, yOf(sp[2]))
                        for (let i = 3; i + 2 < sp.length; i += 3) ctx.lineTo(sp[i] * w, yOf(sp[i + 2]))
                        for (let i = sp.length - 3; i >= 0; i -= 3) ctx.lineTo(sp[i] * w, yOf(sp[i + 1]))
                        ctx.closePath()
                        ctx.fill()
                        ctx.globalAlpha = 1
                    }
                    ctx.strokeStyle = colour
                    ctx.lineWidth = s.kind === "held" ? 1 : 2
                    if (s.kind === "target") ctx.setLineDash([6, 4])
                    else if (s.kind === "predicted") ctx.setLineDash([1, 4])
                    else ctx.setLineDash([])
                    // Runs of equal trust: trusted strong, the others faint
                    let i = 0
                    while (i + 2 < p.length) {
                        const trusted = p[i + 2] === 1
                        ctx.globalAlpha = trusted || s.kind !== "measured" ? 1.0 : 0.3
                        ctx.beginPath()
                        ctx.moveTo(p[i] * w, yOf(p[i + 1]))
                        let j = i + 3
                        while (j + 2 < p.length) {
                            ctx.lineTo(p[j] * w, yOf(p[j + 1]))
                            if ((p[j + 2] === 1) !== trusted) break
                            j += 3
                        }
                        ctx.stroke()
                        i = j >= p.length - 2 ? p.length : j
                    }
                    ctx.globalAlpha = 1
                    drawn++
                    kinds.push(s.kind)
                }
                ctx.setLineDash([])
                root.drawnSeries = drawn
                root.drawnKinds = kinds
            }
            Connections {
                target: root
                function onGraphChanged() { canvas.requestPaint() }
            }
            Connections {
                target: Theme
                function onDarkChanged() { canvas.requestPaint() }
            }
            onWidthChanged: requestPaint()
            onHeightChanged: requestPaint()
        }
        MouseArea {
            anchors.fill: parent
            enabled: root.interactive
            property real startX: -1
            onPressed: mouse => startX = mouse.x
            onReleased: mouse => {
                if (startX < 0) return
                const a = Math.min(startX, mouse.x), b = Math.max(startX, mouse.x)
                if (b - a < 6) root.picked(root.hzOf(mouse.x))
                else root.spanned(root.hzOf(a), root.hzOf(b))
                startX = -1
            }
        }
    }
    // dB labels on the left, frequency labels along the bottom
    Repeater {
        model: root.dbMarks
        Text {
            required property real modelData
            x: 4
            y: plot.y + plot.yOf(modelData) - height / 2
            text: String(modelData)
            font.family: Theme.fontMono.family
            font.pixelSize: 9
            color: Theme.textMuted
        }
    }
    Repeater {
        model: root.freqMarks
        Text {
            required property var modelData
            x: plot.x + root.xOfHz(modelData.hz) - width / 2
            y: root.height - height - 3
            text: modelData.text
            font.family: Theme.fontMono.family
            font.pixelSize: 9
            color: Theme.textMuted
        }
    }
    Text {
        x: plot.x + 4
        y: 0
        text: root.unit
        font.pixelSize: 9
        color: Theme.textMuted
    }
    // The legend
    Rectangle {
        visible: root.series.length > 0
        anchors.top: parent.top
        anchors.right: parent.right
        anchors.margins: 12
        width: key.implicitWidth + 12
        height: key.implicitHeight + 6
        radius: Theme.radiusSmall
        color: Theme.window
        opacity: 0.9
        Column {
            id: key
            anchors.centerIn: parent
            spacing: 2
            Repeater {
                model: root.series
                Row {
                    required property var modelData
                    spacing: 4
                    Rectangle {
                        anchors.verticalCenter: parent.verticalCenter
                        width: 12
                        height: 2
                        color: root.colourOf(modelData)
                    }
                    Text {
                        text: modelData.label + (modelData.kind === "target" ? qsTr(" (dashed)") : modelData.kind === "predicted" ? qsTr(" (dotted)") : "")
                        font.pixelSize: 9
                        color: Theme.text
                    }
                }
            }
        }
    }
    Text {
        visible: root.series.length === 0
        anchors.centerIn: parent
        text: qsTr("No curve yet")
        font: Theme.fontSmall
        color: Theme.textMuted
    }

    Accessible.role: Accessible.Chart
    Accessible.name: title
}
