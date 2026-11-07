import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Window 2.15
import Qt.labs.platform 1.1 as Platform
import "components"

// Firmware Update (Tools menu, the version banner): installs the firmware
// bundled with this Console, or a .uf2 of the user's, onto the connected DSPi or a
// board already in BOOTSEL. The user confirms once; the updater does the rest:
// check the file and the chip, ask the firmware guard (a core-voltage risk takes a
// typed answer), restart into the bootloader, write, wait for the device, check the
// version it reports. All of it works on a read-only device.
AppWindow {
    id: win
    title: "Firmware Update"
    visible: false
    width: 460
    height: 510 + titlebarHeight
    minimumWidth: width
    maximumWidth: width
    minimumHeight: height
    maximumHeight: height

    // Opening asked for an export of the configuration first
    signal exportRequested()

    property bool confirmed: false      // the user committed to the update
    property bool rebootRequested: false
    readonly property string phase: firmware.state
    readonly property bool connected: bridge.connected
    readonly property bool custom: firmware.imageCustom
    readonly property bool newer: !custom && firmware.match === 3
    readonly property string bundled: firmware.imageVersion || "unknown"
    readonly property string primaryTitle: custom ? "Install File" : newer ? "Downgrade" : "Update Firmware"
    readonly property int step: phase === "writing" ? 1 : phase === "waitingForDevice" ? 2 : phase === "verified" || phase === "unconfirmed" ? 3 : 0
    // Under way or being asked: the buttons below have nothing to add
    readonly property bool engaged: ["checking", "question", "waitingForDrive", "writing", "waitingForDevice"].indexOf(phase) >= 0

    // Every open starts over at detection
    onVisibleChanged: {
        // A board left in its bootloader by a late Cancel stays the subject
        if (visible) {
            confirmed = false
            rebootRequested = false
            if (phase !== "inBootloader") { firmware.reset(); firmware.useBundledImage() }
            firmware.beginWatching()
        }
        else firmware.stopWatching()
    }

    // Cancel usually closes this window, and this is what it has to say about that
    onPhaseChanged: {
        if (phase !== "inBootloader") return
        confirmed = false
        if (!visible) { visible = true; raise() }
    }

    function start() {
        confirmed = true
        // The updater restarts the device, after the guard has had its say
        firmware.installWhenReady()
    }
    function resetRun() {
        confirmed = false
        rebootRequested = false
        firmware.reset()
    }

    // The status card's contents for the current state
    readonly property var card: {
        switch (phase) {
        case "idle": case "waitingForBoard":
            if (confirmed) return { icon: "search", spin: true, title: "Looking for the board",
                text: "Waiting for it to appear in bootloader mode. If nothing happens after a few seconds, unplug the board, hold BOOTSEL, and plug it back in." }
            if (connected && !firmware.canReboot) return { icon: "plug", tint: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5), title: "Can't restart this device",
                text: firmware.rebootBlocked + (firmware.writeBlocked === "" ? " A board already in bootloader mode can still be updated: hold BOOTSEL while plugging it in." : "") }
            if (connected) return { icon: "check-circle", tint: isMacOS ? MacColors.accent : "#3a96ff", title: "Ready when you are",
                text: "Click " + primaryTitle + " to begin. The device will restart into bootloader mode, and audio will stop until the update finishes. Nothing is written without this click." }
            return { icon: "plug", tint: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5), title: "Connect a board",
                text: "No device is connected. Hold the BOOTSEL button while plugging a board in, and it will appear here." }
        case "waitingForVolume":
            return { icon: "drive", spin: true, title: firmware.chipName + " found",
                text: "The board is in bootloader mode. Waiting for its " + firmware.volumeName + " drive to mount - this usually takes a second or two." }
        case "ready":
            if (firmware.writeBlocked !== "") return { icon: "plug", tint: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5), title: "The demo is open", text: firmware.writeBlocked }
            if (confirmed) return { icon: "drive", spin: true, title: "Preparing to write",
                text: "Opening the " + firmware.chipName + "'s " + firmware.volumeName + " drive." }
            return { icon: "drive", tint: isMacOS ? MacColors.green : "#32d74b", title: firmware.chipName + " ready",
                text: "The board is in bootloader mode and ready to receive firmware " + bundled + ". Click " + primaryTitle + " to begin." }
        case "inBootloader":
            if (confirmed) return { icon: "drive", spin: true, title: "Waiting for the drive", text: "Waiting for the board's drive to appear." }
            return { icon: "drive", tint: isMacOS ? MacColors.orange : "#ff9f0a", title: "Nothing was written", text: firmware.failure }
        case "checking":
            return { icon: "search", spin: true, title: "Checking",
                text: "Looking at the file and the board. Nothing has changed yet." }
        case "waitingForDrive":
            return { icon: "drive", spin: true, title: "Restarting into bootloader mode",
                text: "Waiting for the board's drive to appear. Audio stops until the update finishes." }
        case "waitingForDevice":
            return { icon: "revert", spin: true, title: "Firmware written",
                text: "The board is restarting with its new firmware. This can take up to half a minute; leave it plugged in." }
        case "verified":
            return { icon: "check-circle", tint: isMacOS ? MacColors.green : "#32d74b", size: 36, title: "Update complete",
                text: "The device is back and confirmed running firmware " + firmware.verifiedVersion + "." }
        case "unconfirmed":
            return { icon: "question-circle", tint: isMacOS ? MacColors.orange : "#ff9f0a", title: "Written, not confirmed", text: firmware.failure }
        case "failed":
            return { icon: firmware.failureMundane ? "question-circle" : "warning", tint: isMacOS ? MacColors.orange : "#ff9f0a",
                title: firmware.failureMundane ? "Not quite ready" : "The update did not complete", text: firmware.failure }
        default:
            return { icon: "", title: "", text: "" }
        }
    }
    readonly property bool showBootselHint: ((phase === "idle" || phase === "waitingForBoard") && (confirmed || !connected))
                                            || (phase === "failed" && firmware.noBoardFailure)
    readonly property bool showManualHint: firmware.canReboot && !confirmed && (phase === "idle" || phase === "waitingForBoard")

    component Panel: Rectangle {
        radius: 8
        color: isMacOS ? MacColors.opacity(MacColors.controlBackground, 0.6) : Qt.rgba(1, 1, 1, 0.045)
        border.color: isMacOS ? MacColors.opacity(MacColors.gray, 0.2) : Qt.rgba(1, 1, 1, 0.07)
    }
    Column {
        anchors.fill: parent

        // ── Header ──
        Item {
            width: parent.width
            height: 56
            Rectangle {
                id: tile
                x: 16
                anchors.verticalCenter: parent.verticalCenter
                width: 28; height: 28; radius: 7
                color: isMacOS ? "transparent" : "#0a7cff"
                Icon { anchors.centerIn: parent; name: "chip"; size: 17; color: isMacOS ? MacColors.accent : "white" }
            }
            Column {
                anchors.left: tile.right
                anchors.leftMargin: 10
                anchors.verticalCenter: parent.verticalCenter
                spacing: 1
                Text { text: "Firmware Update"; font.pixelSize: 13; font.weight: Font.DemiBold; color: isMacOS ? MacColors.label : "white" }
                Text { text: "Install firmware " + win.bundled + " onto a DSPi board"; font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5) }
            }
            Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.07) }
        }

        Item {
            width: parent.width
            height: win.height - win.titlebarHeight - 56 - 52

            Column {
                id: body
                x: 16
                y: 14
                width: parent.width - 32
                spacing: 12

                // The two versions in play
                Panel {
                    width: parent.width
                    height: versions.height + 20
                    Column {
                        id: versions
                        x: 12
                        y: 10
                        width: parent.width - 24
                        spacing: 6
                        LabelValueRow { label: "Will install"; value: win.bundled + (win.custom ? "  (" + firmware.imageName + ")" : "") }
                        LabelValueRow { label: "Connected device"; value: firmware.deviceVersion || "None"; secondary: !firmware.deviceVersion }
                        // A .uf2 of one's own, checked as soon as it is chosen
                        Text {
                            objectName: "firmwareChooseFile"
                            visible: !win.engaged && win.phase !== "verified"
                            text: firmware.imageLoading ? "Reading the file…"
                                  : win.custom || firmware.imageProblem !== "" ? "Use this Console's firmware instead" : "Install a .uf2 file instead…"
                            font.pixelSize: 11
                            font.underline: fileMouse.containsMouse
                            color: isMacOS ? MacColors.link : "#3a96ff"
                            MouseArea {
                                id: fileMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: {
                                    if (win.custom || firmware.imageProblem !== "") firmware.useBundledImage()
                                    else uf2Dialog.open()
                                }
                            }
                        }
                        Text {
                            objectName: "firmwareImageProblem"
                            visible: firmware.imageProblem !== ""
                            width: versions.width
                            wrapMode: Text.WordWrap
                            textFormat: Text.PlainText
                            text: firmware.imageProblem
                            font.pixelSize: 11
                            color: isMacOS ? MacColors.orange : "#ff9f0a"
                        }
                        Row {
                            visible: win.newer
                            spacing: 6
                            Icon { name: "arrow-down-circle"; size: 13; color: isMacOS ? MacColors.orange : "#ff9f0a"; anchors.verticalCenter: parent.verticalCenter }
                            Text {
                                width: versions.width - 20
                                wrapMode: Text.WordWrap
                                text: "The device is newer than this Console, so this would be a downgrade."
                                font.pixelSize: 11
                                color: isMacOS ? MacColors.orange : "#ff9f0a"
                            }
                        }
                    }
                }

                // Prepare - Write - Verify - Done
                StepStrip {
                    width: parent.width
                    labels: ["Prepare", "Write", "Verify", "Done"]
                    current: win.step
                    dimmed: win.phase === "failed"
                }

                // What is happening now, and what the user does next
                Item {
                    id: statusCard
                    width: parent.width
                    height: body.parent.height - body.y - y - (hints.visible ? hints.height + body.spacing : 0) - 14
                    InstallStateCard {
                        anchors.fill: parent
                        visible: win.phase !== "writing" && win.phase !== "question"
                        icon: win.card.icon
                        tint: isMacOS ? (win.card.tint || MacColors.accent) : win.card.tint || "#3a96ff"
                        spin: win.card.spin === true
                        iconSize: win.card.size || 28
                        title: win.card.title
                        text: win.card.text
                    }
                    FirmwareQuestion {
                        objectName: "firmwareQuestion"
                        anchors.fill: parent
                        visible: win.phase === "question"
                    }
                    InstallWritingCard {
                        anchors.fill: parent
                        visible: win.phase === "writing"
                        progress: firmware.progress
                        boardName: firmware.chipName
                        version: win.bundled
                    }
                }

                // BOOTSEL, or flashing a UF2 of one's own
                Item {
                    id: hints
                    visible: win.showBootselHint || win.showManualHint
                    width: parent.width
                    height: win.showBootselHint ? bootsel.height : manual.height
                    Row {
                        id: bootsel
                        visible: win.showBootselHint
                        spacing: 8
                        Icon { name: "cs-button"; size: 14; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5) }
                        Text {
                            width: hints.width - 22
                            wrapMode: Text.WordWrap
                            text: "Hold the BOOTSEL button on your Pico-compatible device while plugging it into your computer."
                            font.pixelSize: 11
                            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
                        }
                    }
                    Row {
                        id: manual
                        visible: !win.showBootselHint && win.showManualHint
                        spacing: 4
                        Text { text: "Flashing a UF2 of your own?"; font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5) }
                        Text {
                            text: "Enter bootloader mode without installing"
                            font.pixelSize: 11
                            font.underline: linkMouse.containsMouse
                            color: isMacOS ? MacColors.link : "#3a96ff"
                            MouseArea {
                                id: linkMouse
                                anchors.fill: parent
                                hoverEnabled: true
                                cursorShape: Qt.PointingHandCursor
                                onClicked: firmware.enterBootloader()
                            }
                        }
                    }
                }
            }
        }

        // ── Buttons ──
        Item {
            width: parent.width
            height: 52
            Rectangle { width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.07) }
            Row {
                x: 16
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                AppButton {
                    visible: win.phase === "verified" || win.phase === "unconfirmed"
                    text: "Update Another Board"
                    onClicked: win.resetRun()
                }
                AppButton {
                    visible: win.phase !== "verified" && win.phase !== "unconfirmed"
                    text: "Cancel"
                    onClicked: win.close()
                }
                // A UF2 write leaves presets alone, but a format change between
                // versions can make them unreadable: offer a backup first
                AppButton {
                    objectName: "firmwareExport"
                    visible: win.phase !== "verified" && win.phase !== "unconfirmed" && win.connected && !win.confirmed
                    text: "Export Configuration…"
                    onClicked: win.exportRequested()
                }
            }
            Row {
                anchors.right: parent.right
                anchors.rightMargin: 16
                anchors.verticalCenter: parent.verticalCenter
                spacing: 8
                AppButton {
                    visible: win.phase === "verified" || win.phase === "unconfirmed"
                    text: "Done"
                    primary: true
                    onClicked: win.close()
                }
                AppButton {
                    visible: win.phase === "failed"
                    text: "Try Again"
                    primary: true
                    onClicked: win.resetRun()
                }
                AppButton {
                    objectName: "firmwareInstall"
                    visible: !win.engaged && win.phase !== "verified" && win.phase !== "unconfirmed" && win.phase !== "failed"
                    text: win.primaryTitle
                    primary: true
                    // A device that can't be restarted from here leaves only a drive to write
                    enabled: !win.confirmed && !firmware.imageLoading && firmware.imageProblem === ""
                             && (!win.connected || firmware.canReboot || ((win.phase === "ready" || win.phase === "inBootloader") && firmware.writeBlocked === ""))
                    onClicked: win.start()
                }
            }
        }
    }

    Platform.FileDialog {
        id: uf2Dialog
        title: "Choose a Firmware File"
        fileMode: Platform.FileDialog.OpenFile
        nameFilters: ["Firmware (*.uf2)", "All files (*)"]
        onAccepted: firmware.chooseImage(file)
    }

    Shortcut { sequence: "Escape"; enabled: win.phase !== "writing" && win.phase !== "waitingForDevice"; onActivated: win.close() }
}
