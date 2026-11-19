import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LKUI1 -- Window > System...: the audio link's installation on the hub
// this console is on (the web console's System view, SW3), from
// SystemController: every element with its role and alink state, the alink
// delay and its range, the hop offset and the phase error; the scene bar; and
// the Clock-and-audio-links editor. Simple shows the status only; the editor and the
// scene bar are Advanced's (docs/UI-MODES.md). The hub's role decides what may
// be changed: a Viewer gets nothing that writes.
ToolWindow {
    id: window
    objectName: "SystemWindow"
    windowId: "system"
    defaultWidth: 860
    defaultHeight: 640
    minimumWidth: 560
    minimumHeight: 360
    title: qsTr("System")

    readonly property bool simple: ModeController.mode === "simple"
    readonly property bool editable: SystemController.mayEdit && !SystemController.busy

    onVisibleChanged: if (visible) SystemController.refresh()

    function healthColor(h) {
        if (h === "ok") return Theme.success
        if (h === "warning") return Theme.warning
        if (h === "problem") return Theme.dangerText
        return Theme.offline
    }
    function stateColor(u) {
        if (!u.connected) return Theme.offline
        // -- WP AUSB2 -- asked to be in step and cannot: muted or unaligned
        if (u.usbLockError) return Theme.dangerText
        // -- WP HWLK1F -- a firmware without the audio link: a fact, not a fault
        if (u.state === "no_alink") return Theme.offline
        if (u.state === "holdover") return Theme.dangerText
        if (u.state === "searching" || u.state === "off" || u.state === "unknown") return Theme.warning
        // -- WP AUSB1 -- USB-direct: no time base or the host out of range is a fault
        if (u.state === "no_usb_direct") return Theme.offline
        if (u.state === "no_sof" || u.state === "rejected") return Theme.dangerText
        if (u.state === "pull_in" || u.state === "anchoring" || u.state === "usb_off") return Theme.warning
        if (u.state === "rate_locked" && u.usbModeWanted !== "rate_lock") return Theme.warning
        return Theme.success
    }
    function frames(v) { return v === undefined || v === null ? "–" : String(v) }
    function ms(v) { return v === undefined || v === null ? "" : qsTr("%1 ms").arg(Number(v).toFixed(2)) }
    function roleTitle(r) {
        return r === "admin" ? qsTr("Admin") : r === "control" ? qsTr("Control") : r === "viewer" ? qsTr("Viewer") : r
    }

    // An editor field: shows the draft's value (or the view's), takes a typed
    // one back, and follows the draft when it is not being typed in
    component DraftField: BorderedField {
        id: f
        property var read: () => ""
        property var write: t => true
        function sync() { if (!activeFocus) text = read() }
        Layout.preferredWidth: 80
        enabled: window.editable
        Accessible.name: label
        Component.onCompleted: sync()
        Connections {
            target: SystemController
            function onDraftChanged() { f.sync() }
            function onRoomsChanged() { f.sync() }
            function onEditUnitsChanged() { f.sync() }
            function onUnitsChanged() { f.sync() }
        }
        onCommitted: t => { if (!write(t)) text = read() }
    }

    // Not on a hub (a device this console holds itself, or none)
    Text {
        objectName: "SystemWindow.none"
        anchors.centerIn: parent
        width: parent.width - 2 * Theme.spacingLarge
        visible: !SystemController.available
        horizontalAlignment: Text.AlignHCenter
        wrapMode: Text.Wrap
        text: SystemController.error.length > 0 && DeviceModel.remote
              ? SystemController.error
              : qsTr("Open a DSPi on a hub with an audio link to see its installation here.")
        font: Theme.fontBody
        color: Theme.textMuted
    }

    Flickable {
        id: flick
        anchors.fill: parent
        visible: SystemController.available
        contentWidth: width
        contentHeight: column.implicitHeight + 2 * Theme.spacing
        clip: true
        ScrollBar.vertical: AppScrollBar {}

        ColumnLayout {
            id: column
            x: Theme.spacing
            y: Theme.spacing
            width: flick.width - 2 * Theme.spacing
            spacing: Theme.spacing

            // ---- Status ----------------------------------------------------
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacingSmall
                Rectangle {
                    implicitWidth: 10
                    implicitHeight: 10
                    radius: 5
                    color: window.healthColor(SystemController.health)
                }
                Text {
                    objectName: "SystemWindow.summary"
                    Layout.fillWidth: true
                    text: SystemController.summary
                    font: Theme.fontTitle
                    color: Theme.text
                    elide: Text.ElideRight
                    Accessible.role: Accessible.StaticText
                    Accessible.name: text
                }
                Text {
                    objectName: "SystemWindow.role"
                    text: qsTr("Role on the hub: %1").arg(window.roleTitle(SystemController.role))
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
            }
            Text {
                Layout.fillWidth: true
                visible: SystemController.error.length > 0
                text: SystemController.error
                font: Theme.fontSmall
                color: Theme.warning
                wrapMode: Text.Wrap
            }
            Repeater {
                model: SystemController.alarms
                delegate: Text {
                    required property var modelData
                    Layout.fillWidth: true
                    text: qsTr("%1: %2").arg(modelData.subject).arg(modelData.message)
                          + (modelData.state === "acknowledged" ? " " + qsTr("(acknowledged)") : "")
                    font: Theme.fontSmall
                    color: modelData.severity === "critical" ? Theme.alarmRaised : Theme.alarmAcknowledged
                    wrapMode: Text.Wrap
                }
            }

            // ---- Scene bar (Advanced) --------------------------------------
            SectionHeading {
                Layout.fillWidth: true
                visible: !window.simple && SystemController.scenes.length > 0
                text: qsTr("Scenes")
            }
            Flow {
                objectName: "SystemWindow.scenes"
                Layout.fillWidth: true
                visible: !window.simple && SystemController.scenes.length > 0
                spacing: Theme.spacingSmall
                Repeater {
                    model: SystemController.scenes
                    delegate: AppButton {
                        required property var modelData
                        required property int index
                        objectName: "SystemWindow.scene." + modelData.id
                        text: (index + 1) + "  " + modelData.name
                        primary: SystemController.recalledScene === modelData.name
                        enabled: SystemController.mayRecall && !SystemController.busy
                        toolTip: SystemController.mayRecall
                                 ? (modelData.systemMute === true ? qsTr("Recall %1; it sets the system mute").arg(modelData.name)
                                    : modelData.systemMute === false ? qsTr("Recall %1; it clears the system mute").arg(modelData.name)
                                    : qsTr("Recall %1 on every unit it names").arg(modelData.name))
                                 : SystemController.recallNote
                        onClicked: SystemController.recall(modelData.id)
                    }
                }
            }
            Text {
                objectName: "SystemWindow.recallNote"
                Layout.fillWidth: true
                visible: !window.simple && !SystemController.mayRecall && SystemController.scenes.length > 0
                text: SystemController.recallNote
                font: Theme.fontSmall
                color: Theme.textMuted
            }
            ColumnLayout {
                Layout.fillWidth: true
                visible: !window.simple && SystemController.lastRecall.message !== undefined
                spacing: 2
                Text {
                    objectName: "SystemWindow.recallResult"
                    Layout.fillWidth: true
                    text: SystemController.lastRecall.message || ""
                    font: Theme.fontBody
                    color: SystemController.lastRecall.taken ? Theme.text : Theme.dangerText
                    wrapMode: Text.Wrap
                    Accessible.role: Accessible.StaticText
                    Accessible.name: text
                }
                Text {
                    Layout.fillWidth: true
                    visible: text.length > 0
                    text: (SystemController.lastRecall.writes || [])
                          .map(w => qsTr("%1 · %2: %3").arg(w.unit).arg(w.what).arg(w.resultWord)).join("    ")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }

            // ---- Units -------------------------------------------------------
            SectionHeading {
                Layout.fillWidth: true
                text: qsTr("Units")
            }
            Rectangle {
                Layout.fillWidth: true
                implicitHeight: table.implicitHeight + 2
                radius: Theme.radius
                color: Theme.panel
                border.color: Theme.separator

                ColumnLayout {
                    id: table
                    objectName: "SystemWindow.units"
                    x: 1
                    y: 1
                    width: parent.width - 2
                    spacing: 0

                    component Cell: Text {
                        property int cellWidth: 90
                        Layout.preferredWidth: cellWidth
                        font: Theme.fontSmall
                        color: Theme.text
                        elide: Text.ElideRight
                    }

                    RowLayout {
                        Layout.fillWidth: true
                        Layout.margins: Theme.spacingSmall
                        spacing: Theme.spacingSmall
                        Cell { Layout.fillWidth: true; text: qsTr("Unit"); color: Theme.textMuted }
                        Cell { text: qsTr("Role"); color: Theme.textMuted; cellWidth: 70 }
                        Cell { text: qsTr("Audio link"); color: Theme.textMuted; cellWidth: 130 }
                        Cell {
                            text: qsTr("Audio link delay"); color: Theme.textMuted; cellWidth: 120
                            HoverHandler { id: dLocalHover }
                            ToolTip.visible: dLocalHover.hovered
                            ToolTip.text: qsTr("Frames, with ms beside them (d_local)")
                        }
                        Cell {
                            text: qsTr("Audio link delay range"); color: Theme.textMuted; cellWidth: 100
                            HoverHandler { id: rangeHover }
                            ToolTip.visible: rangeHover.hovered
                            ToolTip.text: qsTr("Frames (d_local_min to d_local_max)")
                        }
                        Cell {
                            text: qsTr("Hop offset"); color: Theme.textMuted; cellWidth: 70
                            HoverHandler { id: hopHover }
                            ToolTip.visible: hopHover.hovered
                            ToolTip.text: qsTr("Frames (hops_offset)")
                        }
                        Cell { text: qsTr("Phase error"); color: Theme.textMuted; cellWidth: 80 }
                    }
                    Repeater {
                        model: SystemController.units
                        delegate: Rectangle {
                            id: unitRow
                            required property var modelData
                            required property int index
                            objectName: "SystemWindow.unit." + modelData.id
                            property string shownName: modelData.name + ", " + modelData.stateWord
                            // -- WP AUSB1 -- a USB-direct unit's statistics line below its name
                            property bool usb: modelData.transport === "usb_direct"
                            // -- WP ATQT -- a unit fed by S/PDIF is not on the audio link: its row is the S/PDIF table's
                            visible: modelData.transport !== "spdif"
                            Layout.fillWidth: true
                            implicitHeight: (usb && modelData.usbStats ? Theme.rowHeight + 24 : Theme.rowHeight + 8)
                                            + (usb && (modelData.usbLockError || modelData.usbPlayUnalignedUnsupported) ? 16 : 0)
                            color: index % 2 ? Qt.rgba(Theme.text.r, Theme.text.g, Theme.text.b, 0.03) : "transparent"
                            Accessible.role: Accessible.Row
                            Accessible.name: [modelData.name, modelData.role, modelData.stateWord].join(", ")
                            RowLayout {
                                anchors.fill: parent
                                anchors.leftMargin: Theme.spacingSmall
                                anchors.rightMargin: Theme.spacingSmall
                                spacing: Theme.spacingSmall
                                ColumnLayout {
                                    Layout.fillWidth: true
                                    spacing: 0
                                    Cell { Layout.fillWidth: true; text: unitRow.modelData.name; font: Theme.fontBody }
                                    Cell {
                                        Layout.fillWidth: true
                                        visible: text.length > 0
                                        text: [unitRow.modelData.room, unitRow.modelData.serial].filter(s => s && s.length).join(" · ")
                                        color: Theme.textMuted
                                        font: Theme.fontCaption
                                    }
                                    // -- WP AUSB2 -- why it can't lock, or firmware that cannot play unaligned
                                    Cell {
                                        objectName: "SystemWindow.lockError." + unitRow.modelData.id
                                        Layout.fillWidth: true
                                        visible: unitRow.usb && text.length > 0
                                        text: unitRow.modelData.usbLockError && unitRow.modelData.usbLockErrorCause
                                              ? qsTr("Can't lock: %1").arg(unitRow.modelData.usbLockErrorCause)
                                              : unitRow.modelData.usbPlayUnalignedUnsupported
                                              ? qsTr("Its firmware cannot play unaligned: it mutes when it can't lock") : ""
                                        color: unitRow.modelData.usbLockError ? Theme.dangerText : Theme.warning
                                        font: Theme.fontCaption
                                    }
                                    Cell {
                                        objectName: "SystemWindow.usbStats." + unitRow.modelData.id
                                        Layout.fillWidth: true
                                        visible: unitRow.usb && text.length > 0
                                        text: unitRow.modelData.usbStats || ""
                                        color: Theme.textMuted
                                        font: Theme.fontCaption
                                    }
                                }
                                Cell { text: unitRow.modelData.role; cellWidth: 70 }
                                RowLayout {
                                    Layout.preferredWidth: 130
                                    spacing: 4
                                    Rectangle {
                                        implicitWidth: 8
                                        implicitHeight: 8
                                        radius: 4
                                        color: window.stateColor(unitRow.modelData)
                                    }
                                    Cell {
                                        cellWidth: 118
                                        text: unitRow.modelData.pending ? qsTr("%1, pending").arg(unitRow.modelData.stateWord)
                                                                         : unitRow.modelData.stateWord
                                        color: unitRow.modelData.pending ? Theme.warning : Theme.text
                                        ToolTip.visible: pendingHover.hovered && unitRow.modelData.pending
                                        ToolTip.text: qsTr("The audio link delay the system delay asks for is not written to this unit yet (unit: %1 frames, hop offset %2)")
                                                      .arg(window.frames(unitRow.modelData.unitAlinkDelayFrames))
                                                      .arg(window.frames(unitRow.modelData.unitHopOffset))
                                        HoverHandler { id: pendingHover }
                                    }
                                }
                                Cell {
                                    objectName: "SystemWindow.delay." + unitRow.modelData.id
                                    cellWidth: 120
                                    text: unitRow.usb
                                          ? qsTr("%1 smp").arg(window.frames(unitRow.modelData.usbDelaySamples)) + "  " + window.ms(unitRow.modelData.usbDelayMs)
                                          : unitRow.modelData.kind === "unit"
                                          ? window.frames(unitRow.modelData.alinkDelayFrames) + "  " + window.ms(unitRow.modelData.alinkDelayMs)
                                          : unitRow.modelData.hopDelayFrames !== undefined && unitRow.modelData.hopDelayFrames !== null
                                            ? qsTr("+%1 frames").arg(unitRow.modelData.hopDelayFrames) : "–"
                                }
                                Cell {
                                    cellWidth: 100
                                    text: unitRow.usb
                                          ? (unitRow.modelData.usbDelayMin !== undefined && unitRow.modelData.usbDelayMin !== null
                                             ? qsTr("%1–%2 smp").arg(unitRow.modelData.usbDelayMin).arg(unitRow.modelData.usbDelayMax) : "–")
                                          : unitRow.modelData.alinkDelayMin !== undefined && unitRow.modelData.alinkDelayMin !== null
                                          ? unitRow.modelData.alinkDelayMin + "–" + unitRow.modelData.alinkDelayMax : "–"
                                }
                                Cell { cellWidth: 70; text: unitRow.usb ? "–" : window.frames(unitRow.modelData.hopOffset) }
                                Cell {
                                    cellWidth: 80
                                    text: unitRow.usb
                                          ? (unitRow.modelData.phaseErrorSamples !== undefined && unitRow.modelData.phaseErrorSamples !== null
                                             ? qsTr("%1 smp").arg(Number(unitRow.modelData.phaseErrorSamples).toFixed(2)) : "–")
                                          : unitRow.modelData.phaseErrorFrames !== undefined && unitRow.modelData.phaseErrorFrames !== null
                                          ? Number(unitRow.modelData.phaseErrorFrames).toFixed(2) : "–"
                                }
                            }
                        }
                    }
                }
            }
            Repeater {
                model: SystemController.issues
                delegate: Text {
                    required property string modelData
                    Layout.fillWidth: true
                    text: modelData
                    font: Theme.fontSmall
                    color: Theme.warning
                    wrapMode: Text.Wrap
                }
            }

            // ---- Clock and audio links (Advanced) --------------------------------
            SectionHeading {
                Layout.fillWidth: true
                visible: !window.simple
                text: qsTr("Clock and audio links")
            }
            Text {
                objectName: "SystemWindow.editNote"
                Layout.fillWidth: true
                visible: !window.simple && !SystemController.mayEdit
                text: qsTr("%1 This part is read only.").arg(SystemController.editNote)
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
            GridLayout {
                objectName: "SystemWindow.editor"
                Layout.fillWidth: true
                visible: !window.simple
                columns: 3
                columnSpacing: Theme.spacing
                rowSpacing: Theme.spacingSmall

                Text {
                    text: qsTr("System delay")
                    font: Theme.fontBody
                    color: Theme.text
                    HoverHandler { id: dHover }
                    ToolTip.visible: dHover.hovered
                    ToolTip.text: qsTr("D: every unit's audio link delay is worked out from it")
                }
                RowLayout {
                    spacing: Theme.spacingSmall
                    AppCheckBox {
                        objectName: "SystemWindow.delayAuto"
                        text: qsTr("Automatic")
                        checked: SystemController.draftDelayAuto
                        enabled: window.editable
                        onToggled: SystemController.setSystemDelay(checked, SystemController.systemDelayFrames)
                    }
                    DraftField {
                        objectName: "SystemWindow.delayFrames"
                        enabled: window.editable && !SystemController.draftDelayAuto
                        validator: IntValidator { bottom: 0; top: 65535 }
                        label: qsTr("System delay in frames")
                        read: () => String(SystemController.draftDelayFrames)
                        write: t => { SystemController.setSystemDelay(false, parseInt(t) || 0); return true }
                    }
                    Text {
                        text: qsTr("frames")
                        font: Theme.fontSmall
                        color: Theme.textMuted
                    }
                }
                Text {
                    Layout.fillWidth: true
                    text: qsTr("In use: %1 frames (%2)").arg(SystemController.systemDelayFrames).arg(window.ms(SystemController.systemDelayMs))
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }

                Repeater {
                    model: SystemController.rooms
                    delegate: RowLayout {
                        id: roomRow
                        required property var modelData
                        Layout.columnSpan: 3
                        spacing: Theme.spacing
                        Text {
                            Layout.preferredWidth: 220
                            text: qsTr("Lip sync, %1").arg(roomRow.modelData.name)
                            font: Theme.fontBody
                            color: Theme.text
                            elide: Text.ElideRight
                        }
                        DraftField {
                            objectName: "SystemWindow.lipSync." + roomRow.modelData.id
                            placeholderText: qsTr("Model's")
                            label: qsTr("Lip sync of %1 in µs").arg(roomRow.modelData.name)
                            read: () => SystemController.roomLipSyncText(roomRow.modelData.id)
                            write: t => SystemController.setRoomLipSync(roomRow.modelData.id, t)
                        }
                        Text {
                            text: qsTr("µs")
                            font: Theme.fontSmall
                            color: Theme.textMuted
                        }
                    }
                }
                Repeater {
                    model: SystemController.editUnits
                    delegate: RowLayout {
                        id: latencyRow
                        required property var modelData
                        Layout.columnSpan: 3
                        spacing: Theme.spacing
                        Text {
                            Layout.preferredWidth: 220
                            text: qsTr("Measured latency, %1").arg(latencyRow.modelData.name)
                            font: Theme.fontBody
                            color: Theme.text
                            elide: Text.ElideRight
                        }
                        DraftField {
                            objectName: "SystemWindow.latency." + latencyRow.modelData.id
                            placeholderText: String(latencyRow.modelData.latencyUs)
                            label: qsTr("Measured latency of %1 in µs").arg(latencyRow.modelData.name)
                            read: () => SystemController.unitLatencyText(latencyRow.modelData.id)
                            write: t => SystemController.setUnitLatency(latencyRow.modelData.id, t)
                        }
                        Text {
                            text: qsTr("µs; empty: the output latency counts")
                            font: Theme.fontSmall
                            color: Theme.textMuted
                        }
                    }
                }
            }
            // -- WP AUSB2 -- the USB-direct units' mode and lock-error action, and
            // keeping them aligned (Installer: Advanced, Admin)
            GridLayout {
                objectName: "SystemWindow.usbEditor"
                Layout.fillWidth: true
                visible: !window.simple && SystemController.usbUnits.length > 0
                columns: 3
                columnSpacing: Theme.spacing
                rowSpacing: Theme.spacingSmall
                Repeater {
                    model: SystemController.usbUnits
                    delegate: RowLayout {
                        id: usbRow
                        required property var modelData
                        readonly property var modes: ["", "off", "rate_lock", "phase_lock"]
                        readonly property var actions: ["mute", "play_unaligned"]
                        Layout.columnSpan: 3
                        spacing: Theme.spacing
                        Text {
                            Layout.preferredWidth: 220
                            text: qsTr("USB-direct, %1").arg(usbRow.modelData.name)
                            font: Theme.fontBody
                            color: Theme.text
                            elide: Text.ElideRight
                        }
                        AppComboBox {
                            objectName: "SystemWindow.usbMode." + usbRow.modelData.id
                            implicitWidth: 130
                            label: qsTr("USB mode of %1").arg(usbRow.modelData.name)
                            enabled: window.editable
                            model: [qsTr("Its own"), qsTr("Off"), qsTr("Rate lock"), qsTr("Phase lock")]
                            currentIndex: Math.max(0, usbRow.modes.indexOf(SystemController.usbModeOf(usbRow.modelData.id)))
                            onActivated: i => SystemController.setUsbMode(usbRow.modelData.id, usbRow.modes[i])
                        }
                        AppComboBox {
                            objectName: "SystemWindow.onLockError." + usbRow.modelData.id
                            implicitWidth: 240
                            label: qsTr("What %1 does when it can't lock").arg(usbRow.modelData.name)
                            enabled: window.editable
                            model: [qsTr("Mute when it can't lock"), qsTr("Play unaligned when it can't lock")]
                            currentIndex: Math.max(0, usbRow.actions.indexOf(SystemController.onLockErrorOf(usbRow.modelData.id)))
                            onActivated: i => SystemController.setOnLockError(usbRow.modelData.id, usbRow.actions[i])
                            HoverHandler { id: lockHover }
                            ToolTip.visible: lockHover.hovered
                            ToolTip.text: qsTr("When a phase-locked unit can't be in step (its host's rate out of range, a rate or delay that can't phase-lock), it stays muted, or plays unaligned")
                        }
                    }
                }
                AppCheckBox {
                    objectName: "SystemWindow.keepUsbAligned"
                    Layout.columnSpan: 3
                    text: qsTr("Keep USB-direct units aligned")
                    checked: SystemController.keepUsbAligned
                    enabled: window.editable
                    onToggled: SystemController.setKeepUsbAligned(checked)
                    ToolTip.visible: hovered
                    ToolTip.text: qsTr("When a USB-direct stream's start moves, the hub writes their output delays again by itself")
                }
            }
            // -- WP ATQT -- the S/PDIF units: a follower's source, a source's slots and D
            SpdifUnitsCard {
                Layout.fillWidth: true
                visible: !window.simple && (SystemController.spdifUnits.length > 0 || window.editable)
                editable: window.editable
            }
            Text {
                objectName: "SystemWindow.stale"
                Layout.fillWidth: true
                visible: !window.simple && SystemController.stale
                text: qsTr("The installation changed on the hub while you edited it. Save is withdrawn: reload it and make your changes again.")
                font: Theme.fontBody
                color: Theme.warning
                wrapMode: Text.Wrap
            }
            RowLayout {
                Layout.fillWidth: true
                visible: !window.simple
                spacing: Theme.spacingSmall
                AppButton {
                    objectName: "SystemWindow.save"
                    text: qsTr("Save")
                    primary: true
                    visible: !SystemController.stale
                    enabled: window.editable && SystemController.dirty
                    toolTip: SystemController.mayEdit ? qsTr("Save the changes on the hub") : SystemController.editNote
                    onClicked: SystemController.save()
                }
                AppButton {
                    objectName: "SystemWindow.reload"
                    text: SystemController.stale ? qsTr("Reload") : qsTr("Revert")
                    primary: SystemController.stale
                    enabled: SystemController.dirty && !SystemController.busy
                    onClicked: SystemController.reload()
                }
                AppButton {
                    objectName: "SystemWindow.align"
                    text: qsTr("Align audio link delays…")
                    enabled: window.editable && !SystemController.dirty
                    toolTip: SystemController.mayEdit
                             ? qsTr("Align every unit: write the audio link delays the system delay asks for")
                             : SystemController.editNote
                    onClicked: alignQuestion.open()
                }
                Text {
                    objectName: "SystemWindow.actionMessage"
                    Layout.fillWidth: true
                    text: SystemController.actionMessage
                    font: Theme.fontSmall
                    color: SystemController.actionFailed ? Theme.dangerText : Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
        }
    }

    QuestionDialog {
        id: alignQuestion
        objectName: "SystemWindow.alignQuestion"
        title: qsTr("Align audio link delays?")
        text: qsTr("Every unit restarts its outputs in step with a short silence (2 to 6 ms).")
        acceptText: qsTr("Align")
        onAnswered: answer => { if (answer === "accept") SystemController.align() }
    }
}
