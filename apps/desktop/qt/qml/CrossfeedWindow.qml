import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Window 2.15
import "components"

// Crossfeed: BS2B headphone crossfeed. Laid out as on the macOS Console:
// response and presets on the left, parameters, ITD and output pairs on the
// right. On firmware with the pair map (`ext`, src/ExtController.h) the output
// pairs section also says which two outputs each pair processes.
AppWindow {
    id: win
    fitHeight: header.height + flick.contentHeight
    title: "Crossfeed"
    visible: false
    width: 720
    height: 470 + titlebarHeight
    minimumWidth: 640
    minimumHeight: 360 + titlebarHeight

    readonly property var presets: [
        { name: "Default",   detail: "700 Hz / 4.5 dB · Balanced, most popular", fc: 700, feed: 4.5 },
        { name: "Chu Moy",   detail: "700 Hz / 6.0 dB · Stronger spatial effect", fc: 700, feed: 6.0 },
        { name: "Jan Meier", detail: "650 Hz / 9.5 dB · Natural speaker-like", fc: 650, feed: 9.5 },
        { name: "Custom",    detail: "User-defined parameters" }]
    readonly property bool isCustom: bridge.crossfeedPreset === 3
    // What the rows and the graph show: a preset's own values (crossfeed.c), which
    // the device doesn't report, or the device's custom values under Custom
    readonly property var activePreset: presets[bridge.crossfeedPreset] || presets[3]
    readonly property real shownFreq: activePreset.fc !== undefined ? activePreset.fc : bridge.crossfeedFreq
    readonly property real shownFeed: activePreset.feed !== undefined ? activePreset.feed : bridge.crossfeedFeed
    // Stereo output pairs (S/PDIF instances); the mono PDM sub is never crossfed
    readonly property int pairCount: Math.max(1, Math.floor(bridge.numOutputChannels / 2))

    // The pair map section: hidden when the firmware stalls the extension channel
    readonly property bool pairMapShown: ext.hasPairMap && bridge.connected
    readonly property var pairChoices: [
        { key: "default", text: "Default (1/2, 3/4, …)" },
        { key: "k340", text: "One DAC per ear (K340)" },
        { key: "custom", text: "Custom…" }]
    property bool editingPairs: false
    property bool offerK340Mask: false
    property bool applyingDraft: false
    property var pairDraft: []           // one { left, right } per pair
    property var outputLabels: []        // the editor's menus, taken when it opens
    readonly property bool draftValid: ext.pairsValid(pairDraft)
    readonly property bool k340MaskOffered: offerK340Mask && ext.choice === "k340"
        && (bridge.crossfeedOutputPairMask & ext.k340PairMask) !== ext.k340PairMask

    onVisibleChanged: ext.watching = visible
    onPairMapShownChanged: if (!pairMapShown) { editingPairs = false; offerK340Mask = false }

    function outputName(o) { return bridge.channelName(o + 2) || "Out " + (o + 1) }

    function pairNames() {
        var n = []
        var pairs = pairMapShown ? ext.pairs : []
        for (var p = 0; p < pairCount; p++) {
            var left = p < pairs.length ? pairs[p].left : 2 * p
            var right = p < pairs.length ? pairs[p].right : 2 * p + 1
            n.push(outputName(left) + " / " + outputName(right))
        }
        return n
    }

    function pickPairChoice(key) {
        ext.clearError()
        if (key === "custom") {
            var labels = []
            for (var o = 0; o < 2 * ext.pairCount; o++) labels.push((o + 1) + " — " + outputName(o))
            outputLabels = labels
            pairDraft = ext.pairs
            editingPairs = true
            return
        }
        offerK340Mask = key === "k340"
        ext.applyPreset(key)
    }
    function setDraftOutput(pair, side, output) {
        var draft = []
        for (var p = 0; p < pairDraft.length; p++)
            draft.push({ left: pairDraft[p].left, right: pairDraft[p].right })
        draft[pair][side] = output
        pairDraft = draft
    }
    function applyDraft() { applyingDraft = ext.applyPairs(pairDraft) }
    function cancelDraft() { ext.clearError(); editingPairs = false }
    function enableK340Mask() {
        bridge.setCrossfeedOutputPairMask(bridge.crossfeedOutputPairMask | ext.k340PairMask)
        offerK340Mask = false
    }

    Connections {
        target: ext
        function onApplyFinished(ok) {
            if (win.applyingDraft && ok) win.editingPairs = false
            win.applyingDraft = false
        }
    }

    component SectionLabel: Text {
        font.pixelSize: 11
        font.weight: Font.Bold
        font.letterSpacing: 0.4
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
    }
    component Divider: Rectangle { width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.08) }

    ToolHeader {
        id: header
        width: parent.width
        icon: "headphones"
        title: "Crossfeed"
        subtitle: "BS2B Bauer stereophonic-to-binaural"
        checked: bridge.crossfeedEnabled
        onToggled: bridge.setCrossfeed(enable)
    }

    Flickable {
        id: flick
        anchors.top: header.bottom
        anchors.bottom: parent.bottom
        width: parent.width
        contentHeight: columns.height + 32
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {}

        Row {
            id: columns
            x: 16
            y: 16
            spacing: 20
            enabled: bridge.connected
            readonly property real colWidth: (win.width - 32 - 2 * 20 - 1) / 2

            // ── Left: response + presets ──
            Column {
                width: columns.colWidth
                spacing: 12

                SectionLabel { text: "FREQUENCY RESPONSE" }

                Rectangle {
                    width: parent.width
                    height: 170
                    radius: 10
                    color: isMacOS ? MacColors.opacity(MacColors.controlBackground, 0.6) : Qt.rgba(0, 0, 0, 0.2)
                    border.color: isMacOS ? MacColors.opacity(MacColors.gray, 0.2) : Qt.rgba(1, 1, 1, 0.1)

                    Canvas {
                        id: graph
                        anchors.fill: parent
                        anchors.margins: 10
                        // Follow the sliders during a drag
                        property real fc: freqRow.displayValue
                        property real feed: feedRow.displayValue
                        property bool active: bridge.crossfeedEnabled
                        onFcChanged: requestPaint()
                        onFeedChanged: requestPaint()
                        onActiveChanged: requestPaint()
                        onWidthChanged: requestPaint()
                        onHeightChanged: requestPaint()

                        // Complementary first-order lowpass (crossfeed) and
                        // 1 - lowpass (direct), as the firmware applies them
                        function curves() {
                            var fs = 48000
                            var G = 1 / (1 + Math.pow(10, feed / 20))
                            var x = Math.exp(-2 * Math.PI * fc / fs)
                            var a0 = G * (1 - x)
                            var cf = [], dr = []
                            for (var i = 0; i < 100; i++) {
                                var f = 20 * Math.pow(1000, i / 99)
                                var w = 2 * Math.PI * f / fs
                                var re = 1 - x * Math.cos(w), im = x * Math.sin(w)
                                var d2 = re * re + im * im
                                var lr = a0 * re / d2, li = -a0 * im / d2
                                cf.push([f, 20 * Math.log(Math.max(Math.sqrt(lr * lr + li * li), 1e-10)) / Math.LN10])
                                var dre = 1 - lr, dim = -li
                                dr.push([f, 20 * Math.log(Math.max(Math.sqrt(dre * dre + dim * dim), 1e-10)) / Math.LN10])
                            }
                            return { cf: cf, dr: dr }
                        }

                        onPaint: {
                            var ctx = getContext("2d")
                            ctx.reset()
                            var left = 24, w = width - left, h = height - 14
                            var lo = Math.log(20), hi = Math.log(20000)
                            function xOf(f) { return left + (Math.log(f) - lo) / (hi - lo) * w }

                            var c = curves()
                            var mn = 0, mx = -100
                            for (var i = 0; i < 100; i++) {
                                mn = Math.min(mn, c.cf[i][1], c.dr[i][1])
                                mx = Math.max(mx, c.cf[i][1], c.dr[i][1])
                            }
                            var range = Math.max(mx - mn, 10), pad = range * 0.2
                            var dMin = mn - pad, dMax = mx + pad
                            function yOf(g) { return h - (g - dMin) / (dMax - dMin) * h }

                            // Grid: decades and 10 dB lines
                            ctx.lineWidth = 1
                            ctx.strokeStyle = isMacOS ? MacColors.opacity(MacColors.gray, 0.15) : "rgba(255,255,255,0.07)"
                            ctx.fillStyle = isMacOS ? MacColors.opacity(MacColors.secondaryLabel, 0.6) : "rgba(255,255,255,0.4)"
                            ctx.font = "10px sans-serif"
                            ctx.textAlign = "center"
                            var marks = [[100, "100"], [1000, "1k"], [10000, "10k"]]
                            for (i = 0; i < marks.length; i++) {
                                var gx = xOf(marks[i][0])
                                ctx.beginPath(); ctx.moveTo(gx, 0); ctx.lineTo(gx, h); ctx.stroke()
                                ctx.fillText(marks[i][1], gx, h + 12)
                            }
                            ctx.textAlign = "right"
                            for (var g = Math.ceil(dMin / 10) * 10; g <= dMax; g += 10) {
                                var gy = yOf(g)
                                ctx.beginPath(); ctx.moveTo(left, gy); ctx.lineTo(left + w, gy); ctx.stroke()
                                ctx.fillText(g, left - 5, gy + 3)
                            }

                            if (!active) {
                                ctx.textAlign = "center"
                                ctx.fillStyle = isMacOS ? MacColors.opacity(MacColors.secondaryLabel, 0.5) : "rgba(255,255,255,0.35)"
                                ctx.font = "12px sans-serif"
                                ctx.fillText("Disabled", left + w / 2, h / 2)
                                return
                            }

                            function line(pts, colour) {
                                ctx.strokeStyle = colour
                                ctx.lineWidth = 2
                                ctx.lineJoin = "round"
                                ctx.beginPath()
                                for (var k = 0; k < pts.length; k++) {
                                    var px = xOf(pts[k][0]), py = yOf(pts[k][1])
                                    if (k === 0) ctx.moveTo(px, py); else ctx.lineTo(px, py)
                                }
                                ctx.stroke()
                            }
                            line(c.cf, isMacOS ? MacColors.orange : "#ff9f0a")
                            line(c.dr, isMacOS ? MacColors.accent : "#0a7cff")
                        }
                    }

                    // Legend
                    Rectangle {
                        anchors.top: parent.top
                        anchors.right: parent.right
                        anchors.margins: 14
                        width: legend.width + 12
                        height: legend.height + 10
                        radius: 4
                        color: isMacOS ? MacColors.opacity(MacColors.controlBackground, 0.85) : Qt.rgba(0, 0, 0, 0.35)
                        Column {
                            id: legend
                            anchors.centerIn: parent
                            spacing: 3
                            Row { spacing: 6; Rectangle { width: 12; height: 2; radius: 1; color: isMacOS ? MacColors.accent : "#0a7cff"; anchors.verticalCenter: parent.verticalCenter }
                                  Text { text: "Direct"; font.pixelSize: 11; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.85) } }
                            Row { spacing: 6; Rectangle { width: 12; height: 2; radius: 1; color: isMacOS ? MacColors.orange : "#ff9f0a"; anchors.verticalCenter: parent.verticalCenter }
                                  Text { text: "Crossfeed"; font.pixelSize: 11; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.85) } }
                        }
                    }
                }

                Divider {}
                SectionLabel { text: "PRESET" }

                // Radio list
                Column {
                    width: parent.width
                    spacing: 2
                    Repeater {
                        model: win.presets
                        Rectangle {
                            readonly property bool isCurrent: bridge.crossfeedPreset === index
                            width: parent.width
                            height: 38
                            radius: 7
                            color: isMacOS ? "transparent" : presetMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.06) : "transparent"

                            Rectangle {
                                id: radio
                                x: 8
                                anchors.verticalCenter: parent.verticalCenter
                                width: 16; height: 16; radius: 8
                                color: isMacOS ? (parent.isCurrent ? MacColors.accent : "transparent") : parent.isCurrent ? "#0a7cff" : "transparent"
                                border.width: parent.isCurrent ? 0 : 1.5
                                border.color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.35)
                                Rectangle {
                                    visible: parent.parent.isCurrent
                                    anchors.centerIn: parent
                                    width: 6; height: 6; radius: 3
                                    color: "white"
                                }
                            }
                            Column {
                                anchors.left: radio.right
                                anchors.leftMargin: 10
                                anchors.right: parent.right
                                anchors.verticalCenter: parent.verticalCenter
                                spacing: 1
                                Text {
                                    text: modelData.name
                                    font.pixelSize: 13
                                    font.weight: parent.parent.isCurrent ? Font.DemiBold : Font.Normal
                                    color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9)
                                }
                                Text {
                                    width: parent.width
                                    elide: Text.ElideRight
                                    text: modelData.detail
                                    font.pixelSize: 11
                                    color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
                                }
                            }
                            MouseArea {
                                id: presetMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: bridge.setCrossfeedPreset(index)
                            }
                        }
                    }
                }
            }

            Rectangle { width: 1; height: columns.height; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.08) }

            // ── Right: parameters, ITD, output pairs ──
            Column {
                width: columns.colWidth
                spacing: 12

                SectionLabel { text: "PARAMETERS" }

                // Dimmed unless Custom; editing switches to Custom
                Column {
                    objectName: "crossfeedParams"
                    width: parent.width
                    spacing: 12
                    opacity: isMacOS ? (win.isCustom ? 1.0 : 0.5) : win.isCustom ? 1.0 : 0.55

                    ParamRow {
                        id: freqRow
                        objectName: "crossfeedFreqRow"
                        label: "Cutoff Frequency"; unit: "Hz"; from: 500; to: 2000; stepSize: 10; decimals: 0
                        value: win.shownFreq
                        caption: "Simulates head shadow lowpass cutoff. Lower = more bass crossfeed. Typical: 650–700 Hz."
                        onLiveChanged: bridge.setCrossfeedFreq(v, true)
                        onCommitted: {
                            bridge.setCrossfeedFreq(v)
                            // Leaving a preset: its other value carries over
                            if (!win.isCustom) { bridge.setCrossfeedFeed(win.shownFeed); bridge.setCrossfeedPreset(3) }
                        }
                    }
                    Divider {}
                    ParamRow {
                        id: feedRow
                        objectName: "crossfeedFeedRow"
                        label: "Feed Level"; unit: "dB"; from: 0; to: 15; stepSize: 0.1; decimals: 1
                        value: win.shownFeed
                        caption: "Crossfeed attenuation below the direct signal. Higher = more crossfeed. Typical: 4.5–9.5 dB."
                        onLiveChanged: bridge.setCrossfeedFeed(v, true)
                        onCommitted: {
                            bridge.setCrossfeedFeed(v)
                            if (!win.isCustom) { bridge.setCrossfeedFreq(win.shownFreq); bridge.setCrossfeedPreset(3) }
                        }
                    }
                }
                Divider {}

                Item {
                    width: parent.width
                    height: Math.max(34, itdText.height)
                    Column {
                        id: itdText
                        anchors.left: parent.left
                        anchors.right: itdSwitch.left
                        anchors.rightMargin: 10
                        anchors.verticalCenter: parent.verticalCenter
                        spacing: 2
                        Text { text: "Interaural Time Delay"; font.pixelSize: 13; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9) }
                        Text { width: parent.width; wrapMode: Text.WordWrap; text: "Simulates a ~220 µs path difference via an all-pass filter"; font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5) }
                    }
                    ToggleSwitch {
                        id: itdSwitch
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        checked: bridge.crossfeedITD
                        onToggled: bridge.setCrossfeedITD(checked)
                    }
                }
                Divider {}

                Item {
                    width: parent.width
                    height: 20
                    SectionLabel { text: "OUTPUT PAIRS"; anchors.verticalCenter: parent.verticalCenter }
                    Item {
                        id: pairLink
                        anchors.right: parent.right
                        anchors.verticalCenter: parent.verticalCenter
                        width: pairRow.width
                        height: pairRow.height
                        Row {
                            id: pairRow
                            spacing: 4
                            Text {
                                text: "Presets"
                                font.pixelSize: 12
                                color: isMacOS ? MacColors.label : pairMouse.containsMouse ? "white" : Qt.rgba(1, 1, 1, 0.75)
                            }
                            Icon { name: "chev-down"; size: 11; color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.6); anchors.verticalCenter: parent.verticalCenter }
                        }
                        MouseArea {
                            id: pairMouse
                            anchors.fill: parent
                            hoverEnabled: true
                            cursorShape: Qt.PointingHandCursor
                            onClicked: pairMenu.openAt(pairLink, 0, pairLink.height + 4)
                        }
                    }
                }

                Text {
                    width: parent.width
                    wrapMode: Text.WordWrap
                    font.pixelSize: 11
                    color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
                    text: "Crossfeed only the stereo output pairs feeding headphones. Speaker pairs stay bit-accurate. The mono sub is never crossfed."
                }

                // ── Pair map ──
                Column {
                    objectName: "pairMapControls"
                    visible: win.pairMapShown
                    width: parent.width
                    spacing: 8

                    Item {
                        width: parent.width
                        height: 20
                        Text {
                            anchors.verticalCenter: parent.verticalCenter
                            text: "Outputs"
                            font.pixelSize: 13
                            color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9)
                        }
                        MenuLink {
                            anchors.right: parent.right
                            anchors.verticalCenter: parent.verticalCenter
                            menu: pairMapMenu
                            text: ext.choice === "k340" ? win.pairChoices[1].text
                                : ext.choice === "default" ? win.pairChoices[0].text : "Custom"
                        }
                    }

                    Text {
                        width: parent.width
                        wrapMode: Text.WordWrap
                        font.pixelSize: 11
                        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
                        text: "Which two outputs each pair crossfeeds. An output can belong to one pair only."
                    }

                    // After the K340 preset, on a tap only: the factory mask
                    // crossfeeds pair 1 alone, the woofers
                    Column {
                        objectName: "k340MaskOffer"
                        visible: win.k340MaskOffered
                        width: parent.width
                        spacing: 6
                        Text {
                            width: parent.width
                            wrapMode: Text.WordWrap
                            font.pixelSize: 11
                            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
                            text: "Crossfeed the woofers and the tweeters?"
                        }
                        Row {
                            spacing: 8
                            AppButton { text: "Not Now"; onClicked: win.offerK340Mask = false }
                            AppButton { text: "Enable Pairs 1 and 2"; onClicked: win.enableK340Mask() }
                        }
                    }

                    // The custom editor
                    Rectangle {
                        objectName: "pairMapEditor"
                        visible: win.editingPairs
                        width: parent.width
                        height: editor.height + 16
                        radius: 6
                        color: isMacOS ? MacColors.opacity(MacColors.secondaryLabel, 0.08) : Qt.rgba(1, 1, 1, 0.04)
                        Column {
                            id: editor
                            x: 8
                            y: 8
                            width: parent.width - 16
                            spacing: 6
                            Repeater {
                                model: win.editingPairs ? win.pairDraft.length : 0
                                Row {
                                    readonly property int pair: index
                                    spacing: 6
                                    Text {
                                        width: 46
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: "Pair " + (parent.pair + 1)
                                        font.pixelSize: 12
                                        color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.9)
                                    }
                                    StyledComboBox {
                                        objectName: "pairLeft" + parent.pair
                                        width: 92
                                        model: win.outputLabels
                                        currentIndex: win.pairDraft[parent.pair].left
                                        displayText: "Out " + (currentIndex + 1)
                                        onActivated: win.setDraftOutput(parent.pair, "left", index)
                                    }
                                    Text {
                                        anchors.verticalCenter: parent.verticalCenter
                                        text: "↔"
                                        font.pixelSize: 12
                                        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
                                    }
                                    StyledComboBox {
                                        objectName: "pairRight" + parent.pair
                                        width: 92
                                        model: win.outputLabels
                                        currentIndex: win.pairDraft[parent.pair].right
                                        displayText: "Out " + (currentIndex + 1)
                                        onActivated: win.setDraftOutput(parent.pair, "right", index)
                                    }
                                }
                            }
                            Text {
                                objectName: "pairMapDraftWarning"
                                visible: !win.draftValid
                                width: parent.width
                                wrapMode: Text.WordWrap
                                font.pixelSize: 11
                                color: isMacOS ? MacColors.orange : "#ff9f0a"
                                text: "Each pair needs two different outputs, and no output can be in two pairs."
                            }
                            Row {
                                anchors.right: parent.right
                                spacing: 8
                                AppButton { text: "Cancel"; onClicked: win.cancelDraft() }
                                AppButton {
                                    objectName: "pairMapApply"
                                    text: "Apply"
                                    primary: true
                                    enabled: win.draftValid && !ext.busy
                                    onClicked: win.applyDraft()
                                }
                            }
                        }
                    }

                    Text {
                        objectName: "pairMapError"
                        visible: text !== ""
                        width: parent.width
                        wrapMode: Text.WordWrap
                        font.pixelSize: 11
                        color: isMacOS ? MacColors.orange : "#ff9f0a"
                        text: ext.lastError
                    }

                    // One chip per pair, named by the outputs it processes
                    Grid {
                        id: pairChips
                        objectName: "pairMapChips"
                        width: parent.width
                        columns: 2
                        spacing: 6
                        Repeater {
                            model: win.pairMapShown ? ext.pairs : []
                            Rectangle {
                                readonly property bool isOn: (bridge.crossfeedOutputPairMask >> index) & 1
                                width: (pairChips.width - pairChips.spacing) / 2
                                height: 26
                                radius: 6
                                color: isMacOS ? (isOn ? MacColors.accent : MacColors.opacity(MacColors.secondaryLabel, 0.12)) : isOn ? "#0a7cff" : pairChipMouse.containsMouse ? Qt.rgba(1, 1, 1, 0.12) : Qt.rgba(1, 1, 1, 0.07)
                                border.color: isMacOS ? (isOn ? "transparent" : MacColors.opacity(MacColors.label, 0.08)) : isOn ? "transparent" : Qt.rgba(1, 1, 1, 0.12)
                                Text {
                                    anchors.centerIn: parent
                                    text: (index + 1) + ": Out " + (modelData.left + 1) + " / Out " + (modelData.right + 1)
                                    font.pixelSize: 12
                                    font.weight: Font.DemiBold
                                    color: isMacOS ? (parent.isOn ? "white" : MacColors.opacity(MacColors.label, 0.6)) : parent.isOn ? "white" : Qt.rgba(1, 1, 1, 0.5)
                                }
                                MouseArea {
                                    id: pairChipMouse
                                    anchors.fill: parent
                                    hoverEnabled: true
                                    cursorShape: Qt.PointingHandCursor
                                    onClicked: bridge.setCrossfeedOutputPairMask(bridge.crossfeedOutputPairMask ^ (1 << index))
                                    ToolTip.visible: containsMouse
                                    ToolTip.delay: 500
                                    ToolTip.text: win.outputName(modelData.left) + " / " + win.outputName(modelData.right)
                                }
                            }
                        }
                    }
                }

                ChannelChips {
                    objectName: "plainPairChips"
                    visible: !win.pairMapShown
                    width: parent.width
                    count: win.pairCount
                    mask: bridge.crossfeedOutputPairMask
                    names: win.pairNames()
                    onMaskEdited: bridge.setCrossfeedOutputPairMask(mask)
                }
            }
        }
    }

    ActionMenu {
        id: pairMapMenu
        objectName: "pairMapMenu"
        parent: Overlay.overlay
        items: win.pairChoices
        onTriggered: win.pickPairChoice(key)
    }

    ActionMenu {
        id: pairMenu
        parent: Overlay.overlay
        items: [
            { key: "all", text: "All Pairs" },
            { key: "first", text: "Pair 1 Only (Headphones)" },
            { key: "none", text: "None" }
        ]
        onTriggered: {
            if (key === "all") bridge.setCrossfeedOutputPairMask((1 << win.pairCount) - 1)
            else if (key === "first") bridge.setCrossfeedOutputPairMask(1)
            else if (key === "none") bridge.setCrossfeedOutputPairMask(0)
        }
    }
}
