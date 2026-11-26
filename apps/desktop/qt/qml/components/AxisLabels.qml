import QtQuick
import UConsole

// -- WP DL0 -- A graph's axis labels drawn inside the plot, every one whole
// (DESIGN-LANGUAGE §12): the labels at an edge move in by `inset` instead of
// being cut ("+25", "-25", "20k"). Lay it over the plot (anchors.fill); it
// takes no input. GraphFrame and Modern's GraphPlot use it.
//
// Ticks are lists of { at, text }: `at` from 0 to 1 along the axis.
// - xTicks: along the bottom, at 0 the left edge;
// - yTicks: at the left, at 0 the top edge;
// - rightTicks: at the right (a second scale: phase), at 0 the top edge.
// The left and right labels stay above the bottom row when it has labels.
// Along the bottom a label that would then overlap its neighbour is left out;
// the labels at the two ends are kept. The side axes thin evenly instead
// (placeEven: on a short graph +20, +10, 0, -10, -20).
//
//     AxisLabels {
//         anchors.fill: parent
//         xTicks: [{ at: 0, text: "20" }, { at: 1, text: "20k" }]
//         yTicks: [{ at: 0, text: "+12" }, { at: 0.5, text: "0" }, { at: 1, text: "-12" }]
//     }
Item {
    id: axes
    property var xTicks: []
    property var yTicks: []
    property var rightTicks: []
    property int inset: 6
    property font font: Theme.fontCaption
    property color color: Theme.textMuted
    property color rightColor: color

    readonly property real rowHeight: metrics.height
    // The room the side labels keep to: under the top inset, above the bottom row
    readonly property real sideTop: inset
    readonly property real sideBottom: height - inset - (xTicks.length > 0 ? rowHeight + 2 : 0)

    function clamp(v, lo, hi) { return Math.max(lo, Math.min(v, Math.max(lo, hi))) }
    // The ticks placed along an axis of `length` between lo and hi (a label's
    // start), each `size(text)` long, without overlaps: [{ text, pos }]
    function place(ticks, length, lo, hi, size) {
        const all = (ticks || []).map(t => ({ text: String(t.text), extent: size(String(t.text)) }))
            .map((t, i) => Object.assign(t, { pos: clamp(ticks[i].at * length - t.extent / 2, lo, hi - t.extent) }))
            .sort((a, b) => a.pos - b.pos)
        const kept = []
        all.forEach((t, i) => {
            const last = kept.length ? kept[kept.length - 1] : null
            if (!last || t.pos >= last.pos + last.extent + 2) { kept.push(t); return }
            // The end label wins over an inner one
            if (i === all.length - 1 && kept.length > 1) kept[kept.length - 1] = t
        })
        return kept
    }
    // -- WP SHOWFIX -- A side axis keeps an even set when not every label
    // fits: every 2nd (3rd, …) tick counted from 0, so +25 … -25 by 5 becomes
    // +20 … -20 by 10 instead of losing one label next to the end.
    function placeEven(ticks, length, lo, hi, size) {
        const all = ticks || []
        const zero = Math.max(0, all.findIndex(t => parseFloat(t.text) === 0))
        for (let n = 1; n < all.length; ++n) {
            const kept = all.filter((t, i) => (i - zero) % n === 0)
            const placed = place(kept, length, lo, hi, size)
            if (placed.length === kept.length) return placed
        }
        return place(all, length, lo, hi, size)
    }
    readonly property var placedX: place(xTicks, width, inset, width - inset, t => metrics.advanceWidth(t))
    readonly property var placedY: placeEven(yTicks, height, sideTop, sideBottom, t => rowHeight)
    readonly property var placedRight: placeEven(rightTicks, height, sideTop, sideBottom, t => rowHeight)
    // The room the labels take from each edge (0 without labels there): a grid
    // drawn under them stops short of it
    readonly property real bottomExtent: placedX.length ? inset + rowHeight + 2 : 0
    readonly property real leftExtent: placedY.length ? inset + Math.max(...placedY.map(t => metrics.advanceWidth(t.text))) + 4 : 0
    readonly property real rightExtent: placedRight.length ? inset + Math.max(...placedRight.map(t => metrics.advanceWidth(t.text))) + 4 : 0

    FontMetrics { id: metrics; font: axes.font }

    Repeater {
        model: axes.placedX
        Text {
            objectName: axes.objectName.length ? axes.objectName + ".x." + index : ""
            required property var modelData
            required property int index
            textFormat: Text.PlainText
            text: modelData.text
            font: axes.font
            color: axes.color
            x: modelData.pos
            y: axes.height - axes.inset - axes.rowHeight
        }
    }
    component SideLabel: Text {
        required property var modelData
        required property int index
        textFormat: Text.PlainText
        text: modelData.text
        font: axes.font
        y: modelData.pos
    }
    Repeater {
        model: axes.placedY
        SideLabel {
            objectName: axes.objectName.length ? axes.objectName + ".y." + index : ""
            color: axes.color
            x: axes.inset
        }
    }
    Repeater {
        model: axes.placedRight
        SideLabel {
            objectName: axes.objectName.length ? axes.objectName + ".right." + index : ""
            color: axes.rightColor
            x: axes.width - axes.inset - implicitWidth
        }
    }

    Accessible.ignored: true
}
