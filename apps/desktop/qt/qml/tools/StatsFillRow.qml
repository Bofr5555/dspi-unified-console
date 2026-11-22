import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP9 -- One buffer's fill (the Mac's BufferFillRow): a dot in the
// series' colour, the title, the watermarks and the fill now, over a trace of
// the last 15 seconds with a band between the lowest and highest fill.
Rectangle {
    id: fillRow
    property var entry: ({})
    property color tint: Theme.accent
    property color valueColor: Theme.text
    property string shownName: entry.text || ""

    implicitHeight: 48
    radius: 5
    color: Qt.alpha(Theme.textMuted, 0.07)
    clip: true

    Accessible.role: Accessible.StaticText
    Accessible.name: (entry.title || "") + " " + (entry.text || "")

    RowLayout {
        id: header
        anchors { left: parent.left; right: parent.right; top: parent.top }
        anchors { leftMargin: 6; rightMargin: 6; topMargin: 3 }
        spacing: 6
        Rectangle {
            width: 7
            height: 7
            radius: 3.5
            color: fillRow.entry.hollow ? "transparent" : fillRow.tint
            border.width: 1.5
            border.color: fillRow.tint
        }
        Text {
            textFormat: Text.PlainText
            text: fillRow.entry.title || ""
            font: Qt.font({ pixelSize: Theme.fontCaption.pixelSize, weight: Font.Medium })
            color: Theme.text
            Layout.fillWidth: true
            elide: Text.ElideRight
        }
        Text {
            text: fillRow.entry.range || ""
            font: Qt.font({ family: Theme.fontMono.family, pixelSize: 9 })
            color: Theme.textMuted
            HoverHandler { id: rangeHover }
            HelpTip {
                text: qsTr("Lowest and highest fill since the watermarks were reset")
                visible: rangeHover.hovered
            }
        }
        Text {
            Layout.preferredWidth: 36
            horizontalAlignment: Text.AlignRight
            text: fillRow.entry.text || ""
            font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontSmall.pixelSize, weight: Font.Bold })
            color: fillRow.valueColor
        }
    }

    Canvas {
        id: trace
        anchors.fill: parent
        property var history: fillRow.entry.history || []
        property color ink: fillRow.tint
        property color guide: Theme.separator
        onHistoryChanged: requestPaint()
        onInkChanged: requestPaint()
        onPaint: {
            const ctx = getContext("2d")
            ctx.reset()
            const top = 17, bottom = height - 2
            const yOf = pct => bottom - Math.max(0, Math.min(100, pct)) / 100 * (bottom - top)
            // The watermarks' band
            const lo = fillRow.entry.min || 0, hi = fillRow.entry.max || 0
            ctx.fillStyle = Qt.alpha(ink, 0.14)
            const bandTop = yOf(hi)
            ctx.fillRect(0, bandTop, width, Math.max(1, yOf(lo) - bandTop))
            // The 50 % guide
            ctx.strokeStyle = guide
            ctx.lineWidth = 0.5
            ctx.setLineDash([2, 3])
            ctx.beginPath()
            ctx.moveTo(0, yOf(50))
            ctx.lineTo(width, yOf(50))
            ctx.stroke()
            // The trace, oldest at the left; a missing sample lifts the pen
            ctx.strokeStyle = ink
            ctx.lineWidth = 1.25
            ctx.lineJoin = "round"
            ctx.lineCap = "round"
            ctx.setLineDash(fillRow.entry.dashed ? [3, 2] : [])
            const n = history.length
            if (n < 2) return
            const step = width / (n - 1)
            let pen = false
            ctx.beginPath()
            for (let i = 0; i < n; ++i) {
                const v = history[i]
                if (v === undefined || v === null || isNaN(v)) { pen = false; continue }
                const x = i * step, y = yOf(v)
                if (pen) ctx.lineTo(x, y)
                else ctx.moveTo(x, y)
                pen = true
            }
            ctx.stroke()
        }
    }

    HoverHandler { id: rowHover }
    HelpTip {
        text: qsTr("%1 fill level over the last 15 seconds").arg(fillRow.entry.title || "")
        visible: rowHover.hovered && !rangeHover.hovered
    }
}
