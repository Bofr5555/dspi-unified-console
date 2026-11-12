import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP23 -- Settings > Advanced > Core Voltage (behaviour.md §20): the
// device's core voltage, the calibration flow and the manual requests. Every
// warning, limit, description and the confirmation word are the core's
// (CoreVoltageController.view); a request that changes the voltage goes
// through the Confirm block, which asks for the word when the core says so.
SettingsPage {
    id: page
    objectName: "CoreVoltagePage"

    readonly property var cv: CoreVoltageController
    readonly property var view: cv.view
    readonly property var st: cv.status
    readonly property string phase: cv.phase
    readonly property bool hasStatus: st.liveVreg !== undefined
    readonly property var choices: view.choices ? view.choices : []
    // Nothing else is asked while a request waits for its Confirm
    readonly property bool idleUi: pending === null && !cv.busy
    readonly property bool canWrite: cv.writable && idleUi
    readonly property bool manualOn: canWrite && !cv.calibrating

    // The request waiting in the Confirm block: {action, code, soak, on, text}
    property var pending: null
    property int trialCode: -1
    property int soakS: 0
    property int manualCode: -1

    function volts(code) { return SettingsController.voltsLabel(code) }
    function choiceTexts() { return choices.map(c => volts(c.code)) }
    function choiceCodes() { return choices.map(c => c.code) }
    // The picked trial voltage: the person's, else the failure's next step,
    // else the lowest offered
    function pickedTrial() {
        const codes = choiceCodes()
        if (codes.indexOf(trialCode) >= 0) return trialCode
        if (view.nextVreg && codes.indexOf(view.nextVreg) >= 0) return view.nextVreg
        return codes.length ? codes[0] : -1
    }
    function pickedSoak() {
        const list = cv.soakChoices
        if (list.indexOf(soakS) >= 0) return soakS
        return view.defaultSoakS ? view.defaultSoakS : (list.length ? list[0] : 0)
    }
    function pickedManual() {
        const codes = choiceCodes()
        return codes.indexOf(manualCode) >= 0 ? manualCode : (codes.length ? codes[0] : -1)
    }

    // Asks for a request in the Confirm block
    function ask(action, code, text, extra) {
        const p = { action: action, code: code, text: text, needsWord: cv.needsWord(action, code) }
        for (const k in extra) p[k] = extra[k]
        pending = p
        wordField.text = ""
        revealConfirm.restart()
    }
    // Once the block is laid out: in view, and the keyboard in it
    Timer {
        id: revealConfirm
        interval: 30
        onTriggered: {
            if (!page.pending) return
            page.reveal(confirmBlock)
            if (page.pending.needsWord) wordField.forceActiveFocus()
            else confirmButton.forceActiveFocus()
        }
    }
    // Scrolls `item` (one of the page's sections) into view
    function reveal(item) {
        const top = item.mapToItem(page.contentItem, 0, 0).y - Theme.spacing
        const bottom = top + item.height + 2 * Theme.spacing
        if (top < page.contentY) page.contentY = Math.max(0, top)
        else if (bottom > page.contentY + page.height)
            page.contentY = Math.max(0, Math.min(top, page.contentHeight - page.height))
    }
    function confirm() {
        const p = pending
        if (!p || (p.needsWord && !cv.wordMatches(wordField.text))) return
        const typed = p.needsWord ? wordField.text : ""
        pending = null
        wordField.text = ""
        switch (p.action) {
        case "start": cv.start(p.code, p.soak, typed); break
        case "commit": cv.commit(typed); break
        case "keep": cv.keep(p.soak); break
        case "trial": cv.manualTrial(p.code, typed); break
        case "set": cv.manualSet(p.code, typed); break
        case "reset": cv.manualSet(0, typed); break
        case "auto": cv.manualAuto(p.on); break
        case "clear": cv.manualClear(); break
        }
    }
    function cancel() {
        pending = null
        wordField.text = ""
    }

    Component.onCompleted: {
        cv.pageShown = true
        // Reached only from Advanced: restored as the last page, it goes there
        if (!SettingsController.canGoBack) Qt.callLater(() => SettingsController.open("advanced"))
    }
    Component.onDestruction: {
        cv.pageShown = false
        // Leaving the review closes it; a run goes on until it ends
        if (phase === "review" && cv.writable) cv.closeFlow()
    }

    // ---- Back, and the core's warning ----------------------------------------------
    RowLayout {
        Layout.fillWidth: true
        AppButton {
            objectName: "CoreVoltage.back"
            text: qsTr("Back")
            iconName: "chevron-left"
            toolTip: qsTr("Back to Advanced")
            onClicked: SettingsController.open("advanced")
        }
        Item { Layout.fillWidth: true }
    }

    Rectangle {
        objectName: "CoreVoltage.warning"
        visible: (view.warning || "").length > 0
        Layout.fillWidth: true
        implicitHeight: warningRow.implicitHeight + 2 * Theme.spacing
        radius: Theme.radius
        color: Theme.bannerFill("warning")
        border.width: 1
        border.color: Theme.bannerEdge("warning")
        RowLayout {
            id: warningRow
            x: Theme.spacing
            y: Theme.spacing
            width: parent.width - 2 * Theme.spacing
            spacing: Theme.spacing
            Icon {
                Layout.alignment: Qt.AlignTop
                name: "triangle-alert"
                size: 16
                color: Theme.warning
            }
            Text {
                objectName: "CoreVoltage.warningText"
                Layout.fillWidth: true
                text: view.warning || ""
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.Wrap
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }
        }
    }

    // ---- On the Device ---------------------------------------------------------------
    SettingsGroup {
        objectName: "CoreVoltage.device"
        title: qsTr("On the Device")
        iconName: "hard-drive"
        Text {
            visible: !page.hasStatus
            text: qsTr("Reading…")
            font: Theme.fontBody
            color: Theme.textMuted
        }
        SettingsValueRow {
            objectName: "CoreVoltage.running"
            visible: page.hasStatus
            title: qsTr("Running at")
            value: page.hasStatus ? page.volts(page.st.liveVreg) + (page.st.trialActive ? " " + qsTr("(trial)") : "") : ""
        }
        SettingsValueRow {
            objectName: "CoreVoltage.saved"
            visible: page.hasStatus
            title: qsTr("Saved")
            value: !page.hasStatus ? ""
                   : page.st.storedSel === 0 ? qsTr("Build default")
                   : page.volts(page.st.storedSel) + (page.st.autoRaised ? qsTr(", raised by the device") : "")
        }
        SettingsValueRow {
            objectName: "CoreVoltage.default"
            visible: page.hasStatus
            title: qsTr("Build default")
            value: page.hasStatus ? page.volts(page.st.defaultVreg) : ""
        }
        SettingsValueRow {
            objectName: "CoreVoltage.autoState"
            visible: page.hasStatus
            title: qsTr("Automatic raising")
            value: page.st.autoDisabled ? qsTr("Off") : qsTr("On")
        }
        SettingsValueRow {
            objectName: "CoreVoltage.hangs"
            visible: page.hasStatus
            title: qsTr("Hangs this session")
            value: page.hasStatus ? String(page.st.hangCount) : ""
        }
        SettingsValueRow {
            objectName: "CoreVoltage.lastFailed"
            visible: page.hasStatus
            title: qsTr("Last failed trial")
            value: !page.hasStatus || !page.st.failedTrialVreg ? qsTr("None")
                   : page.st.failedTrialUptimeS
                     ? qsTr("%1, hung after %2").arg(page.volts(page.st.failedTrialVreg))
                                                 .arg(cv.durationLabel(page.st.failedTrialUptimeS))
                     : page.volts(page.st.failedTrialVreg)
        }
    }

    // ---- Calibrate ---------------------------------------------------------------------
    SettingsGroup {
        objectName: "CoreVoltage.calibrate"
        title: qsTr("Calibrate")
        iconName: "activity"
        footer: view.soakNote || ""

        SettingsValueRow {
            objectName: "CoreVoltage.state"
            title: qsTr("State")
            value: cv.stateName(page.phase)
        }
        Text {
            objectName: "CoreVoltage.message"
            visible: text.length > 0
            Layout.fillWidth: true
            text: view.message || ""
            font: Theme.fontBody
            color: page.phase === "failed" || page.phase === "verify_failed" ? Theme.dangerText : Theme.textMuted
            wrapMode: Text.Wrap
            Accessible.role: Accessible.StaticText
            Accessible.name: text
        }
        Text {
            objectName: "CoreVoltage.linkLost"
            visible: view.linkLost === true
            Layout.fillWidth: true
            text: qsTr("The device stopped answering. The result is still open.")
            font: Theme.fontBody
            color: Theme.warning
            wrapMode: Text.Wrap
        }
        Text {
            objectName: "CoreVoltage.error"
            visible: text.length > 0
            Layout.fillWidth: true
            text: view.lastError || ""
            font: Theme.fontBody
            color: Theme.dangerText
            wrapMode: Text.Wrap
            Accessible.role: Accessible.StaticText
            Accessible.name: text
        }

        // The soak's progress, and Abort
        RowLayout {
            objectName: "CoreVoltage.soak"
            visible: page.phase === "soaking" || page.phase === "starting_trial" || page.phase === "clearing"
            Layout.fillWidth: true
            spacing: Theme.spacing
            Icon {
                name: "timer"
                size: 14
                color: Theme.textMuted
            }
            Text {
                objectName: "CoreVoltage.progress"
                Layout.fillWidth: true
                text: qsTr("%1 of %2 at %3").arg(cv.durationLabel(view.elapsedS || 0))
                                           .arg(cv.durationLabel(view.soakS || 0))
                                           .arg(page.volts(view.targetVreg || 0))
                font: Theme.fontBody
                color: Theme.text
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }
            AppButton {
                objectName: "CoreVoltage.abort"
                text: qsTr("Abort")
                destructive: true
                enabled: cv.writable && !cv.busy
                onClicked: { page.cancel(); cv.endTrial() }
            }
        }

        // Voltage and soak, when a run can start or a found trial can be kept
        RowLayout {
            visible: view.canStart === true || view.canKeep === true
            Layout.fillWidth: true
            spacing: Theme.spacingLarge
            SettingsPickerRow {
                objectName: "CoreVoltage.trialVoltage"
                visible: view.canStart === true
                enabled: page.canWrite
                label: qsTr("Trial voltage")
                values: page.choiceCodes()
                texts: page.choiceTexts()
                value: page.pickedTrial()
                onChosen: v => page.trialCode = v
            }
            SettingsPickerRow {
                objectName: "CoreVoltage.soakTime"
                enabled: page.canWrite
                label: qsTr("Soak time")
                values: cv.soakChoices
                texts: cv.soakChoices.map(s => s === view.defaultSoakS
                                          ? qsTr("%1 (suggested)").arg(cv.durationLabel(s)) : cv.durationLabel(s))
                value: page.pickedSoak()
                onChosen: v => page.soakS = v
            }
        }

        Flow {
            // No empty row while none of its buttons applies: a run between
            // its steps (Abort is in the soak's row)
            visible: !cv.calibrating || page.phase === "passed"
            Layout.fillWidth: true
            spacing: Theme.spacing
            AppButton {
                objectName: "CoreVoltage.begin"
                visible: page.phase === "idle"
                text: qsTr("Calibrate…")
                enabled: page.canWrite
                onClicked: cv.openFlow()
            }
            AppButton {
                objectName: "CoreVoltage.start"
                visible: view.canStart === true
                text: qsTr("Start…")
                primary: true
                enabled: page.canWrite && page.pickedTrial() >= 0
                onClicked: page.ask("start", page.pickedTrial(),
                                    qsTr("Start a trial at %1 for %2? Nothing is saved; a hang or a reboot returns to the saved voltage.")
                                        .arg(page.volts(page.pickedTrial())).arg(cv.durationLabel(page.pickedSoak())),
                                    { soak: page.pickedSoak() })
            }
            AppButton {
                objectName: "CoreVoltage.keep"
                visible: view.canKeep === true
                text: qsTr("Keep…")
                enabled: page.canWrite
                onClicked: page.ask("keep", view.targetVreg,
                                    qsTr("Keep the trial at %1 and soak it for %2?")
                                        .arg(page.volts(view.targetVreg)).arg(cv.durationLabel(page.pickedSoak())),
                                    { soak: page.pickedSoak() })
            }
            AppButton {
                objectName: "CoreVoltage.commit"
                visible: view.canCommit === true
                text: qsTr("Save…")
                primary: true
                enabled: page.canWrite
                onClicked: page.ask("commit", view.targetVreg,
                                    qsTr("Save %1 on the device? It runs at this voltage from now on, after every reboot.")
                                        .arg(page.volts(view.targetVreg)), {})
            }
            AppButton {
                objectName: "CoreVoltage.endTrial"
                visible: page.phase === "passed" || page.phase === "trial_found" || page.phase === "trial_left_running"
                         || page.phase === "verify_failed"
                text: qsTr("End Trial")
                enabled: cv.writable && page.idleUi
                onClicked: cv.endTrial()
            }
            AppButton {
                objectName: "CoreVoltage.close"
                visible: page.phase !== "idle" && !cv.calibrating && page.phase !== "trial_found"
                         && page.phase !== "trial_left_running"
                text: qsTr("Close")
                enabled: cv.writable && page.idleUi
                onClicked: cv.closeFlow()
            }
        }
    }

    // ---- Confirm: the request waiting, and the core's word when it asks for it ---
    Rectangle {
        id: confirmBlock
        objectName: "CoreVoltage.confirm"
        visible: page.pending !== null
        Layout.fillWidth: true
        implicitHeight: confirmColumn.implicitHeight + 2 * Theme.spacing
        radius: Theme.radius
        color: Theme.panel
        border.width: 2
        border.color: page.pending && page.pending.needsWord ? Theme.danger : Theme.accent
        Accessible.role: Accessible.Grouping
        Accessible.name: qsTr("Confirm")

        ColumnLayout {
            id: confirmColumn
            x: Theme.spacing
            y: Theme.spacing
            width: parent.width - 2 * Theme.spacing
            spacing: Theme.spacing
            Text {
                text: qsTr("Confirm")
                font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
                color: Theme.text
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            Text {
                objectName: "CoreVoltage.confirmText"
                Layout.fillWidth: true
                text: page.pending ? page.pending.text : ""
                font: Theme.fontBody
                color: Theme.text
                wrapMode: Text.Wrap
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }
            Text {
                objectName: "CoreVoltage.wordPrompt"
                visible: page.pending !== null && page.pending.needsWord
                Layout.fillWidth: true
                text: qsTr("The core asks for its word for this voltage: type %1, exactly, to confirm.").arg(view.confirmationWord || "")
                font: Theme.fontBody
                color: Theme.dangerText
                wrapMode: Text.Wrap
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }
            BorderedField {
                id: wordField
                objectName: "CoreVoltage.word"
                visible: page.pending !== null && page.pending.needsWord
                Layout.preferredWidth: 220
                label: qsTr("Confirmation word")
                placeholderText: view.confirmationWord || ""
                inputMethodHints: Qt.ImhNoPredictiveText | Qt.ImhNoAutoUppercase
                onAccepted: page.confirm()
                Keys.onEscapePressed: page.cancel()
            }
            RowLayout {
                spacing: Theme.spacing
                AppButton {
                    id: confirmButton
                    objectName: "CoreVoltage.confirmButton"
                    text: qsTr("Confirm")
                    destructive: page.pending !== null && page.pending.needsWord
                    primary: !destructive
                    enabled: cv.writable && !cv.busy && page.pending !== null
                             && (!page.pending.needsWord || cv.wordMatches(wordField.text))
                    onClicked: page.confirm()
                    Keys.onEscapePressed: page.cancel()
                }
                AppButton {
                    objectName: "CoreVoltage.cancel"
                    text: qsTr("Cancel")
                    onClicked: page.cancel()
                    Keys.onEscapePressed: page.cancel()
                }
            }
        }
    }

    // ---- Manual -------------------------------------------------------------------
    SettingsGroup {
        objectName: "CoreVoltage.manual"
        title: qsTr("Manual")
        iconName: "settings-2"
        footer: cv.calibrating ? qsTr("Not available while a calibration runs.") : ""

        SettingsPickerRow {
            objectName: "CoreVoltage.manualVoltage"
            enabled: page.manualOn
            label: qsTr("Voltage")
            values: page.choiceCodes()
            texts: page.choiceTexts()
            value: page.pickedManual()
            onChosen: v => page.manualCode = v
        }
        Flow {
            Layout.fillWidth: true
            spacing: Theme.spacing
            AppButton {
                objectName: "CoreVoltage.manualTrial"
                text: qsTr("Trial…")
                enabled: page.manualOn && page.pickedManual() >= 0
                onClicked: page.ask("trial", page.pickedManual(),
                                    qsTr("Try %1 until the next reboot? Nothing is saved.").arg(page.volts(page.pickedManual())), {})
            }
            AppButton {
                objectName: "CoreVoltage.manualSave"
                text: qsTr("Save…")
                enabled: page.manualOn && page.pickedManual() >= 0
                onClicked: page.ask("set", page.pickedManual(),
                                    qsTr("Save %1 on the device? It runs at this voltage from now on, after every reboot.")
                                        .arg(page.volts(page.pickedManual())), {})
            }
            AppButton {
                objectName: "CoreVoltage.manualReset"
                text: qsTr("Reset…")
                enabled: page.manualOn
                onClicked: page.ask("reset", 0,
                                    qsTr("Go back to the build default (%1)?")
                                        .arg(page.hasStatus ? page.volts(page.st.defaultVreg) : ""), {})
            }
            AppButton {
                objectName: "CoreVoltage.manualAuto"
                readonly property bool turnOn: page.st.autoDisabled === true
                text: turnOn ? qsTr("Turn On…") : qsTr("Turn Off…")
                toolTip: qsTr("Automatic raising")
                Accessible.name: turnOn ? qsTr("Turn on automatic raising") : qsTr("Turn off automatic raising")
                enabled: page.manualOn && page.hasStatus
                onClicked: page.ask("auto", 0,
                                    turnOn ? qsTr("Let the device raise its saved voltage by itself after repeated hangs?")
                                           : qsTr("Stop the device from raising its saved voltage by itself?"),
                                    { on: turnOn })
            }
            AppButton {
                objectName: "CoreVoltage.manualClear"
                text: qsTr("Clear…")
                toolTip: qsTr("Forget the hang count and the last failed trial")
                enabled: page.manualOn
                onClicked: page.ask("clear", 0, qsTr("Forget the hang count and the last failed trial?"), {})
            }
        }
    }
}
