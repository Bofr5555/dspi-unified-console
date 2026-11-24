import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP DL6 -- One channel on Modern's Overview (DESREV §5, SHOW finding 9):
// its pill and name, its role, a small response, its level, and chips for what
// is set on it (bands, crossover, what feeds it, delay, gain, mute, polarity).
// An output in the confirmed speaker layout is named after its speaker
// ("Centre woofer"); any other is named as everywhere else ("Out 1"). The card
// is one button: it opens the channel (`chosen`). `entry` is one column of
// DashboardController.cards. The button keeps Classic's Overview name,
// "Dashboard.open.<channel>".
AbstractButton {
    id: card
    property var entry: ({})
    readonly property int channel: entry.channel ?? -1
    signal chosen(int channel)

    ChannelInfo { id: info; channel: card.channel }
    readonly property bool isOutput: info.isOutput
    readonly property int output: isOutput ? info.number : -1

    // ---- The speaker layout, when the person confirmed it ----
    readonly property var layoutRow: SpeakersController.status !== "confirmed" || output < 0
                                     ? ({}) : SpeakersController.rowOf(output)
    readonly property bool trusted: layoutRow.included === true
    readonly property string role: trusted ? layoutRow.role : ""
    readonly property string speaker: trusted ? (layoutRow.speaker || "").trim() : ""
    function roleText(r) {
        return r === "sub" ? qsTr("Subwoofer") : r === "low" ? qsTr("Woofer") : r === "mid" ? qsTr("Mid")
             : r === "high" ? qsTr("Tweeter") : qsTr("Full range")
    }
    function speakerName(s, r) {
        if (s.length === 0) return ""
        //: %1 is a speaker's name from the speaker layout ("Centre"); the output drives its woofer
        if (r === "low") return qsTr("%1 woofer").arg(s)
        //: %1 is a speaker's name from the speaker layout; the output drives its mid driver
        if (r === "mid") return qsTr("%1 mid").arg(s)
        //: %1 is a speaker's name from the speaker layout; the output drives its tweeter
        if (r === "high") return qsTr("%1 tweeter").arg(s)
        //: %1 is a speaker's name from the speaker layout; the output drives its subwoofer
        if (r === "sub") return s.toLowerCase().startsWith("sub") ? s : qsTr("%1 sub").arg(s)
        return s
    }
    readonly property string name: speakerName(speaker, role) || info.label || (entry.heading ?? "")
    readonly property string caption: !info.known ? ""
        : trusted ? qsTr("%1 · %2").arg(roleText(role)).arg(info.label)
        : isOutput ? qsTr("Output") : qsTr("Input")

    // ---- What is set ----
    function labelOf(wire) {
        const row = DeviceModel.revision < 0 ? -1 : DeviceModel.channels.rowOfChannel(wire)
        return row < 0 ? "" : DeviceModel.channels.get(row, "label") || ""
    }
    function hz(f) {
        return f >= 1000 ? qsTr("%1 kHz").arg(Number((f / 1000).toFixed(2))) : qsTr("%1 Hz").arg(Math.round(f))
    }
    function db(v) {
        const t = Number(v).toFixed(1).replace(/\.0$/, "")
        return (v > 0 ? "+" : v < 0 ? "−" : "") + t.replace("-", "") + " " + qsTr("dB")
    }
    // [{ key, text, kind }]: kind "" (set), "muted" (nothing set) or "warning"
    readonly property var chips: {
        const list = []
        if (ChannelController.revision < 0 || channel < 0 || !info.known) return list
        // PEQ bands in use
        let bands = 0
        for (let b = 0; b < 10; ++b) if (ChannelController.band(channel, b, false).type !== 0) ++bands
        list.push({ key: "bands", kind: bands ? "" : "muted",
                    text: bands === 0 ? qsTr("No bands") : bands === 1 ? qsTr("1 band") : qsTr("%1 bands").arg(bands) })
        if (isOutput) {
            // The crossover: the first high pass and the first low pass
            if (ChannelController.crossoverSupported) {
                const brief = ["", "LR", "BW", "BS"]
                const xo = []
                for (const low of [false, true]) {
                    for (let b = 0; b < 4; ++b) {
                        const p = ChannelController.band(channel, b, true)
                        if (p.type === 0 || ChannelController.isLowPass(p.type) !== low) continue
                        const f = ChannelController.familyOf(p.type)
                        //: A crossover in a chip: high or low pass, family and slope, frequency ("HP LR24 80 Hz")
                        xo.push({ key: low ? "lowPass" : "highPass", kind: p.bypass ? "muted" : "",
                                  text: qsTr("%1 %2%3 %4").arg(low ? qsTr("LP") : qsTr("HP")).arg(brief[f] ?? "")
                                          .arg(ChannelController.orderOf(p.type) * 6).arg(hz(p.freq)) })
                        break
                    }
                }
                for (const c of xo) list.push(c)
            }
            // What feeds it
            const from = []
            for (let i = 0; i < ChannelController.numInputs; ++i)
                if (ChannelController.route(i, output).enabled === true) from.push(labelOf(i))
            list.push({ key: "feeds", kind: from.length ? "" : "muted",
                        text: from.length ? qsTr("from %1").arg(from.join(" + ")) : qsTr("No input") })
            const o = ChannelController.output(output)
            if (o.delay > 0) list.push({ key: "delay", kind: "", text: qsTr("%1 ms").arg(DashboardController.delayText(o.delay)) })
            if (o.gain !== 0) list.push({ key: "gain", kind: "", text: db(o.gain) })
            if (ChannelController.outputPolarity(output) === 1) list.push({ key: "inverted", kind: "warning", text: qsTr("Inverted") })
            if (o.muted) list.push({ key: "muted", kind: "warning", text: qsTr("Muted") })
        } else {
            // Where it goes
            const to = []
            for (let o = 0; o < ChannelController.numOutputs; ++o)
                if (ChannelController.route(channel, o).enabled === true) to.push(labelOf(ChannelController.outputChannel(o)))
            list.push({ key: "feeds", kind: to.length ? "" : "muted",
                        text: to.length === 0 ? qsTr("To no output")
                            : to.length <= 3 ? qsTr("to %1").arg(to.join(" + ")) : qsTr("to %1 outputs").arg(to.length) })
            const pre = ChannelController.preamp(info.number)
            if (pre !== 0) list.push({ key: "gain", kind: "", text: qsTr("Preamp %1").arg(db(pre)) })
        }
        return list
    }

    objectName: "Dashboard.open." + channel
    implicitWidth: 260
    implicitHeight: body.implicitHeight + 2 * Theme.spacing
    padding: Theme.spacing
    hoverEnabled: true
    focusPolicy: Qt.TabFocus
    onClicked: card.chosen(card.channel)

    background: Rectangle {
        radius: Theme.radius
        color: card.down ? Theme.controlPressed : card.hovered ? Theme.controlHover : Theme.panel
        border.width: card.visualFocus ? Theme.focusRingWidth : 0
        border.color: Theme.focusRing
    }

    contentItem: ColumnLayout {
        id: body
        spacing: Theme.spacingSmall

        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            Pill {
                visible: info.known
                text: info.descriptor
                tint: info.tint
            }
            Text {
                objectName: "OverviewChannelCard.name." + card.channel
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: card.name
                font: Theme.fontTitle
                color: Theme.text
                elide: Text.ElideRight
            }
        }
        Text {
            objectName: "OverviewChannelCard.role." + card.channel
            Layout.fillWidth: true
            Layout.topMargin: -4
            textFormat: Text.PlainText
            text: card.caption
            font: Theme.fontSmall
            color: Theme.textMuted
            elide: Text.ElideRight
        }

        // The response, small: +12 dB to −24 dB, a line at 0 dB
        Canvas {
            id: response
            objectName: "OverviewChannelCard.response." + card.channel
            readonly property real dbTop: 12
            readonly property real dbBottom: -24
            readonly property var curve: GraphController.curveRevision < 0 || card.channel < 0 ? [] : GraphController.magnitudeCurve(card.channel)
            readonly property color ink: info.tint
            readonly property color grid: Theme.graphGrid
            Layout.fillWidth: true
            Layout.preferredHeight: 48
            onCurveChanged: requestPaint()
            onInkChanged: requestPaint()
            onGridChanged: requestPaint()
            onWidthChanged: requestPaint()
            onPaint: {
                const g = getContext("2d")
                g.reset()
                const yOf = v => Math.max(0, Math.min(height, (dbTop - v) / (dbTop - dbBottom) * height))
                g.strokeStyle = grid
                g.lineWidth = 1
                g.beginPath()
                g.moveTo(0, Math.round(yOf(0)) + 0.5)
                g.lineTo(width, Math.round(yOf(0)) + 0.5)
                g.stroke()
                if (curve.length < 2) return
                const step = width / (curve.length - 1)
                g.beginPath()
                g.moveTo(0, yOf(curve[0]))
                for (let i = 1; i < curve.length; ++i) g.lineTo(i * step, yOf(curve[i]))
                g.strokeStyle = ink
                g.lineWidth = 1.5
                g.stroke()
                g.lineTo(width, height)
                g.lineTo(0, height)
                g.closePath()
                g.fillStyle = Qt.rgba(ink.r, ink.g, ink.b, 0.12)
                g.fill()
            }
            Accessible.ignored: true
        }

        // The level, fed by the meter poll itself
        Meter {
            objectName: "OverviewChannelCard.meter." + card.channel
            Layout.fillWidth: true
            Layout.preferredHeight: 3
            channel: card.channel
            color: Theme.meterLevel
            trackColor: Theme.meterTrack
            clipColor: Theme.danger
            clipping: card.channel >= 0 && card.channel < 32 && ((DashboardController.clipLatched >>> card.channel) & 1) !== 0
            opacity: card.entry.muted === true ? 0.4 : 1
            Accessible.ignored: true
        }

        Flow {
            Layout.fillWidth: true
            Layout.topMargin: 2
            spacing: 6
            Repeater {
                model: card.chips
                Rectangle {
                    id: chip
                    required property var modelData
                    objectName: "OverviewChannelCard.chip." + card.channel + "." + modelData.key
                    property string shownName: modelData.text
                    implicitWidth: chipText.implicitWidth + 16
                    implicitHeight: 20
                    radius: height / 2
                    color: Theme.control
                    Text {
                        id: chipText
                        anchors.centerIn: parent
                        textFormat: Text.PlainText
                        text: chip.modelData.text
                        font: Theme.fontSmall
                        color: chip.modelData.kind === "muted" ? Theme.textMuted
                             : chip.modelData.kind === "warning" ? Theme.warning : Theme.text
                    }
                }
            }
        }
    }

    Accessible.role: Accessible.Button
    //: %1 is a channel's name, %2 what is set on it ("1 band, from L")
    Accessible.name: qsTr("%1, %2").arg(name).arg(chips.map(c => c.text).join(", "))
    //: %1 is a channel's name
    Accessible.description: qsTr("Opens %1 in the Equaliser").arg(name)
    Accessible.onPressAction: card.clicked()
}
