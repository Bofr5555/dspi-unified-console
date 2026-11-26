import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP UB1 -- (UXP B1, owner D5) "From Speaker Layout…": the crossovers and
// the routing the confirmed speaker layout asks for, set when the person
// presses Apply, as one undo step (LayoutSetupController). One row per
// speaker: each transition between two of its ways (woofer ◂▸ tweeter) with
// its frequency and slope; a speaker shaped as an earlier one ("Right") is
// "Same as Left" until unticked. A tweeter's high pass is offered at its floor
// at the lowest and refused below it (the notice says so, Apply waits). The
// routing part says which input feeds which output. Which parts start ticked
// is the opener's: the Crossover page crossovers, Routing the routing, the
// Speakers page both.
//
//     CrossoverFromLayout { id: fromLayout; routingWanted: true }
//     fromLayout.open()
Dialog {
    id: sheet
    objectName: "CrossoverFromLayout"

    property bool crossoversWanted: true
    property bool routingWanted: false
    // What the controller offers (read on open), and the person's copy of its rows
    property var plan: ({})
    property var rows: []
    property bool doCrossovers: false
    property bool doRouting: false
    property string refusal: ""
    readonly property bool hasCrossovers: (plan.speakers || []).length > 0
    readonly property bool hasRoutes: (plan.routes || []).length > 0
    readonly property string blocker: plan.blocker || ""
    // Why Apply waits, in the controller's words ("" when it may run)
    readonly property string problem: !doCrossovers || blocker.length ? "" : LayoutSetupController.check(values())
    // -- FIX016 -- (F5) routing that would feed a tweeter or mid driver with no high pass
    readonly property string routeProblem: !doRouting || !hasRoutes || blocker.length ? ""
        : (LayoutSetupController.revision, LayoutSetupController.routeRefusal(doCrossovers && hasCrossovers))
    readonly property bool canApply: blocker.length === 0 && problem.length === 0 && routeProblem.length === 0
                                     && ((doCrossovers && hasCrossovers) || (doRouting && hasRoutes))
    // The slopes the firmware takes, as the Crossover page lists them
    readonly property var slopes: {
        const list = []
        const families = ChannelController.crossoverFamilies()
        const brief = ["", "LR", "BW", "BS"]
        for (let f = 1; f < families.length; ++f)
            for (const order of ChannelController.ordersOf(f))
                list.push({ family: f, order: order,
                            label: qsTr("%1 %2 dB/oct").arg(families[f]).arg(order * 6),
                            //: A slope's short form, explained by the Crossover page's legend ("LR 24")
                            brief: qsTr("%1 %2").arg(brief[f]).arg(order * 6) })
        return list
    }

    function reload() {
        plan = LayoutSetupController.plan()
        rows = (plan.speakers || []).map(r => Object.assign({}, r, {
            same: r.sameAs >= 0,
            transitions: r.transitions.map(t => Object.assign({}, t))
        }))
        doCrossovers = crossoversWanted && hasCrossovers
        doRouting = routingWanted && hasRoutes
        refusal = ""
    }
    // The values Apply sends: a "same as" row takes its model's
    function values() {
        return rows.map(r => ({ transitions: (r.same && r.sameAs >= 0 ? rows[r.sameAs] : r).transitions
                                    .map(t => ({ hz: t.hz, family: t.family, order: t.order })) }))
    }
    function edit(row, index, change) {
        const next = rows.slice()
        const r = Object.assign({}, next[row])
        if (index < 0) Object.assign(r, change)
        else {
            r.transitions = r.transitions.slice()
            r.transitions[index] = Object.assign({}, r.transitions[index], change)
        }
        next[row] = r
        rows = next
        refusal = ""
    }
    function slopeIndex(t) { return slopes.findIndex(s => s.family === t.family && s.order === t.order) }
    function apply() {
        refusal = LayoutSetupController.apply(values(), doCrossovers, doRouting)
        if (refusal.length === 0) close()
    }

    onAboutToShow: reload()
    // The layout or the device changed while it is open: what it offers again
    Connections {
        target: sheet.visible ? LayoutSetupController : null
        function onChanged() {
            const now = LayoutSetupController.plan()
            if (JSON.stringify(now.speakers) !== JSON.stringify(sheet.plan.speakers)
                || now.blocker !== sheet.plan.blocker || JSON.stringify(now.routes) !== JSON.stringify(sheet.plan.routes))
                sheet.reload()
        }
    }
    Connections {   // another device becoming active: its answer would go there
        target: sheet.visible ? DeviceRegistry : null
        function onActiveLeaving() { sheet.close() }
    }

    title: qsTr("From Speaker Layout")
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    readonly property real hostWidth: parent ? parent.width : 640
    readonly property real hostHeight: parent ? parent.height : 480
    width: Math.min(620, hostWidth - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: Math.max(Theme.spacingLarge, (hostHeight - height) / 3)
    padding: Theme.spacingLarge
    topPadding: Theme.spacingSmall

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: FloatingBackground { Accessible.name: sheet.title }

    header: Item {
        implicitHeight: titleText.implicitHeight
        Text {
            id: titleText
            width: parent.width
            leftPadding: Theme.spacingLarge
            rightPadding: Theme.spacingLarge
            topPadding: Theme.spacingLarge
            textFormat: Text.PlainText
            text: sheet.title
            font: Theme.fontTitle
            color: Theme.text
            elide: Text.ElideRight
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
    }

    contentItem: Flickable {
        id: flick
        objectName: "CrossoverFromLayout.scroll"
        implicitHeight: Math.min(body.implicitHeight, Math.max(160, sheet.hostHeight * 0.75 - 120))
        contentWidth: width
        contentHeight: body.implicitHeight
        boundsBehavior: Flickable.StopAtBounds
        clip: true
        ScrollBar.vertical: AppScrollBar {}

        ColumnLayout {
            id: body
            width: flick.width
            spacing: Theme.spacingSmall

            Text {
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: qsTr("Starting crossovers and routes for the speakers you confirmed. Nothing changes until you press Apply.")
                font: Theme.fontBody
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            NoticeRow {
                objectName: "CrossoverFromLayout.blocker"
                visible: sheet.blocker.length > 0
                Layout.fillWidth: true
                kind: "info"
                text: sheet.blocker
                actionText: sheet.plan.blockerKind === "layout" ? qsTr("Open Speakers…") : ""
                onAction: {
                    sheet.close()
                    SpeakersController.openSettings()
                }
            }

            // ---- Crossovers ----
            AppCheckBox {
                objectName: "CrossoverFromLayout.crossovers"
                visible: sheet.blocker.length === 0
                Layout.topMargin: Theme.spacingSmall
                text: qsTr("Crossovers")
                font: Theme.fontHeading
                enabled: sheet.hasCrossovers
                checked: sheet.doCrossovers
                onToggled: sheet.doCrossovers = checked
                Accessible.name: qsTr("Set the crossovers")
            }
            Text {
                objectName: "CrossoverFromLayout.noCrossovers"
                visible: sheet.blocker.length === 0 && !sheet.hasCrossovers
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: sheet.plan.crossovers === false ? qsTr("This firmware has no crossovers.")
                      : qsTr("No speaker in the layout has two ways or a subwoofer.")
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            // Counts, not the rows: an edit changes a row's values, never its delegate
            // (a field that commits is still there after its commit)
            Repeater {
                model: sheet.blocker.length === 0 ? sheet.rows.length : 0
                ColumnLayout {
                    id: speaker
                    required property int index
                    readonly property var modelData: sheet.rows[index] || ({ title: "", sameAs: -1, same: false, transitions: [] })
                    readonly property bool same: modelData.same && modelData.sameAs >= 0
                    objectName: "CrossoverFromLayout.speaker." + index
                    Layout.fillWidth: true
                    spacing: 4
                    opacity: sheet.doCrossovers ? 1 : Theme.dim(0.55)
                    enabled: sheet.doCrossovers

                    RowLayout {
                        visible: speaker.modelData.sameAs >= 0
                        Layout.fillWidth: true
                        spacing: Theme.spacing
                        Text {
                            Layout.preferredWidth: 96
                            textFormat: Text.PlainText
                            text: speaker.modelData.title
                            font.family: Theme.fontBody.family
                            font.pixelSize: Theme.fontBody.pixelSize
                            font.weight: Font.DemiBold
                            color: Theme.text
                            elide: Text.ElideRight
                        }
                        AppCheckBox {
                            objectName: speaker.objectName + ".same"
                            Layout.fillWidth: true
                            //: The other side of a pair takes the first side's values ("Same as Left")
                            text: qsTr("Same as %1").arg(speaker.modelData.sameAs >= 0 ? sheet.rows[speaker.modelData.sameAs].title : "")
                            checked: speaker.same
                            onToggled: sheet.edit(speaker.index, -1, { same: checked })
                        }
                    }
                    Repeater {
                        model: speaker.same ? 0 : speaker.modelData.transitions.length
                        RowLayout {
                            id: line
                            required property int index
                            readonly property var modelData: speaker.modelData.transitions[index] || ({ words: "", hz: 0, floorHz: 0 })
                            objectName: speaker.objectName + ".transition." + index
                            Layout.fillWidth: true
                            spacing: Theme.spacing
                            Text {
                                Layout.preferredWidth: 96
                                textFormat: Text.PlainText
                                // The speaker's name on its first line, unless "Same as" says it
                                text: line.index === 0 && speaker.modelData.sameAs < 0 ? speaker.modelData.title : ""
                                font.family: Theme.fontBody.family
                                font.pixelSize: Theme.fontBody.pixelSize
                                font.weight: Font.DemiBold
                                color: Theme.text
                                elide: Text.ElideRight
                            }
                            Text {
                                objectName: line.objectName + ".words"
                                Layout.fillWidth: true
                                textFormat: Text.PlainText
                                text: line.modelData.words
                                font: Theme.fontBody
                                color: Theme.text
                                elide: Text.ElideRight
                            }
                            Text {
                                textFormat: Text.PlainText
                                //: Before a crossover's frequency: "at [2000] Hz"
                                text: qsTr("at")
                                font: Theme.fontSmall
                                color: Theme.textMuted
                            }
                            ChannelValueField {
                                objectName: line.objectName + ".freq"
                                label: qsTr("%1, %2 frequency").arg(speaker.modelData.title).arg(line.modelData.words)
                                unit: qsTr("Hz")
                                fieldWidth: 64
                                Layout.fillWidth: false
                                scrollStep: 10
                                maxDecimals: 0
                                minValue: 10
                                maxValue: 20000
                                value: line.modelData.hz
                                onCommitted: v => sheet.edit(speaker.index, line.index, { hz: Math.round(v) })
                            }
                            AppComboBox {
                                objectName: line.objectName + ".slope"
                                Layout.preferredWidth: 110
                                label: qsTr("%1, %2 slope").arg(speaker.modelData.title).arg(line.modelData.words)
                                model: sheet.slopes.map(s => s.label)
                                currentIndex: sheet.slopeIndex(line.modelData)
                                displayText: currentIndex >= 0 ? sheet.slopes[currentIndex].brief : ""
                                popup.width: Math.max(width, 210)
                                onActivated: index => sheet.edit(speaker.index, line.index,
                                                                 { family: sheet.slopes[index].family, order: sheet.slopes[index].order })
                                Accessible.description: currentText
                            }
                        }
                    }
                }
            }
            Text {
                objectName: "CrossoverFromLayout.floor"
                visible: text.length > 0 && sheet.blocker.length === 0
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: sheet.plan.floorText || ""
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            NoticeRow {
                objectName: "CrossoverFromLayout.problem"
                visible: sheet.problem.length > 0
                Layout.fillWidth: true
                kind: "warning"
                text: sheet.problem
            }
            Repeater {
                model: sheet.blocker.length === 0 ? (sheet.plan.untouched || []) : []
                Text {
                    required property string modelData
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: modelData
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }

            // ---- Routing ----
            AppCheckBox {
                objectName: "CrossoverFromLayout.routing"
                visible: sheet.blocker.length === 0
                Layout.topMargin: Theme.spacing
                text: qsTr("Routing")
                font: Theme.fontHeading
                enabled: sheet.hasRoutes
                checked: sheet.doRouting
                onToggled: sheet.doRouting = checked
                Accessible.name: qsTr("Set the routing")
            }
            Repeater {
                model: sheet.blocker.length === 0 ? (sheet.plan.routes || []) : []
                RowLayout {
                    id: route
                    required property var modelData
                    required property int index
                    objectName: "CrossoverFromLayout.route." + index
                    Layout.fillWidth: true
                    spacing: Theme.spacing
                    opacity: sheet.doRouting ? 1 : Theme.dim(0.55)
                    Text {
                        Layout.preferredWidth: 96
                        textFormat: Text.PlainText
                        text: route.modelData.name
                        font: Theme.fontBody
                        color: Theme.text
                        elide: Text.ElideRight
                    }
                    Text {
                        objectName: route.objectName + ".outputs"
                        Layout.fillWidth: true
                        textFormat: Text.PlainText
                        //: An input and the outputs it feeds: "→ Left woofer · 1, Left tweeter · 2"
                        text: qsTr("→ %1").arg(route.modelData.outputs.join(", "))
                        font: Theme.fontBody
                        color: Theme.text
                        wrapMode: Text.Wrap
                    }
                }
            }
            Repeater {
                model: sheet.blocker.length === 0 ? (sheet.plan.routeNotes || []) : []
                Text {
                    required property string modelData
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: modelData
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
            Text {
                visible: sheet.blocker.length === 0 && sheet.hasRoutes
                Layout.fillWidth: true
                textFormat: Text.PlainText
                text: qsTr("The other outputs keep their routes. A crosspoint that stays keeps its gain and polarity.")
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            // -- FIX016 -- (F5)
            NoticeRow {
                objectName: "CrossoverFromLayout.routeProblem"
                visible: sheet.routeProblem.length > 0
                Layout.fillWidth: true
                kind: "warning"
                text: sheet.routeProblem
            }
            NoticeRow {
                objectName: "CrossoverFromLayout.refusal"
                visible: sheet.refusal.length > 0 && sheet.refusal !== sheet.problem && sheet.refusal !== sheet.routeProblem
                Layout.fillWidth: true
                kind: "error"
                text: sheet.refusal
            }
        }
    }

    footer: Item {
        implicitHeight: buttons.implicitHeight + 2 * Theme.spacing
        Text {
            objectName: "CrossoverFromLayout.oneStep"
            x: Theme.spacingLarge
            width: parent.width - buttons.width - 3 * Theme.spacingLarge
            anchors.verticalCenter: buttons.verticalCenter
            visible: sheet.blocker.length === 0
            textFormat: Text.PlainText
            text: qsTr("One undo step puts it all back.")
            font: Theme.fontSmall
            color: Theme.textMuted
            elide: Text.ElideRight
        }
        Row {
            id: buttons
            x: parent.width - width - Theme.spacingLarge
            y: Theme.spacing
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "CrossoverFromLayout.cancel"
                text: qsTr("Cancel")
                onClicked: sheet.close()
            }
            AppButton {
                objectName: "CrossoverFromLayout.apply"
                text: qsTr("Apply")
                primary: enabled
                enabled: sheet.canApply
                onClicked: sheet.apply()
                Keys.onReturnPressed: if (enabled) sheet.apply()
                Keys.onEnterPressed: if (enabled) sheet.apply()
            }
        }
    }
}
