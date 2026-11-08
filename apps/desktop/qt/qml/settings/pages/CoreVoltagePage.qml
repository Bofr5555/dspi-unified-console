import QtQuick 2.15
import QtQuick.Window 2.15
import "../"
import "../../components"

// -- WP QD28 -- Core voltage: calibration and the manual requests, over the core's
// flow (vreg, CG11). Opened only from Settings > Advanced, in place of that page.
// Every limit, sentence and the OVERVOLT rule are the core's; a write is staged
// here and goes out only from the confirmation card, except the abort.
SettingsPage {
    id: page
    objectName: "coreVoltagePage"
    title: "Core Voltage"
    signal back()

    // Read while this page shows in a window that's open
    readonly property bool shown: Window.visibility !== Window.Hidden && Window.visibility !== Window.Minimized
    onShownChanged: vreg.watching = shown
    Component.onCompleted: vreg.watching = shown
    Component.onDestruction: {
        vreg.watching = false
        vreg.cancel()
        // Leaving closes the flow; a run goes on until it is ended
        if (["review", "ended", "committed", "failed", "unknown", "verify_failed"].indexOf(vreg.phase) >= 0) vreg.closeFlow()
    }

    readonly property var view: vreg.view
    readonly property var status: view.status || null
    readonly property var choices: view.choices || []
    readonly property string phase: vreg.phase
    readonly property bool running: ["clearing", "starting_trial", "soaking", "passed", "ending", "committing"].indexOf(phase) >= 0
    readonly property bool flowOpen: phase !== "idle"

    function minutes(s) { return Math.round(s / 60) }
    function formatTime(s) {
        var h = Math.floor(s / 3600), m = Math.floor(s % 3600 / 60), sec = s % 60
        if (h > 0) return h + " h " + m + " min"
        if (m > 0) return m + " min " + sec + " s"
        return sec + " s"
    }
    function choiceOptions() {
        var list = []
        for (var i = 0; i < choices.length; i++)
            list.push({ text: vreg.volts(choices[i].code) + (choices[i].code === (status ? status.defaultVreg : -1) ? " (build default)" : ""),
                        value: choices[i].code })
        return list
    }
    function offered(code) {
        for (var i = 0; i < choices.length; i++) if (choices[i].code === code) return true
        return false
    }
    readonly property var phaseTitles: ({
        idle: "Not started", review: "Ready", clearing: "Clearing the record", starting_trial: "Starting the trial",
        soaking: "Soaking", passed: "Passed", failed: "Failed", unknown: "Unknown", trial_found: "Trial found",
        trial_left_running: "Trial left running", ending: "Ending the trial", ended: "Ended",
        committing: "Saving", committed: "Saved", verify_failed: "Not confirmed"
    })

    // What the next run tries: after a failure the step up, else the lowest offered
    property int startCode: 0
    property int soakMinutes: 0
    property int manualCode: 0
    // The view's own phase: page.phase may not have caught up when the view changes
    property string pickedFor: ""
    function syncPicks() {
        if (view.phase === "failed" && pickedFor !== "failed" && view.nextVreg > 0 && offered(view.nextVreg)) startCode = view.nextVreg
        else if (!offered(startCode) && choices.length > 0) startCode = choices[0].code
        pickedFor = view.phase || ""
        if (!offered(manualCode) && choices.length > 0) manualCode = choices[0].code
        if (soakMinutes <= 0 && view.defaultSoakS > 0) soakMinutes = minutes(view.defaultSoakS)
    }
    onViewChanged: syncPicks()
    readonly property var soakOptions: {
        var list = [], min = minutes(view.minSoakS || 600)
        var all = [10, 20, 30, 45, 60, 90, 120, 240]
        for (var i = 0; i < all.length; i++)
            if (all[i] >= min) list.push({ text: all[i] + " min" + (all[i] * 60 === view.defaultSoakS ? " (suggested)" : ""), value: all[i] })
        return list
    }

    SettingsSection {
        SettingsButtonRow {
            objectName: "vregBack"
            title: "Advanced"
            detail: "Back to the Advanced page"
            buttonText: "Back"
            onClicked: page.back()
        }
    }

    // Nothing here while the connection can't use it, unless a run is in a phase
    SettingsSection {
        objectName: "vregUnavailable"
        visible: !vreg.available
        SettingsRow {
            title: !bridge.connected ? "No device is connected."
                 : !vreg.polled ? "Reading…"
                 : "Core voltage can't be changed on this connection: the firmware has no core voltage control, the connection is read-only, or a hub did not allow it."
        }
    }

    SettingsSection {
        objectName: "vregWarning"
        visible: vreg.available
        title: "Before You Change It"
        footnote: page.view.soakNote || ""
        SettingsRow {
            objectName: "vregWarningText"
            title: page.view.warning || ""
            titleColor: isMacOS ? MacColors.orange : "#ff9f0a"
        }
    }

    // The one way a write goes out
    SettingsSection {
        objectName: "vregConfirm"
        visible: vreg.pending.action !== undefined
        title: "Confirm"
        SettingsRow {
            objectName: "vregConfirmTitle"
            title: vreg.pending.title || ""
            detail: vreg.pending.detail || ""
        }
        SettingsRow {
            visible: vreg.pending.needsWord === true
            title: "This raises the voltage above 1.20 V"
            detail: "Type " + (vreg.pending.word || "") + " to confirm."
            Rectangle {
                width: 150
                height: 26
                radius: 6
                color: Qt.rgba(1, 1, 1, word.activeFocus ? 0.10 : 0.05)
                border.width: word.activeFocus ? 1 : 0
                border.color: isMacOS ? MacColors.accent : "#0a7cff"
                TextInput {
                    id: word
                    objectName: "vregWord"
                    x: 8
                    width: parent.width - 16
                    height: parent.height
                    verticalAlignment: Text.AlignVCenter
                    maximumLength: 16
                    selectByMouse: true
                    clip: true
                    activeFocusOnTab: true   // -- WP QD22 --
                    Accessible.name: "Confirmation word"
                    Accessible.description: "Type " + (vreg.pending.word || "") + " to confirm."

                    font.pixelSize: 13
                    color: isMacOS ? MacColors.label : "white"
                    selectionColor: isMacOS ? MacColors.selectedControl : "#0a7cff"
                    selectedTextColor: "white"
                    // Typed afresh for every request
                    Connections {
                        target: vreg
                        function onPendingChanged() { word.text = "" }
                    }
                }
            }
        }
        SettingsButtonRow {
            objectName: "vregConfirmButton"
            title: "Send it to the device"
            buttonText: "Confirm"
            destructive: true
            enabled: !vreg.busy && (vreg.pending.needsWord !== true || word.text !== "")
            onClicked: vreg.confirm(word.text)
        }
        SettingsButtonRow {
            objectName: "vregCancel"
            title: "Leave it as it is"
            buttonText: "Cancel"
            onClicked: vreg.cancel()
        }
    }

    SettingsSection {
        objectName: "vregStatus"
        visible: vreg.available
        title: "On the Device"
        SettingsValueRow {
            objectName: "vregRunning"
            title: "Running at"
            value: !page.status ? "Reading…"
                 : page.status.millivolts > 0 ? (page.status.millivolts / 1000).toFixed(2) + " V" + (page.status.trialActive ? " (trial)" : "")
                 : vreg.volts(page.status.liveVreg)
        }
        SettingsValueRow {
            objectName: "vregLinkLost"
            visible: page.view.linkLost === true || !bridge.connected
            title: "Connection"
            value: "Not answering: a hang in progress, or the link dropped. The result is still open."
        }
        SettingsValueRow {
            objectName: "vregStored"
            visible: page.status !== null
            title: "Saved"
            value: !page.status ? "" : page.status.storedSel === 0 ? "Build default"
                 : vreg.volts(page.status.storedSel) + (page.status.autoRaised ? ", raised by the device" : "")
        }
        SettingsValueRow {
            visible: page.status !== null
            title: "Build default"
            value: page.status ? vreg.volts(page.status.defaultVreg) : ""
        }
        SettingsValueRow {
            objectName: "vregHangs"
            visible: page.status !== null
            title: "Hangs this session"
            value: page.status ? String(page.status.hangCount) : ""
        }
        SettingsValueRow {
            objectName: "vregFailure"
            visible: page.status !== null
            title: "Last failed trial"
            value: !page.status || page.status.failedTrialVreg === 0 ? "None"
                 : vreg.volts(page.status.failedTrialVreg)
                   + (page.status.failedTrialUptimeS > 0 ? ", hung after " + page.formatTime(page.status.failedTrialUptimeS) + " of uptime" : "")
        }
    }

    SettingsSection {
        objectName: "vregFlow"
        visible: vreg.available
        title: "Calibrate"
        footnote: "The flow offers the build default up to 1.25 V and never saves by itself."
        SettingsValueRow {
            objectName: "vregPhase"
            title: "State"
            value: page.phaseTitles[page.phase] || page.phase
        }
        SettingsRow {
            objectName: "vregMessage"
            visible: page.flowOpen
            title: page.view.message || ""
        }
        SettingsValueRow {
            objectName: "vregSoak"
            visible: (page.phase === "soaking" || page.phase === "passed") && page.view.soakS > 0
            title: "Soak at " + vreg.volts(page.view.targetVreg)
            value: page.formatTime(Math.min(page.view.elapsedS, page.view.soakS)) + " of " + page.formatTime(page.view.soakS)
        }
        SettingsButtonRow {
            objectName: "vregOpen"
            visible: page.phase === "idle"
            title: "Find the lowest voltage that runs without hangs"
            detail: "Shows the warning and the device's record first. Nothing is sent yet."
            buttonText: "Calibrate…"
            enabled: !vreg.busy
            onClicked: vreg.openFlow()
        }
        SettingsChoiceRow {
            objectName: "vregStartVoltage"
            visible: page.view.canStart === true
            title: "Trial voltage"
            options: page.choiceOptions()
            value: page.startCode
            onChosen: page.startCode = value
        }
        SettingsChoiceRow {
            objectName: "vregSoakChoice"
            visible: page.view.canStart === true || page.view.canKeep === true
            title: "Soak"
            detail: "Keep audio playing during it."
            options: page.soakOptions
            value: page.soakMinutes
            onChosen: page.soakMinutes = value
        }
        SettingsButtonRow {
            objectName: "vregStart"
            visible: page.view.canStart === true
            title: page.phase === "failed" ? "Try the next step up" : page.phase === "review" ? "Start the trial" : "Run it again"
            buttonText: "Start…"
            enabled: !vreg.busy
            onClicked: vreg.request("start", page.startCode, page.soakMinutes * 60)
        }
        SettingsButtonRow {
            objectName: "vregCommit"
            visible: page.view.canCommit === true
            title: "Save " + vreg.volts(page.view.targetVreg) + " on the device"
            buttonText: "Save…"
            enabled: !vreg.busy
            onClicked: vreg.request("commit")
        }
        SettingsButtonRow {
            objectName: "vregKeep"
            visible: page.view.canKeep === true
            title: "Keep the trial and soak it"
            detail: "Sends nothing: the soak counts from now."
            buttonText: "Keep"
            enabled: !vreg.busy
            onClicked: vreg.keepTrial(page.soakMinutes * 60)
        }
        SettingsButtonRow {
            objectName: "vregEnd"
            visible: ["clearing", "starting_trial", "soaking", "passed", "trial_found", "trial_left_running", "verify_failed"].indexOf(page.phase) >= 0
                     || (page.status !== null && page.status.trialActive === true && page.phase !== "ending")
            title: page.phase === "passed" || page.phase === "trial_found" || page.phase === "trial_left_running" ? "End the trial" : "Abort"
            detail: "Returns the device to its saved voltage at once."
            buttonText: page.phase === "passed" || page.phase === "trial_found" || page.phase === "trial_left_running" ? "End Trial" : "Abort"
            destructive: true
            onClicked: vreg.endTrial()
        }
        SettingsButtonRow {
            objectName: "vregClose"
            visible: ["review", "ended", "committed", "failed", "unknown", "verify_failed"].indexOf(page.phase) >= 0
            title: "Close the calibration"
            buttonText: "Close"
            enabled: !vreg.busy
            onClicked: vreg.closeFlow()
        }
    }

    SettingsSection {
        objectName: "vregManual"
        visible: vreg.available
        enabled: !page.running
        title: "Manual"
        footnote: "The same limits and confirmation as the calibration. 1.30 V is not offered here."
        SettingsChoiceRow {
            objectName: "vregManualVoltage"
            title: "Voltage"
            options: page.choiceOptions()
            value: page.manualCode
            onChosen: page.manualCode = value
        }
        SettingsButtonRow {
            objectName: "vregManualTrial"
            title: "Run it as a trial"
            detail: "RAM only: a hang or a reboot returns to the saved voltage."
            buttonText: "Trial…"
            enabled: !vreg.busy && page.choices.length > 0
            onClicked: vreg.request("trial", page.manualCode)
        }
        SettingsButtonRow {
            objectName: "vregManualSet"
            title: "Save it on the device"
            buttonText: "Save…"
            enabled: !vreg.busy && page.choices.length > 0
            onClicked: vreg.request("set", page.manualCode)
        }
        SettingsButtonRow {
            objectName: "vregReset"
            title: "Reset to the build default"
            buttonText: "Reset…"
            enabled: !vreg.busy
            onClicked: vreg.request("reset")
        }
        SettingsButtonRow {
            objectName: "vregAuto"
            title: "Automatic raising"
            detail: page.status ? (page.status.autoDisabled ? "Off" : "On: after two hangs in one power session the device raises its saved voltage a step, up to 1.25 V.") : ""
            buttonText: page.status && page.status.autoDisabled ? "Turn On…" : "Turn Off…"
            enabled: !vreg.busy && page.status !== null
            onClicked: vreg.request("auto", page.status.autoDisabled ? 1 : 0)
        }
        SettingsButtonRow {
            objectName: "vregClear"
            title: "Clear the hang count and the last failure"
            buttonText: "Clear…"
            enabled: !vreg.busy
            onClicked: vreg.request("clear")
        }
    }

    SettingsStatusLine {
        objectName: "vregError"
        text: vreg.lastError
        error: true
    }
}
