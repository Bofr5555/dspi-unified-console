import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP UA4 -- (UXP A4) The Overview's "Set up this DSPi" list: while anything
// is missing on the active device, one row a step with whether it is done and
// its one action (speakers confirmed, inputs routed, crossovers set where the
// layout has drivers, measured, saved to the device). It goes when every row
// is done, or for good when the person closes it (per device,
// OnboardingController.closeChecklist). Not on a device this app may not
// change (read-only firmware, the installer lock).
ColumnLayout {
    id: list
    objectName: "OverviewChecklist"
    property Item host: null

    readonly property string serial: DeviceModel.serial
    readonly property var sc: SpeakersController
    readonly property bool closed: OnboardingController.checklistRevision >= 0 && OnboardingController.checklistClosed(serial)

    // The layout's outputs and roles: the confirmed layout's, else the draft's
    readonly property var layoutRows: (sc.rows || []).filter(r => r.included)
    readonly property bool hasDrivers: layoutRows.some(r => r.role !== "full")

    readonly property bool speakersDone: sc.status === "confirmed"
    readonly property bool routedDone: {
        if (ChannelController.revision < 0) return false
        for (let i = 0; i < ChannelController.numInputs; ++i)
            for (let o = 0; o < ChannelController.numOutputs; ++o)
                if (ChannelController.route(i, o).enabled === true) return true
        return false
    }
    // The outputs of the layout that drive a woofer, mid, tweeter or sub with no crossover filter
    readonly property var withoutCrossover: {
        const names = []
        if (ChannelController.revision < 0 || DeviceModel.revision < 0) return names
        for (const r of layoutRows) {
            if (r.role === "full") continue
            const wire = ChannelController.outputChannel(r.output)
            let set = false
            for (let b = 0; b < 4 && !set; ++b) set = ChannelController.band(wire, b, true).type !== 0
            if (!set) names.push(r.label)
        }
        return names
    }
    readonly property bool crossoverShown: hasDrivers && ChannelController.crossoverSupported
    readonly property bool measuredNow: MeasureController.hasResult
    onMeasuredNowChanged: if (measuredNow && serial.length) OnboardingController.noteMeasured(serial)
    Component.onCompleted: if (measuredNow && serial.length) OnboardingController.noteMeasured(serial)
    readonly property bool measuredDone: measuredNow
                                         || (OnboardingController.checklistRevision >= 0 && OnboardingController.measuredSeen(serial))
    readonly property bool savedDone: !SidebarController.unsaved

    function roleCount() {
        const speakers = {}
        for (const r of layoutRows) speakers[r.group] = true
        return Object.keys(speakers).length
    }
    // [{ key, done, title, detail, action, run }]
    readonly property var rows: {
        const out = []
        out.push({ key: "speakers", done: speakersDone, title: qsTr("Speakers confirmed"),
                   //: The setup list's speaker row when done: %1 outputs, %2 speakers (groups) of the layout
                   detail: speakersDone ? qsTr("%1 outputs, %2 speakers").arg(layoutRows.length).arg(roleCount())
                                        : qsTr("Which output drives what: measuring and AI agents need it"),
                   action: qsTr("Set Up Speakers…"), run: () => SettingsController.showPage("speakers") })
        out.push({ key: "routing", done: routedDone, title: qsTr("Inputs routed"),
                   detail: routedDone ? qsTr("Audio reaches the outputs") : qsTr("No input reaches an output yet"),
                   action: qsTr("Open Routing"), run: () => list.host && list.host.go("routing", "") })
        if (crossoverShown)
            out.push({ key: "crossover", done: withoutCrossover.length === 0, title: qsTr("Crossovers set"),
                       detail: withoutCrossover.length === 0 ? qsTr("Every driver has its filter")
                             //: The setup list's crossover row: %1 lists outputs ("Out 1, Out 2")
                             : qsTr("No crossover on %1").arg(withoutCrossover.join(", ")),
                       action: qsTr("Open Crossover"), run: () => list.host && list.host.go("equaliser", "crossover") })
        out.push({ key: "measure", done: measuredDone, title: qsTr("Measured"),
                   detail: measuredDone ? qsTr("A measurement was made") : qsTr("With a microphone, to tune by"),
                   action: qsTr("Measure"), run: () => list.host && list.host.go("measure", "measure") })
        out.push({ key: "save", done: savedDone, title: qsTr("Saved to the device"),
                   detail: savedDone ? qsTr("Nothing waits to be saved") : qsTr("Changes are lost when the DSPi loses power"),
                   action: qsTr("Save to Device"), run: () => Shell.trigger("device.commit") })
        return out
    }
    readonly property int doneCount: rows.filter(r => r.done).length
    readonly property bool complete: doneCount === rows.length

    visible: DeviceModel.connected && ChannelController.writable && serial.length > 0 && !closed && !complete
    spacing: Theme.spacingSmall

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacingSmall
        Text {
            text: qsTr("SET UP THIS DSPI")
            font: Theme.fontHeading
            color: Theme.textMuted
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Text {
            objectName: "OverviewChecklist.count"
            Layout.fillWidth: true
            //: The setup list's progress: %1 steps done of %2
            text: qsTr("%1 of %2 done").arg(list.doneCount).arg(list.rows.length)
            font: Theme.fontSmall
            color: Theme.textMuted
        }
        IconButton {
            objectName: "OverviewChecklist.close"
            iconName: "x"
            toolTip: qsTr("Close the setup list for this DSPi")
            onClicked: OnboardingController.closeChecklist(list.serial)
            Accessible.name: toolTip
        }
    }
    Card {
        Layout.fillWidth: true
        Layout.maximumWidth: Theme.contentMaxWidth
        padding: Theme.spacingSmall
        Repeater {
            model: list.rows
            delegate: RowLayout {
                id: step
                required property var modelData
                required property int index
                objectName: "OverviewChecklist.row." + modelData.key
                readonly property bool done: modelData.done
                Layout.fillWidth: true
                Layout.minimumHeight: Theme.rowHeight
                spacing: Theme.spacing
                Icon {
                    Layout.leftMargin: Theme.spacingSmall
                    name: step.done ? "circle-check" : "circle-dashed"
                    color: step.done ? Theme.success : Theme.textMuted
                    Accessible.role: Accessible.StaticText
                    Accessible.name: step.done ? qsTr("Done") : qsTr("Not done")
                }
                Text {
                    objectName: "OverviewChecklist.title." + step.modelData.key
                    Layout.preferredWidth: 170
                    textFormat: Text.PlainText
                    text: step.modelData.title
                    font: Theme.fontBody
                    color: step.done ? Theme.textMuted : Theme.text
                    elide: Text.ElideRight
                }
                Text {
                    objectName: "OverviewChecklist.detail." + step.modelData.key
                    Layout.fillWidth: true
                    textFormat: Text.PlainText
                    text: step.modelData.detail
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    elide: Text.ElideRight
                }
                AppButton {
                    objectName: "OverviewChecklist.action." + step.modelData.key
                    visible: !step.done
                    text: step.modelData.action
                    onClicked: step.modelData.run()
                }
            }
        }
    }

    Accessible.role: Accessible.Grouping
    Accessible.name: qsTr("Set up this DSPi")
}
