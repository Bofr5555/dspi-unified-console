import QtQuick
import UConsole

// A level meter on a dBFS scale (the Pro layer's meter, docs/UI-MODES.md):
// horizontal or vertical, in three bands (Theme.meterLow, meterMid,
// meterHigh) or in one neutral colour, with the hold line and the clip latch
// of the core's ballistics (Poller.peakHold, Poller.clipHold), and a gain
// reduction variant that grows from the far end.
//
//     LevelMeter { channel: 9 }                         // follows the meter poll
//     LevelMeter { level: v; hold: h; clipped: c }     // a caller's own values
//     LevelMeter { reduction: true; reductionDb: 4.5 } // a limiter's gain reduction
Item {
    id: meter
    // A wire channel to follow, or -1 for the values below
    property int channel: -1
    // Linear peak and held peak, 0..1
    property real level: channel >= 0 && channel < Poller.peaks.length ? Poller.peaks[channel] : 0
    property real hold: channel >= 0 && channel < Poller.peakHold.length ? Poller.peakHold[channel] : 0
    property bool clipped: channel >= 0 && channel < 32 && ((Poller.clipHold >>> channel) & 1) !== 0
    property int orientation: Qt.Horizontal
    // Three bands, or one neutral colour (the classic screens)
    property bool segmented: true
    property bool showHold: true
    // The gain reduction variant: reductionDb (0 or more) over reductionRange
    property bool reduction: false
    property real reductionDb: 0
    property real reductionRange: 12
    property string accessibleName: qsTr("Level")

    readonly property bool vertical: orientation === Qt.Vertical
    readonly property real span: vertical ? height : width
    readonly property real thickness: vertical ? width : height
    readonly property real clipZone: clipped && !reduction ? 3 : 0

    function dbOf(linear) { return linear > 0 ? 20 * Math.log10(linear) : -Infinity }
    // Where a dBFS value sits along the scale, 0..1
    function fraction(db) {
        if (!isFinite(db)) return 0
        return Math.max(0, Math.min(1, (db - Theme.meterFloorDb) / -Theme.meterFloorDb))
    }
    readonly property real levelDb: dbOf(level)
    readonly property real filled: reduction ? Math.max(0, Math.min(1, reductionDb / reductionRange)) : fraction(levelDb)
    readonly property real held: fraction(dbOf(hold))

    implicitWidth: vertical ? 8 : 120
    implicitHeight: vertical ? 120 : 8

    Rectangle {
        anchors.fill: parent
        radius: 2
        color: Theme.meterTrack
    }

    // The fill: a window onto the full-length bands, so a band keeps its place
    Item {
        id: fill
        readonly property real length: Math.max(0, meter.span - meter.clipZone) * meter.filled
        clip: true
        // Level grows from the start (left or bottom); reduction from the far end
        x: meter.vertical ? 0 : (meter.reduction ? meter.span - length : 0)
        y: meter.vertical ? (meter.reduction ? 0 : meter.span - length) : 0
        width: meter.vertical ? meter.thickness : length
        height: meter.vertical ? length : meter.thickness

        Repeater {
            model: meter.reduction ? [{ from: 0, to: 1, color: Theme.meterMid }]
                 : !meter.segmented ? [{ from: 0, to: 1, color: Theme.meterLevel }]
                 : [{ from: 0, to: meter.fraction(Theme.meterMidDb), color: Theme.meterLow },
                    { from: meter.fraction(Theme.meterMidDb), to: meter.fraction(Theme.meterHighDb), color: Theme.meterMid },
                    { from: meter.fraction(Theme.meterHighDb), to: 1, color: Theme.meterHigh }]
            Rectangle {
                required property var modelData
                readonly property real full: meter.span - meter.clipZone
                readonly property real a: modelData.from * full
                readonly property real b: modelData.to * full
                color: modelData.color
                // Positions along the meter, in the fill's coordinates
                x: meter.vertical ? 0 : a - fill.x
                y: meter.vertical ? (meter.span - b) - fill.y : 0
                width: meter.vertical ? meter.thickness : b - a
                height: meter.vertical ? b - a : meter.thickness
            }
        }
    }

    // The hold line
    Rectangle {
        objectName: "LevelMeter.hold"
        visible: meter.showHold && !meter.reduction && meter.held > 0
        readonly property real at: Math.max(0, meter.span - meter.clipZone - 2) * meter.held
        color: Theme.meterHold
        x: meter.vertical ? 0 : at
        y: meter.vertical ? meter.span - at - 2 : 0
        width: meter.vertical ? meter.thickness : 2
        height: meter.vertical ? 2 : meter.thickness
    }

    // The clip latch at the far end, held as long as the core holds it
    Rectangle {
        objectName: "LevelMeter.clip"
        visible: meter.clipZone > 0
        color: Theme.meterHigh
        x: meter.vertical ? 0 : meter.span - 3
        y: 0
        width: meter.vertical ? meter.thickness : 3
        height: meter.vertical ? 3 : meter.thickness
    }

    Accessible.role: Accessible.ProgressBar
    Accessible.name: accessibleName
    Accessible.description: reduction
        //: A limiter's gain reduction; %1 is a number of dB
        ? qsTr("%1 dB reduction").arg(reductionDb.toFixed(1))
        : (isFinite(levelDb) ? qsTr("%1 dBFS").arg(levelDb.toFixed(1)) : qsTr("No signal"))
          + (clipped ? ", " + qsTr("clipped") : "")
}
