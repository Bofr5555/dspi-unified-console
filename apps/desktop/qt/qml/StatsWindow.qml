import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import QtQuick.Window 2.15
import DSPi 1.0
import "components"

// System Statistics, after the macOS Console: device and system figures,
// audio output and PDM over/underrun counters, S/PDIF DMA starvation, live
// buffer fill levels with a 15 s trace, and (when they apply) the S/PDIF
// input, LG Sound Sync, ADAT output and I2S slave clock. Polled only while
// the window is open: counters and ring health every 2 s, buffers every 60 ms.
AppWindow {
    id: win
    title: qsTr("System Statistics")
    visible: false
    width: 960
    height: 780 + titlebarHeight    // -- WP QD29 -- Column 1 with ring health and PDM, unscrolled
    minimumWidth: 900
    minimumHeight: 480 + titlebarHeight

    readonly property bool shown: visible && visibility !== Window.Minimized
    onShownChanged: stats.watching = shown

    readonly property string dash: "-"
    readonly property var info: stats.info
    readonly property var buf: stats.buffers
    readonly property var ring: stats.ringHealth    // -- WP QD29 --
    readonly property bool live: bridge.connected && info.clockMHz !== undefined
    readonly property real columnWidth: (width - 2) / 3
    readonly property var outputTypes: bridge.hardware.outputTypes || []

    // A clock for the "time since" rows, only while they show something
    property real now: Date.now()
    Timer {
        interval: 1000
        repeat: true
        running: win.shown && (win.info.lastEventMs || 0) > 0
        onTriggered: win.now = Date.now()
    }
    function span(ms) {
        if (ms <= 0) return dash
        var s = Math.floor(ms / 1000), h = Math.floor(s / 3600), m = Math.floor((s % 3600) / 60), r = s % 60
        function two(n) { return (n < 10 ? "0" : "") + n }
        return h > 0 ? qsTr("%1h %2m %3s").arg(h).arg(two(m)).arg(two(r)) : qsTr("%1m %2s").arg(m).arg(two(r))
    }
    function num(v) { return v === undefined || v === null ? dash : String(v) }
    // -- WP QD29 -- formatLatencyMs and formatGapUs on the Mac
    function latencyMs(us) { return us % 1000 === 0 ? qsTr("%1 ms").arg(String(us / 1000)) : qsTr("%1 ms").arg((us / 1000).toFixed(2)) }
    function gapText(us) { return us < 10000 ? qsTr("%1 µs").arg(String(us)) : qsTr("%1 ms").arg((us / 1000).toFixed(1)) }

    // Colours: macOS dark system colours
    readonly property color green: isMacOS ? MacColors.green : "#32d74b"
    readonly property color yellow: isMacOS ? MacColors.yellow : "#ffd60a"
    readonly property color orange: isMacOS ? MacColors.orange : "#ff9f0a"
    readonly property color red: isMacOS ? MacColors.red : "#ff453a"
    readonly property color grey: isMacOS ? MacColors.gray : "#8e8e93"

    component SectionLabel: Text {
        font.pixelSize: 11
        font.weight: Font.Bold
        font.letterSpacing: 0.4
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
        topPadding: 4
        bottomPadding: 2
    }
    component Divider: Rectangle { width: parent ? parent.width : 0; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.07) }

    // Title left, value right
    component InfoRow: Item {
        property string title: ""
        property string value: ""
        property color valueColor: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9)
        // macOS: the starvation section's per-output rows have a secondary title,
        // its timer rows a secondary title and value
        property bool secondaryTitle: secondaryValue
        property bool secondaryValue: false
        width: parent ? parent.width : 0
        height: 22
        Text { anchors.verticalCenter: parent.verticalCenter; text: parent.title; font.pixelSize: 11; font.weight: Font.Medium; color: isMacOS ? (parent.secondaryTitle ? MacColors.secondaryLabel : MacColors.label) : Qt.rgba(1, 1, 1, 0.75) }
        Text {
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            text: parent.value
            font.pixelSize: 12
            font.weight: Font.Bold
            color: isMacOS ? (parent.secondaryValue ? MacColors.secondaryLabel : parent.valueColor) : parent.valueColor
        }
    }
    // -- WP QD29 -- An InfoRow that explains itself, with the plain colour for a count at 0
    component RingRow: InfoRow {
        property string tip: ""
        readonly property color plain: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9)
        MouseArea { id: ringHover; anchors.fill: parent; hoverEnabled: true; acceptedButtons: Qt.NoButton }
        ToolTip.visible: ringHover.containsMouse
        ToolTip.delay: 600
        ToolTip.text: tip
    }
    // Title left, a state dot and its name right
    component StateRow: Item {
        property string title: ""
        property string text: ""
        property color dot: win.grey
        width: parent ? parent.width : 0
        height: 22
        Text { anchors.verticalCenter: parent.verticalCenter; text: parent.title; font.pixelSize: 11; font.weight: Font.Medium; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.75) }
        Row {
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: 6
            Rectangle { width: 6; height: 6; radius: 3; color: parent.parent.dot; anchors.verticalCenter: parent.verticalCenter }
            Text { text: parent.parent.text; font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.6) }
        }
    }
    // Over / underrun counters: orange overruns, red underruns when non-zero
    component CounterRow: Item {
        property string title: ""
        property string subtitle: ""
        property var over
        property var under
        property bool hasUnder: true
        width: parent ? parent.width : 0
        height: Math.max(34, counterTitle.height + 8)
        Column {
            id: counterTitle
            anchors.verticalCenter: parent.verticalCenter
            Text { text: parent.parent.title; font.pixelSize: 11; font.weight: Font.Medium; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.85) }
            Text { text: parent.parent.subtitle; font.pixelSize: 10; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45) }
        }
        Row {
            anchors.right: parent.right
            anchors.verticalCenter: parent.verticalCenter
            spacing: 14
            Column {
                Text {
                    anchors.right: parent.right
                    text: win.num(parent.parent.parent.over)
                    font.pixelSize: 14
                    font.weight: Font.Bold
                    color: isMacOS ? ((parent.parent.parent.over || 0) > 0 ? win.orange : MacColors.label) : (parent.parent.parent.over || 0) > 0 ? win.orange : Qt.rgba(1, 1, 1, 0.9)
                }
                Text { anchors.right: parent.right; text: qsTr("over"); font.pixelSize: 9; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45) }
            }
            Column {
                visible: parent.parent.hasUnder
                Text {
                    anchors.right: parent.right
                    text: win.num(parent.parent.parent.under)
                    font.pixelSize: 14
                    font.weight: Font.Bold
                    color: isMacOS ? ((parent.parent.parent.under || 0) > 0 ? win.red : MacColors.label) : (parent.parent.parent.under || 0) > 0 ? win.red : Qt.rgba(1, 1, 1, 0.9)
                }
                Text { anchors.right: parent.right; text: qsTr("under"); font.pixelSize: 9; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45) }
            }
        }
    }
    // A buffer's fill now, its watermarks, and the last 15 s
    component BufferRow: Rectangle {
        id: br
        property string title: ""
        property color tint: "white"
        property bool hollow: false
        property int series: 0
        property var levels: ({})        // {fill, min, max}, and for an output its band from the core
        property int kind: 0             // 0 S/PDIF, 1 PDM DMA, 2 PDM ring
        readonly property int fill: levels.fill !== undefined ? levels.fill : 0
        // -- WP QD29 -- The band is judged by the live latency target; without a band, the fixed ones
        readonly property color fillColor: levels.band === "good" ? win.green : levels.band === "warn" ? win.yellow : levels.band === "bad" ? win.red
                                         : kind === 0 ? (fill === 0 || fill === 100 ? win.red : fill < 25 || fill > 75 ? win.yellow : win.green)
                                         : kind === 1 ? (fill > 50 ? win.red : fill < 5 || fill > 30 ? win.yellow : win.green)
                                         : (fill > 50 ? win.red : fill > 20 ? win.yellow : win.green)
        width: parent ? parent.width : 0
        height: 48
        radius: 5
        color: isMacOS ? MacColors.opacity(MacColors.secondaryLabel, 0.07) : Qt.rgba(1, 1, 1, 0.05)
        Row {
            x: 8
            y: 3
            spacing: 6
            height: 14
            Rectangle {
                width: 7; height: 7; radius: 3.5
                anchors.verticalCenter: parent.verticalCenter
                color: br.hollow ? "transparent" : br.tint
                border.width: 1.5
                border.color: br.tint
            }
            Text { anchors.verticalCenter: parent.verticalCenter; text: br.title; font.pixelSize: 10; font.weight: Font.Medium; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.85) }
            Text {
                anchors.verticalCenter: parent.verticalCenter
                text: br.levels.min !== undefined && br.levels.max >= br.levels.min ? br.levels.min + "–" + br.levels.max + "%" : ""
                font.pixelSize: 9
                color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45)
                MouseArea { id: wmHover; anchors.fill: parent; hoverEnabled: true; acceptedButtons: Qt.NoButton }
                ToolTip.visible: wmHover.containsMouse
                ToolTip.delay: 600
                ToolTip.text: qsTr("Lowest and highest fill since the watermarks were reset")
            }
        }
        Text {
            anchors.right: parent.right
            anchors.rightMargin: 8
            y: 2
            width: 36
            horizontalAlignment: Text.AlignRight
            text: br.fill + "%"
            font.pixelSize: 11
            font.weight: Font.Bold
            color: br.fillColor
        }
        BufferTraceItem {
            x: 4
            y: 17
            width: parent.width - 8
            height: parent.height - 19
            controller: stats
            series: br.series
            color: br.tint
            dashed: br.hollow
            bandMin: br.levels.min !== undefined ? br.levels.min : 100
            bandMax: br.levels.max !== undefined ? br.levels.max : 0
        }
        MouseArea { id: rowHover; anchors.fill: parent; hoverEnabled: true; acceptedButtons: Qt.NoButton; z: -1 }
        ToolTip.visible: rowHover.containsMouse
        ToolTip.delay: 800
        ToolTip.text: qsTr("%1 fill level over the last 15 seconds").arg(br.title)
    }

    Flickable {
        anchors.fill: parent
        anchors.bottomMargin: footer.height
        contentHeight: columns.height + 32
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {}

        Row {
            id: columns
            y: 16

            // ── Column 1: device, system, output counters ──
            Column {
                width: win.columnWidth
                leftPadding: 16
                rightPadding: 16
                spacing: 4
                readonly property real inner: width - 32

                SectionLabel { text: qsTr("DEVICE INFORMATION") }
                Column {
                    width: parent.inner
                    InfoRow { title: qsTr("Platform"); value: bridge.connected ? bridge.platformName : win.dash }
                    InfoRow { title: qsTr("Firmware"); value: bridge.connected ? bridge.firmwareVersion : win.dash }
                    InfoRow { title: qsTr("Serial"); value: bridge.selectedSerial || win.dash }
                    InfoRow { title: qsTr("Reconnects"); value: String(stats.reconnects) }
                }
                Item { width: 1; height: 8 }
                SectionLabel { text: qsTr("SYSTEM INFORMATION") }
                Column {
                    width: parent.inner
                    InfoRow { title: qsTr("Clock Frequency"); value: win.live ? qsTr("%1 MHz").arg(win.info.clockMHz.toFixed(1)) : win.dash }
                    InfoRow { title: qsTr("Core Voltage"); value: win.live && win.info.coreVolts !== undefined ? qsTr("%1 V").arg(win.info.coreVolts.toFixed(2)) : win.dash }
                    InfoRow { title: qsTr("Sample Rate"); value: win.live && win.info.sampleKHz !== undefined ? qsTr("%1 kHz").arg(win.info.sampleKHz.toFixed(1)) : win.dash }
                    InfoRow { title: qsTr("Temperature"); value: win.live && win.info.tempC !== undefined ? qsTr("%1 °C").arg(win.info.tempC.toFixed(1)) : win.dash }
                    InfoRow { title: qsTr("Core 0 Load"); value: bridge.connected ? bridge.cpu0 + " %" : win.dash; valueColor: bridge.cpu0 > 80 ? win.orange : Qt.rgba(1, 1, 1, 0.9) }
                    InfoRow { title: qsTr("Core 1 Load"); value: bridge.connected ? bridge.cpu1 + " %" : win.dash; valueColor: bridge.cpu1 > 80 ? win.orange : Qt.rgba(1, 1, 1, 0.9) }
                    InfoRow { title: qsTr("Core 1 Mode"); value: !bridge.connected ? win.dash : [qsTr("Idle"), qsTr("PDM"), qsTr("EQ Worker")][bridge.core1Mode] || qsTr("Unknown") }
                }
                Item { width: 1; height: 8 }
                SectionLabel { text: qsTr("AUDIO OUTPUT") }
                Column {
                    width: parent.inner
                    CounterRow { title: qsTr("USB Ring"); subtitle: qsTr("ISR → Main Loop"); over: win.info.usbRingOver; hasUnder: false }
                    CounterRow { title: qsTr("Buffer Pool"); subtitle: qsTr("USB → DMA"); over: win.info.spdifOver; under: win.info.spdifUnder }
                }
                // -- WP QD29 -- Output ring health, on firmware that reports it
                Column {
                    objectName: "ringHealthSection"
                    visible: win.ring.targetFrames !== undefined
                    width: parent.inner
                    spacing: 4
                    Item { width: 1; height: 8 }
                    SectionLabel { text: qsTr("OUTPUT RING HEALTH") }
                    Column {
                        width: parent.width
                        RingRow {
                            objectName: "ringFillTarget"
                            title: qsTr("Fill Target")
                            value: qsTr("%1 frames (%2)").arg(win.ring.targetFrames).arg(win.latencyMs(win.ring.targetUs))
                            tip: qsTr("How full the output rings are kept, set by Latency in Settings")
                        }
                        RingRow {
                            objectName: "ringUnderruns"
                            title: qsTr("Underrun Episodes")
                            value: win.num(win.ring.underrunEpisodes)
                            valueColor: win.ring.underrunEpisodes > 0 ? win.red : plain
                            tip: qsTr("Times the outputs ran dry while a source owed audio. A host that pauses its stream does not count.")
                        }
                        RingRow {
                            objectName: "ringRefused"
                            title: qsTr("Refused Blocks")
                            value: win.num(win.ring.refusedBlocks)
                            valueColor: win.ring.refusedBlocks > 0 ? win.orange : plain
                            tip: qsTr("Audio blocks dropped because the rings had no room for them")
                        }
                        RingRow {
                            objectName: "ringLowestFill"
                            title: qsTr("Lowest Fill")
                            value: win.ring.lowestFill !== undefined ? qsTr("%1 frames").arg(win.ring.lowestFill) : win.dash
                            tip: qsTr("The emptiest any output ring got just before new audio arrived, since the watermarks were reset. Near zero means the margin is thin.")
                        }
                        RingRow {
                            objectName: "ringInputGap"
                            title: qsTr("Worst Input Gap")
                            value: win.gapText(win.ring.worstInputGapUs)
                            tip: qsTr("Longest wait between input deliveries since the watermarks were reset")
                        }
                        RingRow {
                            objectName: "ringUsbGap"
                            title: qsTr("Worst USB Gap")
                            value: win.gapText(win.ring.worstUsbGapUs)
                            tip: qsTr("Longest gap between USB audio packets since the watermarks were reset. Long gaps are the host pausing its stream.")
                        }
                        RingRow {
                            objectName: "ringOverBudget"
                            title: qsTr("Over-Budget Steps")
                            value: win.num(win.ring.overBudgetSteps)
                            valueColor: win.ring.overBudgetSteps > 0 ? win.orange : plain
                            tip: qsTr("Background tasks that ran past their time slice in the device's main loop")
                        }
                    }
                }
                Item { width: 1; height: 8 }
                SectionLabel { text: qsTr("PDM (SUBWOOFER)") }
                Column {
                    width: parent.inner
                    CounterRow { title: qsTr("Ring Buffer"); subtitle: qsTr("Core 0 → Core 1"); over: win.info.pdmRingOver; under: win.info.pdmRingUnder }
                    CounterRow { title: qsTr("DMA Buffer"); subtitle: qsTr("Core 1 → PIO"); over: win.info.pdmDmaOver; under: win.info.pdmDmaUnder }
                }
            }

            Rectangle { width: 1; height: columns.height; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.07) }

            // ── Column 2: starvation and buffer fill ──
            Column {
                width: win.columnWidth
                leftPadding: 16
                rightPadding: 16
                spacing: 4
                readonly property real inner: width - 32

                SectionLabel { text: qsTr("SPDIF DMA STARVATION") }
                Column {
                    width: parent.inner
                    Item {
                        width: parent.width
                        height: 26
                        Text { anchors.verticalCenter: parent.verticalCenter; text: qsTr("Total"); font.pixelSize: 11; font.weight: Font.Medium; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.75) }
                        Row {
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            spacing: 8
                            Rectangle {
                                visible: (win.info.starvationDelta || 0) > 0
                                width: deltaText.width + 10
                                height: 16
                                radius: 3
                                color: isMacOS ? MacColors.opacity(MacColors.red, 0.15) : Qt.rgba(1, 0.27, 0.23, 0.15)
                                anchors.verticalCenter: parent.verticalCenter
                                Text { id: deltaText; anchors.centerIn: parent; text: "+" + win.info.starvationDelta; font.pixelSize: 10; font.weight: Font.Bold; color: win.red }
                            }
                            Text {
                                text: win.num(win.info.starvationTotal)
                                font.pixelSize: 14
                                font.weight: Font.Bold
                                color: isMacOS ? ((win.info.starvationTotal || 0) > 0 ? win.red : MacColors.label) : (win.info.starvationTotal || 0) > 0 ? win.red : Qt.rgba(1, 1, 1, 0.9)
                            }
                        }
                    }
                    Repeater {
                        model: Math.max(win.buf.numSpdif || 0, 2)
                        InfoRow {
                            readonly property var per: win.info.starvationPer || []
                            title: qsTr("Out %1/%2").arg(index * 2 + 1).arg(index * 2 + 2)
                            secondaryTitle: true
                            value: per.length > index ? String(per[index]) : win.dash
                            valueColor: isMacOS ? ((per[index] || 0) > 0 ? win.red : MacColors.label) : (per[index] || 0) > 0 ? win.red : Qt.rgba(1, 1, 1, 0.9)
                        }
                    }
                    InfoRow { title: qsTr("Time since last event"); secondaryValue: true; value: (win.info.lastEventMs || 0) > 0 ? win.span(win.now - win.info.lastEventMs) : win.dash }
                    InfoRow {
                        title: qsTr("Time between last two")
                        secondaryValue: true
                        value: (win.info.previousEventMs || 0) > 0 ? win.span(win.info.lastEventMs - win.info.previousEventMs) : win.dash
                    }
                }
                Item { width: 1; height: 8 }
                RowLayout {
                    width: parent.inner
                    SectionLabel { text: qsTr("BUFFER FILL LEVELS") }
                    Item { Layout.fillWidth: true }
                    Rectangle {
                        Layout.preferredWidth: resetText.width + 16
                        Layout.preferredHeight: 20
                        radius: 6
                        color: isMacOS ? (resetMouse.pressed ? MacColors.opacity(MacColors.control, 1.6) : MacColors.control) : resetMouse.pressed ? Qt.rgba(1, 1, 1, 0.14) : resetMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.08) : "transparent"
                        border.color: isMacOS ? "transparent" : Qt.rgba(1, 1, 1, 0.18)
                        Text { id: resetText; anchors.centerIn: parent; text: qsTr("Reset Watermarks"); font.pixelSize: 10; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.85) }
                        ClickArea { id: resetMouse; enabled: bridge.writable; anchors.fill: parent; hoverEnabled: true; cursorShape: Qt.PointingHandCursor; focusRadius: 8
                                    Accessible.name: resetText.text; onActivated: stats.resetWatermarks() }
                    }
                }
                Row {
                    spacing: 14
                    Repeater {
                        model: [{ t: qsTr("Audio Streaming"), on: win.buf.streaming === true }, { t: qsTr("PDM Active"), on: win.buf.pdmActive === true }]
                        Row {
                            spacing: 5
                            Rectangle { width: 6; height: 6; radius: 3; color: modelData.on ? win.green : win.grey; anchors.verticalCenter: parent.verticalCenter }
                            Text { text: modelData.t; font.pixelSize: 10; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.55) }
                        }
                    }
                }
                Column {
                    width: parent.inner
                    spacing: 6
                    topPadding: 4
                    // Keyed on the count: the levels change every poll, the rows don't
                    Repeater {
                        model: win.buf.numSpdif || 0
                        BufferRow {
                            title: qsTr("Out %1/%2 (%3)").arg(index * 2 + 1).arg(index * 2 + 2).arg(win.outputTypes[index] === 1 ? "I2S" : "S/PDIF")
                            tint: bridge.channelColor(2 + index * 2)
                            series: index
                            levels: (win.buf.spdif || [])[index] || ({})
                            kind: 0
                            objectName: "outputFillRow" + index
                        }
                    }
                    BufferRow {
                        visible: win.buf.pdmActive === true
                        title: qsTr("PDM DMA")
                        tint: bridge.channelColor(bridge.numOutputChannels + 1)
                        series: 4
                        levels: win.buf.pdmDma || ({})
                        kind: 1
                    }
                    BufferRow {
                        visible: win.buf.pdmActive === true
                        title: qsTr("PDM Ring")
                        tint: bridge.channelColor(bridge.numOutputChannels + 1)
                        hollow: true
                        series: 5
                        levels: win.buf.pdmRing || ({})
                        kind: 2
                    }
                }
            }

            Rectangle { width: 1; height: columns.height; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.07) }

            // ── Column 3: inputs and links, when they apply ──
            Column {
                width: win.columnWidth
                leftPadding: 16
                rightPadding: 16
                spacing: 4
                readonly property real inner: width - 32
                readonly property var rx: win.info.spdifRx
                readonly property var lg: win.info.lg
                readonly property var adat: win.info.adatOut
                readonly property var i2s: win.info.i2sSlave
                readonly property var clockRows: stats.clockDiag

                Text {
                    visible: !parent.rx && !parent.lg && !parent.adat && !parent.i2s && parent.clockRows.length === 0
                    width: parent.inner
                    wrapMode: Text.WordWrap
                    topPadding: 4
                    text: bridge.connected ? qsTr("No digital inputs or links to report.") : qsTr("Connect a DSPi to see its statistics.")
                    font.pixelSize: 11
                    color: Qt.rgba(1, 1, 1, 0.4)
                }

                // S/PDIF input
                Column {
                    visible: !!parent.rx
                    width: parent.inner
                    spacing: 0
                    readonly property var r: parent.rx || ({})
                    SectionLabel { text: qsTr("S/PDIF INPUT") }
                    StateRow {
                        title: qsTr("State")
                        text: [qsTr("Inactive"), qsTr("Acquiring"), qsTr("Locked"), qsTr("Relocking")][parent.r.state] || qsTr("Inactive")
                        dot: [win.grey, win.yellow, win.green, win.orange][parent.r.state] || win.grey
                    }
                    InfoRow { title: qsTr("Active Source"); value: parent.r.source || win.dash }
                    InfoRow { title: qsTr("Sample Rate"); value: parent.r.locked ? qsTr("%1 kHz").arg(parent.r.rateKHz.toFixed(1)) : win.dash }
                    InfoRow { title: qsTr("Lock Count"); value: win.num(parent.r.lockCount) }
                    InfoRow { title: qsTr("Loss Count"); value: win.num(parent.r.lossCount) }
                    InfoRow { title: qsTr("Parity Errors"); value: parent.r.locked ? win.num(parent.r.parityErrors) : win.dash }
                    InfoRow { title: qsTr("FIFO Fill"); value: parent.r.locked ? parent.r.fifoPct + "%" : win.dash }
                    InfoRow { title: qsTr("RX Pin"); value: parent.r.pin !== undefined ? qsTr("GPIO %1").arg(parent.r.pin) : win.dash }
                    Column {
                        visible: parent.r.locked === true && parent.r.category !== undefined
                        width: parent.width
                        Item { width: 1; height: 6 }
                        SectionLabel { text: qsTr("CHANNEL STATUS") }
                        InfoRow { title: qsTr("Format"); value: parent.parent.r.consumer ? qsTr("Consumer") : qsTr("Professional") }
                        InfoRow { title: qsTr("Audio"); value: parent.parent.r.pcm ? qsTr("PCM") : qsTr("Non-PCM") }
                        InfoRow { title: qsTr("Category"); value: parent.parent.r.category || win.dash }
                        InfoRow { title: qsTr("Word Length"); value: parent.parent.r.wordLength || win.dash }
                        InfoRow { title: qsTr("Copy"); value: parent.parent.r.copyPermitted ? qsTr("Permitted") : qsTr("Prohibited") }
                    }
                    Column {
                        visible: (parent.r.state || 0) !== 0
                        width: parent.width
                        Item { width: 1; height: 6 }
                        SectionLabel { text: qsTr("DEBUG") }
                        InfoRow { title: qsTr("Library State"); value: [qsTr("No Signal"), qsTr("Waiting Stable"), qsTr("Stable")][parent.parent.r.libState] || win.dash }
                        InfoRow { title: qsTr("Stable Callbacks"); value: win.num(parent.parent.r.stableCallbacks) }
                        InfoRow { title: qsTr("Lost Callbacks"); value: win.num(parent.parent.r.lostCallbacks) }
                    }
                    Item { width: 1; height: 8 }
                }

                // LG Sound Sync
                Column {
                    visible: !!parent.lg
                    width: parent.inner
                    readonly property var l: parent.lg || ({})
                    SectionLabel { text: qsTr("LG SOUND SYNC") }
                    InfoRow { title: qsTr("Enabled"); value: parent.l.enabled ? qsTr("Yes") : qsTr("No") }
                    StateRow { title: qsTr("Present"); text: parent.l.present ? qsTr("Yes") : qsTr("No"); dot: parent.l.present ? win.green : win.grey }
                    InfoRow { title: qsTr("TV Volume"); value: parent.l.volume === undefined || parent.l.volume === 255 ? win.dash : parent.l.volume + " / 100" }
                    InfoRow { title: qsTr("TV Mute"); value: parent.l.muted ? qsTr("On") : qsTr("Off") }
                    Item { width: 1; height: 8 }
                }

                // ADAT bulk output
                Column {
                    visible: !!parent.adat
                    width: parent.inner
                    readonly property var a: parent.adat || ({})
                    SectionLabel { text: qsTr("ADAT BULK OUTPUT") }
                    StateRow {
                        title: qsTr("State")
                        text: !parent.a.enabled ? qsTr("Disabled") : parent.a.active ? qsTr("Streaming") : !parent.a.rateOk ? qsTr("Suspended (rate above 48 kHz)") : qsTr("Enabled")
                        dot: !parent.a.enabled ? win.grey : parent.a.active ? win.green : !parent.a.rateOk ? win.orange : win.yellow
                    }
                    InfoRow { title: qsTr("Streaming"); value: parent.a.active ? qsTr("Yes") : qsTr("No") }
                    InfoRow { title: qsTr("Rate Supported"); value: parent.a.rateOk ? qsTr("Yes") : qsTr("No") }
                    InfoRow { title: qsTr("Data Pin"); value: parent.a.pin !== undefined ? qsTr("GPIO %1").arg(parent.a.pin) : win.dash }
                    InfoRow { title: qsTr("Resync Count"); value: win.num(parent.a.resyncCount) }
                    InfoRow { title: qsTr("Slip Count"); value: win.num(parent.a.slipCount) }
                    Item { width: 1; height: 8 }
                }

                // I2S input as clock slave
                Column {
                    visible: !!parent.i2s
                    width: parent.inner
                    readonly property var s: parent.i2s || ({})
                    SectionLabel { text: qsTr("I2S INPUT (SLAVE CLOCK)") }
                    StateRow {
                        title: qsTr("State")
                        text: [qsTr("Inactive"), qsTr("Acquiring"), qsTr("Relocking"), qsTr("Locked")][parent.s.state] || qsTr("Inactive")
                        dot: [win.grey, win.yellow, win.orange, win.green][parent.s.state] || win.grey
                    }
                    InfoRow { title: qsTr("Detected Rate"); value: (parent.s.detectedKHz || 0) > 0 ? qsTr("%1 kHz").arg(parent.s.detectedKHz.toFixed(1)) : win.dash }
                    InfoRow { title: qsTr("Measured Rate"); value: (parent.s.measuredKHz || 0) > 0 ? qsTr("%1 kHz").arg(parent.s.measuredKHz.toFixed(1)) : win.dash }
                    InfoRow { title: qsTr("Lock Count"); value: win.num(parent.s.lockCount) }
                    InfoRow { title: qsTr("Loss Count"); value: win.num(parent.s.lossCount) }
                    Item { width: 1; height: 8 }
                }

                // -- WP QD5 -- Clock diagnostics, on firmware built with them
                Column {
                    objectName: "clockDiagSection"
                    visible: parent.clockRows.length > 0
                    width: parent.inner
                    SectionLabel { text: qsTr("CLOCK DIAGNOSTICS") }
                    Repeater {
                        model: stats.clockDiag
                        InfoRow { title: modelData.title; value: modelData.value }
                    }
                }
            }
        }
    }

    // Footer: connection state
    Rectangle {
        id: footer
        anchors.bottom: parent.bottom
        width: parent.width
        height: 30
        color: isMacOS ? "transparent" : Qt.rgba(1, 1, 1, 0.03)
        Rectangle { width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.08) }
        Row {
            x: 16
            anchors.verticalCenter: parent.verticalCenter
            spacing: 7
            Rectangle { width: 6; height: 6; radius: 3; anchors.verticalCenter: parent.verticalCenter; color: bridge.connected ? win.green : win.red }
            Text { text: bridge.connected ? qsTr("Connected") : qsTr("Disconnected"); font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.55) }
        }
        Text {
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("Updated every 2 seconds")
            font.pixelSize: 11
            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.4)
        }
    }
}
