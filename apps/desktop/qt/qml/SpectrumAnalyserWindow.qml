import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Layouts 1.15
import QtQuick.Window 2.15
import DSPi 1.0
import "components"

// Spectrum Analyser: a larger view of the main window's spectrum. It shows
// the channels the current page selects (the graph's gear picks them) as
// curves, bars or both; channels can be hidden here without changing what
// the device analyses. The status bar reports the device's analyser.
// -- WP QD19 -- While another console drives the device's analyser (DSPi
// Link), the window shows the device's tap and channels, says so in the status
// bar, and offers Take control. Its views always ask for the page's own
// selection (`request`; hidden channels too, as above), so only a change there
// takes the analyser back.
AppWindow {
    id: win
    title: qsTr("Spectrum Analyser")
    visible: false
    width: 760
    height: 560 + titlebarHeight
    minimumWidth: 620
    minimumHeight: 420 + titlebarHeight

    property var app                      // main window
    readonly property var ownSel: app ? app.rtaSelection : ({ tap: 1, channels: [] })
    readonly property var sel: rta.following ? ({ tap: rta.followedTap, channels: rta.followedChannels }) : ownSel
    readonly property bool shown: visible && visibility !== Window.Minimized
    // 0 curves, 1 bars, 2 both; follows the page's switches each time it opens
    property int mode: 0
    property var hiddenChannels: []
    readonly property var visibleChannels: sel.channels.filter(function (c) { return hiddenChannels.indexOf(c) < 0 })

    onVisibleChanged: if (visible && app) mode = app.rtaShowGraph && app.rtaShowBars ? 2 : app.rtaShowBars ? 1 : 0
    onSelChanged: hiddenChannels = hiddenChannels.filter(function (c) { return sel.channels.indexOf(c) >= 0 })

    // -- WP QD23 -- The load figures (shrinks) give way with an ellipsis when the
    // window is too narrow for the status line, after the range text, rather
    // than run past its edge
    component Stat: Text {
        property bool shrinks: false
        font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5); visible: rta.running
        elide: Text.ElideRight; Layout.fillWidth: shrinks && statusLine.othersWidth > statusLine.width; Layout.maximumWidth: implicitWidth
    }

    function colorOf(c) { return bridge.channelColor(rta.appChannel(sel.tap, c)) }
    function nameOf(c) { return bridge.channelName(rta.appChannel(sel.tap, c)) }
    function toggleHidden(c) {
        var h = hiddenChannels.slice(), i = h.indexOf(c)
        if (i >= 0) h.splice(i, 1); else h.push(c)
        hiddenChannels = h
    }

    Rectangle {
        anchors.fill: parent
        color: "transparent"

        // ── Header: page, side, channels, view ──
        Item {
            id: header
            width: parent.width
            height: 34
            RowLayout {
                anchors.fill: parent
                anchors.leftMargin: 12
                anchors.rightMargin: 12
                spacing: 10
                Text {
                    text: !win.app || win.app.onDashboard ? qsTr("Dashboard") : bridge.channelName(win.app.openChannelId)
                    font.pixelSize: 12
                    font.weight: Font.DemiBold
                    color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9)
                }
                Text {
                    text: win.sel.tap === 0 ? qsTr("Inputs") : qsTr("Outputs")
                    font.pixelSize: 12
                    color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
                }
                Rectangle { width: 1; height: 14; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.12) }
                Flickable {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    contentWidth: chips.width
                    clip: true
                    boundsBehavior: Flickable.StopAtBounds
                    flickableDirection: Flickable.HorizontalFlick
                    Row {
                        id: chips
                        height: parent.height
                        spacing: 12
                        Repeater {
                            model: win.sel.channels
                            Item {
                                readonly property bool hidden: win.hiddenChannels.indexOf(modelData) >= 0
                                width: chipRow.width
                                height: parent.height
                                Row {
                                    id: chipRow
                                    anchors.verticalCenter: parent.verticalCenter
                                    spacing: 5
                                    Rectangle {
                                        width: 6; height: 6; radius: 3
                                        anchors.verticalCenter: parent.verticalCenter
                                        color: win.colorOf(modelData)
                                        opacity: parent.parent.hidden ? 0.25 : 1
                                    }
                                    Text {
                                        text: win.nameOf(modelData)
                                        font.pixelSize: 12
                                        font.strikeout: parent.parent.hidden
                                        color: isMacOS ? (parent.parent.hidden ? MacColors.opacity(MacColors.secondaryLabel, 0.4) : MacColors.secondaryLabel) : Qt.rgba(1, 1, 1, parent.parent.hidden ? 0.3 : (chipMouse.containsMouse ? 0.9 : 0.65))
                                    }
                                }
                                ClickArea {
                                    id: chipMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    Accessible.role: Accessible.CheckBox
                                    Accessible.name: qsTr("%1 shown").arg(win.nameOf(modelData))
                                    Accessible.checkable: true
                                    Accessible.checked: !parent.hidden
                                    onActivated: win.toggleHidden(modelData)
                                    ToolTip.visible: containsMouse
                                    ToolTip.delay: 600
                                    ToolTip.text: parent.hidden ? qsTr("Show %1 in this window").arg(win.nameOf(modelData)) : qsTr("Hide %1 in this window").arg(win.nameOf(modelData))
                                }
                            }
                        }
                    }
                }
                SegmentedControl {
                    Layout.preferredWidth: 180
                    height: 24
                    model: [qsTr("Curves"), qsTr("Bars"), qsTr("Both")]
                    currentIndex: win.mode
                    onActivated: win.mode = index
                }
            }
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.07) }
        }

        // ── Body ──
        Rectangle {
            id: body
            anchors.top: header.bottom
            anchors.bottom: statusBar.top
            width: parent.width
            color: Qt.rgba(0, 0, 0, 0.2)

            readonly property string notice: !rta.supported ? (bridge.connected ? qsTr("This firmware has no spectrum analyser") : qsTr("Connect a DSPi to see its spectrum"))
                : win.sel.channels.length === 0 ? qsTr("No channels selected")
                : win.visibleChannels.length === 0 ? qsTr("Every channel is hidden in this window") : ""
            readonly property string noticeDetail: !rta.supported ? ""
                : win.sel.channels.length === 0 ? qsTr("Pick channels with the gear on the response graph.") : ""

            Column {
                visible: body.notice !== ""
                anchors.centerIn: parent
                spacing: 8
                Icon { anchors.horizontalCenter: parent.horizontalCenter; name: "spectrum"; size: 28; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.35) }
                Text { anchors.horizontalCenter: parent.horizontalCenter; text: body.notice; font.pixelSize: 13; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.7) }
                Text { anchors.horizontalCenter: parent.horizontalCenter; visible: text !== ""; text: body.noticeDetail; font.pixelSize: 12; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45) }
            }

            ColumnLayout {
                visible: body.notice === ""
                anchors.fill: parent
                anchors.margins: 12
                spacing: 12

                // Curves on a dBFS grid with frequency lines
                Item {
                    visible: win.mode !== 1
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredHeight: 100     // with the bars: half each
                    Canvas {
                        id: grid
                        anchors.fill: parent
                        readonly property real floorDb: win.app ? win.app.rtaFloorDb : -90
                        readonly property real ceilingDb: win.app ? win.app.rtaCeilingDb : 6
                        readonly property real minF: win.app ? win.app.graphMinFreq : 15
                        readonly property real maxF: win.app ? win.app.graphMaxFreq : 20000
                        onFloorDbChanged: requestPaint()
                        onCeilingDbChanged: requestPaint()
                        onMinFChanged: requestPaint()
                        onMaxFChanged: requestPaint()
                        onWidthChanged: requestPaint()
                        onHeightChanged: requestPaint()
                        onPaint: {
                            var ctx = getContext("2d")
                            ctx.reset()
                            var h = height - 12
                            ctx.font = "10px 'Noto Sans'"
                            ctx.textBaseline = "middle"
                            for (var db = Math.floor(ceilingDb / 12) * 12; db > floorDb; db -= 12) {
                                var y = Math.round(h - (db - floorDb) / (ceilingDb - floorDb) * h) + 0.5
                                ctx.strokeStyle = isMacOS ? MacColors.opacity(MacColors.secondaryLabel, db === 0 ? 0.35 : 0.12) : db === 0 ? "rgba(255,255,255,0.2)" : "rgba(255,255,255,0.07)"
                                ctx.lineWidth = db === 0 ? 1 : 0.5
                                ctx.beginPath(); ctx.moveTo(0, y); ctx.lineTo(width, y); ctx.stroke()
                                ctx.fillStyle = isMacOS ? MacColors.opacity(MacColors.secondaryLabel, 0.6) : "rgba(255,255,255,0.4)"
                                ctx.textAlign = "right"
                                ctx.fillText(db + "", width - 3, y - 7)
                            }
                            var lo = Math.log(minF) / Math.LN10, hi = Math.log(maxF) / Math.LN10
                            var marks = [10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000]
                            ctx.textAlign = "center"
                            ctx.textBaseline = "top"
                            for (var i = 0; i < marks.length; i++) {
                                var f = marks[i]
                                if (f < minF || f > maxF) continue
                                var x = Math.round((Math.log(f) / Math.LN10 - lo) / (hi - lo) * width) + 0.5
                                ctx.strokeStyle = isMacOS ? MacColors.opacity(MacColors.secondaryLabel, 0.1) : "rgba(255,255,255,0.06)"
                                ctx.lineWidth = 0.5
                                ctx.beginPath(); ctx.moveTo(x, 0); ctx.lineTo(x, h); ctx.stroke()
                                ctx.fillStyle = isMacOS ? MacColors.secondaryLabel : "rgba(255,255,255,0.45)"
                                var t = f >= 1000 ? (f / 1000) + "k" : f + ""
                                ctx.fillText(t, Math.max(8, Math.min(width - 10, x)), h + 1)
                            }
                        }
                    }
                    SpectrumCurveItem {
                        anchors.fill: parent
                        controller: rta
                        active: win.shown && win.mode !== 1
                        visible: active
                        tap: win.sel.tap
                        channels: win.sel.channels
                        request: win.ownSel
                        hidden: win.hiddenChannels
                        colors: channels.map(function (c) { return win.colorOf(c) })
                        bottomInset: 12
                        floorDb: grid.floorDb
                        ceilingDb: grid.ceilingDb
                        minFreq: grid.minF
                        maxFreq: grid.maxF
                        strength: 1.0
                        glow: win.app ? win.app.graphShowGlow : false
                        showPeak: win.app ? win.app.rtaShowPeakHold : true
                        smoothing: win.app ? win.app.rtaSmoothing : true
                    }
                }

                // Bars: a grid of channels
                Grid {
                    id: barGrid
                    visible: win.mode !== 0
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    Layout.preferredHeight: 100
                    readonly property int count: win.visibleChannels.length
                    columns: Math.max(1, Math.min(count, win.app ? win.app.rtaBarColumns : 2))
                    readonly property int rowCount: Math.ceil(count / columns)
                    columnSpacing: 12
                    rowSpacing: 12
                    Repeater {
                        model: win.visibleChannels
                        Column {
                            width: (barGrid.width - (barGrid.columns - 1) * 12) / barGrid.columns
                            height: (barGrid.height - (barGrid.rowCount - 1) * 12) / barGrid.rowCount
                            spacing: 3
                            Row {
                                height: 16
                                spacing: 5
                                Rectangle { width: 6; height: 6; radius: 3; color: win.colorOf(modelData); anchors.verticalCenter: parent.verticalCenter }
                                Text { text: win.nameOf(modelData); font.pixelSize: 12; font.weight: Font.DemiBold; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.65); anchors.verticalCenter: parent.verticalCenter }
                            }
                            SpectrumBarsItem {
                                width: parent.width
                                height: parent.height - 19
                                controller: rta
                                active: win.shown && win.mode !== 0
                                tap: win.sel.tap
                                channel: modelData
                                request: win.ownSel
                                color: win.colorOf(modelData)
                                levelLabels: true
                                floorDb: grid.floorDb
                                ceilingDb: grid.ceilingDb
                                showPeak: win.app ? win.app.rtaShowPeakHold : true
                                smoothing: win.app ? win.app.rtaSmoothing : true
                            }
                        }
                    }
                }
            }
        }

        // ── Status bar: the device's analyser ──
        Rectangle {
            id: statusBar
            anchors.bottom: parent.bottom
            width: parent.width
            height: 28
            color: "transparent"
            Rectangle { width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.07) }
            RowLayout {
                id: statusLine
                anchors.fill: parent
                anchors.leftMargin: 12
                anchors.rightMargin: 12
                spacing: 14
                visible: rta.supported
                // -- WP QD23 -- What the line holds besides the range text, at full width
                readonly property real othersWidth: {
                    var w = 0
                    for (var i = 0; i < children.length; i++)
                        if (children[i] !== rangeText && children[i].visible) w += children[i].implicitWidth + spacing
                    return w
                }
                Row {
                    spacing: 6
                    Rectangle { width: 6; height: 6; radius: 3; color: isMacOS ? (rta.running ? MacColors.green : MacColors.opacity(MacColors.secondaryLabel, 0.5)) : rta.running ? "#32d74b" : Qt.rgba(1, 1, 1, 0.3); anchors.verticalCenter: parent.verticalCenter }
                    Text { text: rta.running ? qsTr("Running") : qsTr("Idle"); font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.55) }
                }
                Stat {
                    text: rta.liveCount === 1 ? qsTr("%1 channel, each refreshed every %2 ms").arg(rta.liveCount).arg(Math.round(rta.refreshMs))
                          : qsTr("%1 channels, each refreshed every %2 ms").arg(rta.liveCount).arg(Math.round(rta.refreshMs))
                }
                Stat { text: qsTr("%1 frames/s").arg(String(rta.framesPerSecond)) }
                // -- WP QD19 -- The load figures make room for the following line
                Stat { shrinks: true; text: qsTr("transform %1 µs").arg(String(rta.transformUs)); visible: rta.running && !rta.following }
                Stat { shrinks: true; text: qsTr("main loop %1%").arg(rta.mainLoopPercent.toFixed(1)); visible: rta.running && !rta.following }
                Stat { shrinks: true; text: rta.bassSaturated ? qsTr("bass ≥%1%").arg(rta.bassPercent.toFixed(2)) : qsTr("bass %1%").arg(rta.bassPercent.toFixed(2)); visible: rta.running && !rta.following }
                Item { Layout.fillWidth: true }
                // -- WP QD19 --
                Row {
                    visible: rta.following
                    spacing: 10
                    Text {
                        objectName: "rtaFollowingText"
                        text: qsTr("Following another console")
                        font.pixelSize: 11
                        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.55)
                    }
                    Text {
                        id: takeControl
                        objectName: "rtaTakeControl"
                        signal clicked()
                        text: qsTr("Take control")
                        font.pixelSize: 11
                        font.underline: takeMouse.containsMouse
                        color: isMacOS ? MacColors.accent : "#0a7cff"
                        onClicked: rta.takeControl()
                        ClickArea {
                            id: takeMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            Accessible.role: Accessible.Link
                            Accessible.name: takeControl.text
                            onActivated: takeControl.clicked()
                        }
                    }
                }
                Text {
                    id: rangeText
                    visible: !rta.following
                    readonly property bool shaded: win.mode !== 0 && rta.running && rta.appliedOrder < rta.orderMax && rta.lowestShadedHz > 0
                    font.pixelSize: 11
                    elide: Text.ElideLeft
                    // -- WP QD23 -- Gives way first when the line is too long, then the load figures
                    Layout.preferredWidth: Math.max(0, Math.min(320, implicitWidth, statusLine.width - statusLine.othersWidth - 2))
                    color: isMacOS ? (rta.configRejected || shaded ? MacColors.orange : MacColors.secondaryLabel) : rta.configRejected || shaded ? "#ff9f0a" : Qt.rgba(1, 1, 1, 0.4)
                    text: rta.configRejected ? qsTr("Device refused this configuration")
                        : shaded ? qsTr("Bands up to %1 Hz need a larger transform size in Settings").arg(String(rta.lowestShadedHz))
                        : qsTr("%1 dB range (bass %2 dB)").arg(String(rta.dynamicRange)).arg(String(rta.bassDynamicRange))
                }
            }
        }
    }
}
