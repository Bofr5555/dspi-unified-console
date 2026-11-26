import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5a -- The inspector beside the matrix (MODLAY 2): the output whose
// column is selected, or the input whose row is.
// -- WP MX1 -- It follows the grid (DESREV's mockup), in three states and a
// summary:
//   a crosspoint (the grid's current cell)  From → To; Connected, Gain,
//       Invert phase; what else the input feeds (chips); the output in a line
//   an output (its header)  the channel's inspector (crossover, limiter,
//       Linkwitz) with Enabled, Gain, Delay, Mute, Solo (-- WP GAP2 --),
//       Polarity and what feeds it
//   an input (its row)  the channel's inspector (feeds, headphone profile)
//       with its trim
//   nothing  the device's routing in words
// "Open in Equaliser" under the crosspoint's output or the channel.
// -- WP UA1 -- (UXP A1) an output is named as the pages name it, name first and
// number second ("Left tweeter · 2"); "also feeds" lists the outputs the input
// feeds, and "+" connects it to another.
Inspector {
    id: inspector
    objectName: "RoutingInspector"
    property Item host: null

    // The limiter's state, while its section can be seen
    readonly property bool watching: open
    onWatchingChanged: ChannelController.watchLimiter(watching)
    Component.onDestruction: if (watching) ChannelController.watchLimiter(false)
    Binding {
        target: InspectorState
        property: "windowWidth"
        value: inspector.Window.width
        when: inspector.shown && inspector.Window.width > 0
        restoreMode: Binding.RestoreNone
    }

    content: Component {
        ColumnLayout {
            id: body
            spacing: Theme.spacing

            readonly property int ci: MatrixController.currentInput
            readonly property int co: MatrixController.currentOutput
            readonly property bool cross: ci >= 0 && co >= 0
            readonly property int version: MatrixController.model.version
            readonly property bool on: cross && version >= 0 && MatrixController.model.get(ci, co, "connected") === true
            readonly property real gain: cross && version >= 0 ? (MatrixController.model.get(ci, co, "gain") ?? 0) : 0
            readonly property bool inverted: on && version >= 0 && MatrixController.model.get(ci, co, "invert") === true
            readonly property var src: cross && MatrixController.revision >= 0 ? MatrixController.row(ci) : undefined
            readonly property var dst: cross && MatrixController.revision >= 0 ? MatrixController.column(co) : undefined
            readonly property var mirror: cross && MatrixController.linkPairs ? MatrixController.mirrorOf(ci, co) : ({})

            function rowName(r) { const s = MatrixController.revision < 0 ? undefined : MatrixController.row(r); return s ? s.name : "" }
            // -- WP UA1 -- the output's title ("Left tweeter · 2"), else the matrix's label
            function colName(o) {
                const c = MatrixController.revision < 0 ? undefined : MatrixController.column(o)
                return !c ? "" : info.titleOf(ChannelController.outputChannel(o)) || c.label
            }
            function db(v) { return (v > 0 ? "+" : "") + Number(v).toFixed(1) }
            // "3, 5–7": numbers in runs
            function runs(list) {
                const out = []
                for (let k = 0; k < list.length; ++k) {
                    let e = k
                    while (e + 1 < list.length && list[e + 1] === list[e] + 1) ++e
                    out.push(e - k >= 2 ? list[k] + "–" + list[e] : e > k ? list[k] + ", " + list[e] : String(list[k]))
                    k = e
                }
                return out.join(", ")
            }

            ChannelInfo { id: info; channel: SidebarController.selectedChannel }

            component Line: RowLayout {
                property string label
                default property alias controls: holder.data
                Layout.fillWidth: true
                Layout.preferredHeight: 32
                spacing: Theme.spacingSmall
                Text {
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: parent.label
                    font: Theme.fontBody
                    color: Theme.text
                    elide: Text.ElideRight
                }
                RowLayout { id: holder; spacing: 6 }
            }
            // A switch that shows the device's state: a click asks, the state answers
            component StateSwitch: AppSwitch {
                id: sw
                property bool isOn
                signal asked(bool on)
                checked: isOn
                enabled: MatrixController.writable
                onToggled: {
                    asked(checked)
                    checked = Qt.binding(() => sw.isOn)
                }
            }
            component Unit: Text {
                textFormat: Text.PlainText
                font: Theme.fontSmall
                color: Theme.textMuted
            }
            component Note: Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }

            // ---- Nothing chosen: the routing in words ---------------------------
            ColumnLayout {
                objectName: "RoutingInspector.none"
                visible: !body.cross && !info.known
                Layout.fillWidth: true
                Layout.rightMargin: 28   // the close button
                spacing: 6
                readonly property var idle: {
                    const list = []
                    if (body.version < 0) return list
                    for (let o = 0; o < MatrixController.columns; ++o)
                        if (MatrixController.model.sources(o).length === 0) list.push(o + 1)
                    return list
                }
                readonly property int outputsOn: {
                    let n = 0
                    if (MatrixController.revision < 0) return n
                    for (let o = 0; o < MatrixController.columns; ++o) n += MatrixController.column(o).enabled ? 1 : 0
                    return n
                }
                Text {
                    textFormat: Text.PlainText
                    text: qsTr("Routing")
                    font: Theme.fontTitle
                    color: Theme.text
                }
                Note {
                    objectName: "RoutingInspector.summary"
                    font: Theme.fontBody
                    text: {
                        const n = body.version < 0 ? 0 : MatrixController.model.connectedCount()
                        const parts = [n === 1 ? qsTr("1 crosspoint") : qsTr("%1 crosspoints").arg(n),
                                       qsTr("%1 of %2 outputs on").arg(parent.outputsOn).arg(MatrixController.columns)]
                        //: %1 is a list of output numbers ("4, 6, 7")
                        if (parent.idle.length) parts.push(qsTr("Out %1 fed by nothing").arg(body.runs(parent.idle)))
                        return parts.join(" · ")
                    }
                }
                Note { text: qsTr("Choose a cell, an input's row or an output's column in the matrix to see it here.") }
            }

            // ---- A crosspoint -------------------------------------------------------
            RowLayout {
                objectName: "RoutingInspector.title"
                visible: body.cross
                Layout.fillWidth: true
                Layout.rightMargin: 28
                spacing: 6
                Pill {
                    text: qsTr("IN%1").arg(body.ci + 1)
                    tint: ChannelPalette.input(body.ci)
                }
                Text {
                    textFormat: Text.PlainText
                    text: body.src ? body.src.name : ""
                    font: Theme.fontTitle
                    color: Theme.text
                }
                Text {
                    text: "→"
                    font: Theme.fontTitle
                    color: Theme.textMuted
                }
                Pill {
                    text: body.dst ? body.dst.descriptor : ""
                    tint: ChannelPalette.output(body.co, body.dst && body.dst.pdm)
                }
                Text {
                    objectName: "RoutingInspector.title.output"
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: body.dst ? body.colName(body.co) : ""
                    font: Theme.fontTitle
                    color: Theme.text
                    elide: Text.ElideRight
                }
                Accessible.role: Accessible.Heading
                Accessible.name: (body.src ? body.src.name : "") + " → " + (body.dst ? body.colName(body.co) : "")
            }
            InspectorSection {
                objectName: "RoutingInspector.crosspoint"
                visible: body.cross
                Layout.fillWidth: true
                title: qsTr("Crosspoint")
                summary: body.on ? qsTr("%1 dB").arg(body.db(body.gain)) + (body.inverted ? " · ø" : "") : qsTr("off")
                Line {
                    label: qsTr("Connected")
                    StateSwitch {
                        objectName: "RoutingInspector.connected"
                        isOn: body.on
                        Accessible.name: qsTr("Connected")
                        onAsked: on => MatrixController.setConnected(body.ci, body.co, on)
                    }
                }
                Line {
                    label: qsTr("Gain")
                    visible: body.on
                    CustomSlider {
                        objectName: "RoutingInspector.gainSlider"
                        Layout.preferredWidth: 110
                        from: -60
                        to: 12
                        stepSize: 0.5
                        value: body.gain
                        resetValue: 0
                        label: qsTr("Gain")
                        enabled: MatrixController.writable
                        onValueMoved: v => MatrixController.setGain(body.ci, body.co, v)
                        onCommitted: v => MatrixController.setGain(body.ci, body.co, v)
                    }
                    MatrixField {
                        objectName: "RoutingInspector.gain"
                        Layout.preferredWidth: 56
                        kind: "gain"
                        showUnit: false
                        horizontalAlignment: TextInput.AlignRight
                        value: body.gain
                        label: qsTr("Crosspoint gain")
                        enabled: MatrixController.writable
                        onValueCommitted: v => MatrixController.setGain(body.ci, body.co, v)
                    }
                    Unit { text: qsTr("dB") }
                }
                Line {
                    label: qsTr("Invert phase")
                    visible: body.on
                    StateSwitch {
                        objectName: "RoutingInspector.invert"
                        isOn: body.inverted
                        Accessible.name: qsTr("Invert phase")
                        onAsked: on => MatrixController.setInvert(body.ci, body.co, on)
                    }
                }
                Note {
                    visible: !body.on
                    text: qsTr("Not connected. Space connects it at 0 dB; Enter or a digit types its gain.")
                }
                Note {
                    objectName: "RoutingInspector.mirror"
                    visible: body.mirror.input !== undefined
                    //: %1 → %2 is the crosspoint a change is mirrored to ("R → Out 2")
                    text: body.mirror.input !== undefined
                          ? qsTr("Link L/R pairs: changes here are made on %1 → %2 too.").arg(body.rowName(body.mirror.input)).arg(body.colName(body.mirror.output))
                          : ""
                }
            }
            InspectorSection {
                id: alsoFeeds
                objectName: "RoutingInspector.alsoFeeds"
                visible: body.cross
                Layout.fillWidth: true
                //: %1 is an input's name
                title: qsTr("%1 also feeds").arg(body.src ? body.src.name : "")
                // -- WP UA1 -- the other outputs it feeds (a click disconnects one), and the rest for "+"
                readonly property var feeds: body.version < 0 || !body.cross ? [] : MatrixController.model.feeds(body.ci)
                readonly property var others: {
                    const list = []
                    for (let o = 0; o < MatrixController.columns; ++o) if (o !== body.co && feeds.indexOf(o) >= 0) list.push(o)
                    return list
                }
                readonly property var rest: {
                    const list = []
                    for (let o = 0; o < MatrixController.columns; ++o) if (o !== body.co && feeds.indexOf(o) < 0) list.push(o)
                    return list
                }
                summary: others.length === 0 ? qsTr("nothing else") : ""
                RowLayout {
                    Layout.fillWidth: true
                    spacing: 6
                    ChipGroup {
                        objectName: "RoutingInspector.alsoFeeds.chips"
                        Layout.fillWidth: true
                        Layout.maximumWidth: implicitWidth   // "+" right after the last chip
                        visible: alsoFeeds.others.length > 0
                        allNone: false
                        enabled: MatrixController.writable
                        model: alsoFeeds.others.map(o => ({
                            text: body.colName(o),
                            //: %1 is an output's name; a click disconnects it
                            toolTip: qsTr("Disconnect from %1").arg(body.colName(o))
                        }))
                        selected: alsoFeeds.others.map((o, k) => k)
                        onToggled: (index, on) => MatrixController.setConnected(body.ci, alsoFeeds.others[index], on)
                    }
                    IconButton {
                        id: addFeed
                        objectName: "RoutingInspector.alsoFeeds.add"
                        Layout.alignment: Qt.AlignTop
                        visible: alsoFeeds.rest.length > 0
                        enabled: MatrixController.writable
                        iconName: "plus"
                        //: %1 is an input's name
                        toolTip: qsTr("Connect %1 to another output").arg(body.src ? body.src.name : "")
                        onClicked: addMenu.popup(addFeed, 0, addFeed.height + 2)
                        AppMenu {
                            id: addMenu
                            objectName: "RoutingInspector.alsoFeeds.menu"
                            Instantiator {
                                model: alsoFeeds.rest
                                delegate: AppMenuItem {
                                    required property var modelData
                                    objectName: "RoutingInspector.alsoFeeds.menu." + modelData
                                    text: body.colName(modelData)
                                    onTriggered: MatrixController.setConnected(body.ci, modelData, true)
                                }
                                onObjectAdded: (index, object) => addMenu.insertItem(index, object)
                                onObjectRemoved: (index, object) => addMenu.removeItem(object)
                            }
                        }
                    }
                    Item { Layout.fillWidth: true }
                }
            }
            InspectorSection {
                objectName: "RoutingInspector.outputLine"
                visible: body.cross
                Layout.fillWidth: true
                title: body.dst ? body.colName(body.co) : ""
                summary: body.dst ? [body.dst.enabled ? qsTr("on") : qsTr("off"), qsTr("%1 dB").arg(body.db(body.dst.gain)),
                                     qsTr("%1 ms").arg(Number(body.dst.delay).toFixed(2))].join(" · ") : ""
                Note {
                    text: {
                        if (!body.cross || ChannelController.revision < 0) return ""
                        const p = ChannelController.outputPolarity(body.co)
                        const out = ChannelController.output(body.co)
                        return [p === 1 ? qsTr("Polarity inverted") : p === 2 ? qsTr("Polarity mixed") : qsTr("Polarity normal"),
                                out.limiter ? qsTr("Limiter on") : qsTr("Limiter off")].join(" · ")
                    }
                }
            }

            // ---- An output's column or an input's row --------------------------------
            ChannelInspector {
                Layout.fillWidth: true
                visible: !body.cross && info.known
                channel: SidebarController.selectedChannel
                host: inspector.host
                routingLink: false
            }
            InspectorSection {
                id: outSection
                objectName: "RoutingInspector.output"
                visible: !body.cross && info.isOutput
                Layout.fillWidth: true
                title: qsTr("Output")
                readonly property int o: info.number
                readonly property var col: MatrixController.revision < 0 || o < 0 ? undefined : MatrixController.column(o)
                Line {
                    label: qsTr("Enabled")
                    StateSwitch {
                        objectName: "RoutingInspector.enabled"
                        isOn: outSection.col ? outSection.col.enabled === true : false
                        Accessible.name: qsTr("Enabled")
                        onAsked: MatrixController.toggleEnable(info.number)
                    }
                }
                Line {
                    label: qsTr("Gain")
                    MatrixField {
                        objectName: "RoutingInspector.outputGain"
                        Layout.preferredWidth: 72
                        kind: "gain"
                        maxValue: 20
                        showUnit: false
                        horizontalAlignment: TextInput.AlignRight
                        value: outSection.col ? outSection.col.gain : 0
                        label: qsTr("Output gain")
                        enabled: MatrixController.writable
                        onValueCommitted: v => MatrixController.setOutputGain(info.number, v)
                    }
                    Unit { text: qsTr("dB") }
                }
                Line {
                    label: qsTr("Delay")
                    MatrixField {
                        objectName: "RoutingInspector.outputDelay"
                        Layout.preferredWidth: 72
                        kind: "delay"
                        maxValue: MatrixController.maxDelayMs
                        showUnit: false
                        horizontalAlignment: TextInput.AlignRight
                        value: outSection.col ? outSection.col.delay : 0
                        label: qsTr("Output delay")
                        enabled: MatrixController.writable
                        onValueCommitted: v => MatrixController.setOutputDelay(info.number, v)
                    }
                    Unit { text: qsTr("ms") }
                }
                Line {
                    label: qsTr("Mute")
                    StateSwitch {
                        objectName: "RoutingInspector.mute"
                        isOn: outSection.col ? outSection.col.muted === true : false
                        Accessible.name: qsTr("Mute")
                        onAsked: MatrixController.toggleMute(info.number)
                    }
                }
                // -- WP GAP2 -- only this output plays, until it is switched off (not saved)
                Line {
                    label: qsTr("Solo")
                    StateSwitch {
                        objectName: "RoutingInspector.solo"
                        isOn: SoloController.output >= 0 && SoloController.output === info.number
                        enabled: SoloController.available && (outSection.col ? outSection.col.enabled === true : false)
                        Accessible.name: qsTr("Solo")
                        onAsked: SoloController.toggle(info.number)
                    }
                }
                Line {
                    label: qsTr("Polarity")
                    ChipGroup {
                        objectName: "RoutingInspector.polarity"
                        exclusive: true
                        enabled: MatrixController.writable
                        model: [qsTr("Normal"), qsTr("Inverted")]
                        current: {
                            const p = ChannelController.revision < 0 || info.number < 0 ? -1 : ChannelController.outputPolarity(info.number)
                            return p === 0 ? 0 : p === 1 ? 1 : -1
                        }
                        onChosen: index => ChannelController.setOutputPolarity(info.number, index === 1)
                    }
                }
            }
            InspectorSection {
                id: fedBySection
                objectName: "RoutingInspector.fedBy"
                visible: !body.cross && info.isOutput
                Layout.fillWidth: true
                title: qsTr("Fed by")
                readonly property var sources: body.version < 0 || info.number < 0 ? [] : MatrixController.model.sources(info.number)
                summary: sources.length === 1 ? qsTr("1 input") : qsTr("%1 inputs").arg(sources.length)
                ChipGroup {
                    objectName: "RoutingInspector.fedBy.chips"
                    Layout.fillWidth: true
                    allNone: false
                    enabled: MatrixController.writable
                    model: {
                        const names = []
                        for (let r = 0; r < MatrixController.rows; ++r) names.push(body.rowName(r))
                        return names
                    }
                    selected: fedBySection.sources
                    onToggled: (index, on) => MatrixController.setConnected(index, info.number, on)
                }
            }
            InspectorSection {
                objectName: "RoutingInspector.input"
                visible: !body.cross && info.known && !info.isOutput
                Layout.fillWidth: true
                title: qsTr("Input")
                Line {
                    label: qsTr("Trim (preamp)")
                    MatrixField {
                        objectName: "RoutingInspector.trim"
                        Layout.preferredWidth: 72
                        kind: "gain"
                        showUnit: false
                        horizontalAlignment: TextInput.AlignRight
                        value: MatrixController.revision < 0 || info.number < 0 ? 0 : MatrixController.row(info.number).trim ?? 0
                        label: qsTr("Input trim (preamp)")
                        enabled: MatrixController.writable
                        onValueCommitted: v => MatrixController.setTrim(info.number, v)
                    }
                    Unit { text: qsTr("dB") }
                }
            }

            AppButton {
                objectName: "RoutingInspector.equaliser"
                visible: body.cross || info.known
                text: qsTr("Open in Equaliser")
                onClicked: {
                    if (body.cross) {
                        const ch = ChannelController.outputChannel(body.co)
                        MatrixController.setCurrent(-1, -1)
                        SidebarController.select(ch)
                    }
                    inspector.host.go("equaliser", "peq")
                }
            }
        }
    }
}
