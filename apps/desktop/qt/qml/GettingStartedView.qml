import QtQuick 2.15
import QtQuick.Controls 2.15
import "components"

// The Getting Started wizard: replaces the console inside the main window on
// first launch (and from Help), as on macOS. One objective - a board running
// verified DSPi firmware - in three steps: Welcome, Board, Done. The firmware
// install happens right here, through the same updater as Firmware Update.
// Its words are the core's (onboardingText), resolved for this platform; only
// what the state machine picks between them and the icons belong here.
Rectangle {
    id: wiz
    objectName: "gettingStarted"
    color: isMacOS ? MacColors.windowMaterial : nativeWindowColor
    signal finished()

    readonly property var words: onboardingText.gettingStarted()
    function byId(list, id) {
        for (var i = 0; list && i < list.length; i++) if (list[i].id === id) return list[i]
        return {}
    }
    readonly property var welcomeWords: byId(words.stages, "welcome")
    readonly property var boardWords: byId(words.stages, "board")
    readonly property var doneWords: byId(words.stages, "done")
    readonly property var stages: (words.stages || []).map(function (s) { return s.label })
    readonly property var buttons: words.buttons || {}
    function joined(paragraphs) {
        return (paragraphs || []).map(function (p) { return p.text }).join("\n\n")
    }
    // The core's `icon` names, in this app's icon set
    function iconFor(name) {
        switch (name) {
        case "connect": return "plug"
        case "verified": return "check-circle"
        case "outputs": return "output"
        case "help": return "question-circle"
        default: return name
        }
    }
    // A text's {name}s: values only the app knows, at run time
    function fill(text) {
        var values = { chip: firmware.chipName, volume: firmware.volumeName, version: firmware.verifiedVersion,
                       bundled_version: bundled, device_version: firmware.deviceVersion, failure: firmware.failure }
        return (text || "").replace(/\{(\w+)\}/g, function (all, name) {
            return values[name] !== undefined ? values[name] : all
        })
    }
    property int stage: 0
    property bool confirmed: false
    property bool rebootRequested: false

    readonly property string phase: firmware.state
    readonly property string bundled: firmware.expectedVersion || "unknown"
    readonly property bool installing: phase === "writing" || phase === "waitingForDevice"
    // A running DSPi the board step talks about instead of a bootloader:
    // 1 matches, 2 older, 3 newer (0 = none, or the installer owns the step)
    readonly property int connectedMatch: !confirmed && bridge.connected && (phase === "idle" || phase === "waitingForBoard")
                                          ? firmware.match : 0

    // Detection runs only while the board step is on screen
    onStageChanged: {
        if (stage === 1) { resetRun(); firmware.beginWatching() }
        else firmware.stopWatching()
    }
    Component.onDestruction: firmware.stopWatching()

    function resetRun() {
        confirmed = false
        rebootRequested = false
        firmware.reset()
        firmware.useBundledImage()   // -- WP QD11 -- A file chosen in Firmware Update is not this wizard's
    }
    // Onto a board already in BOOTSEL
    function beginInstall() {
        confirmed = true
        firmware.installWhenReady()
    }
    // Onto the DSPi already connected: restart it into the bootloader first
    function beginConnectedInstall() {
        confirmed = true
        // -- WP QD11 -- The updater restarts the device, after the firmware guard
        firmware.installWhenReady()
    }
    function advance() {
        if (stage < stages.length - 1) stage++
        else finished()
    }

    // Which header the board step shows
    function headerFor(phase, match) {
        switch (phase) {
        // -- WP QD11 --
        case "unconfirmed": return byId(boardWords.headers, "board.unconfirmed")
        case "question": return byId(boardWords.headers, "board.question")
        case "inBootloader": return byId(boardWords.headers, "board.in_bootloader")
        case "verified": return byId(boardWords.headers, "board.verified")
        case "failed": return byId(boardWords.headers, "board.failed")
        case "writing": case "waitingForDevice": return byId(boardWords.headers, "board.installing")
        }
        switch (match) {
        case 1: return byId(boardWords.headers, "board.current")
        case 2: return byId(boardWords.headers, "board.older")
        case 3: return byId(boardWords.headers, "board.newer")
        default: return byId(boardWords.headers, "board.prepare")
        }
    }
    readonly property var boardHeader: headerFor(phase, connectedMatch)
    // A card's words, with how it looks here
    function cardWords(id, look) {
        var words = byId(boardWords.cards, id)
        look.title = fill(words.title)
        look.text = fill(words.text)
        if (words.action) look.action = words.action
        return look
    }
    // The board step's card: { icon, tint, spin, size, title, text, action, actionPrimary }
    function cardFor(phase, confirmed, match, mundane) {
        var green = isMacOS ? MacColors.green : "#32d74b"
        var orange = isMacOS ? MacColors.orange : "#ff9f0a"
        switch (phase) {
        case "idle": case "waitingForBoard":
            if (confirmed) return cardWords("card.looking", { spin: true })
            if (match === 1) return cardWords("card.already_installed", { icon: "check-circle", tint: green, size: 36 })
            if (match === 2) return cardWords("card.update_available", { icon: "arrow-up", actionPrimary: true })
            if (match === 3) return cardWords("card.downgrade", { icon: "arrow-down-circle", tint: orange, actionPrimary: false })
            return cardWords("card.waiting", { spin: true })
        // -- WP QD11 --
        case "checking":
            return cardWords("card.checking", { spin: true })
        case "waitingForDrive":
            return cardWords("card.waiting_for_drive", { spin: true })
        case "unconfirmed":
            return cardWords("card.unconfirmed", { icon: "question-circle", tint: orange })
        case "inBootloader":
            return cardWords("card.in_bootloader", { icon: "drive", tint: orange, actionPrimary: true })
        case "waitingForVolume":
            return cardWords("card.volume_wait", { spin: true })
        case "ready":
            if (confirmed) return cardWords("card.preparing", { spin: true })
            return cardWords("card.ready", { icon: "drive", tint: green, actionPrimary: true })
        case "waitingForDevice":
            return cardWords("card.restarting", { spin: true })
        case "verified":
            return cardWords("card.verified", { icon: "check-circle", tint: green, size: 36 })
        case "failed":
            return cardWords(mundane ? "card.failed_mundane" : "card.failed", { icon: mundane ? "question-circle" : "warning", tint: orange,
                             action: buttons.try_again, actionPrimary: false })
        default:
            return { title: "", text: "" }
        }
    }
    readonly property var card: cardFor(phase, confirmed, connectedMatch, firmware.failureMundane)
    function cardAction() {
        if (phase === "failed") resetRun()
        else if (phase === "ready") beginInstall()
        else beginConnectedInstall()
    }
    // Continue on the board step only once there is nothing left to do there
    readonly property bool showsContinue: stage !== 1 || phase === "verified" || phase === "unconfirmed" || connectedMatch === 1   // -- WP QD11 -- unconfirmed

    component InfoRow: Row {
        property string icon: ""
        property string text: ""
        width: parent ? parent.width : 400
        spacing: 10
        Icon { name: parent.icon; size: 16; color: isMacOS ? MacColors.accent : "#3a96ff" }
        Text {
            width: parent.width - 26
            wrapMode: Text.WordWrap
            text: parent.text
            font.pixelSize: 12
            color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.85)
        }
    }

    // ── Header ──
    Item {
        id: header
        width: parent.width
        height: 104
        Rectangle {
            id: tile
            x: 24
            y: 14
            width: 28; height: 28; radius: 7
            color: isMacOS ? "transparent" : "#0a7cff"
            Icon { anchors.centerIn: parent; name: "cap"; size: 18; color: isMacOS ? MacColors.accent : "white" }
        }
        Column {
            anchors.left: tile.right
            anchors.leftMargin: 10
            anchors.verticalCenter: tile.verticalCenter
            spacing: 1
            Text { text: wiz.words.title || ""; font.pixelSize: 14; font.weight: Font.DemiBold; color: isMacOS ? MacColors.label : "white" }
            Text { text: (wiz.words.step_counter || "").replace("{current}", wiz.stage + 1).replace("{total}", wiz.stages.length); font.pixelSize: 11; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5) }
        }
        StepStrip {
            anchors.horizontalCenter: parent.horizontalCenter
            y: 56
            width: Math.min(420, parent.width - 48)
            labels: wiz.stages
            current: wiz.stage
        }
        Rectangle { anchors.bottom: parent.bottom; width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.07) }
    }

    // ── Content ──
    Flickable {
        id: scroller
        anchors.top: header.bottom
        anchors.bottom: footer.top
        width: parent.width
        contentHeight: content.height + 56
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {}

        Column {
            id: content
            y: 28
            width: Math.min(560, scroller.width - 56)
            x: (scroller.width - width) / 2
            spacing: 16

            Text {
                width: parent.width
                wrapMode: Text.WordWrap
                text: (wiz.stage === 0 ? wiz.welcomeWords.title : wiz.stage === 1 ? wiz.boardHeader.title : wiz.doneWords.title) || ""
                font.pixelSize: 20
                font.weight: Font.DemiBold
                color: isMacOS ? MacColors.label : "white"
            }
            Text {
                width: parent.width
                wrapMode: Text.WordWrap
                text: wiz.stage === 0 ? wiz.joined(wiz.welcomeWords.paragraphs)
                      : wiz.stage === 1 ? wiz.boardHeader.blurb || ""
                      : wiz.joined(wiz.doneWords.paragraphs)
                font.pixelSize: 13
                lineHeight: 1.15
                color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.6)
            }

            // Welcome
            Column {
                visible: wiz.stage === 0
                width: parent.width
                spacing: 12
                Repeater {
                    model: wiz.welcomeWords.rows
                    InfoRow { icon: wiz.iconFor(modelData.icon); text: modelData.text }
                }
            }

            // Board: the install happens in place
            Item {
                visible: wiz.stage === 1
                width: parent.width
                height: 210
                InstallStateCard {
                    anchors.fill: parent
                    visible: wiz.phase !== "writing" && wiz.phase !== "question"   // -- WP QD11 -- question
                    icon: wiz.card.icon || ""
                    tint: isMacOS ? wiz.card.tint || MacColors.accent : wiz.card.tint || "#3a96ff"
                    spin: wiz.card.spin === true
                    iconSize: wiz.card.size || 28
                    title: wiz.card.title
                    text: wiz.card.text
                    AppButton {
                        visible: !!wiz.card.action
                        text: wiz.card.action || ""
                        primary: wiz.card.actionPrimary === true
                        onClicked: wiz.cardAction()
                    }
                }
                // -- WP QD11 --
                FirmwareQuestion {
                    anchors.fill: parent
                    visible: wiz.phase === "question"
                }
                InstallWritingCard {
                    anchors.fill: parent
                    visible: wiz.phase === "writing"
                    progress: firmware.progress
                    boardName: firmware.chipName
                    version: wiz.bundled
                    title: wiz.byId(wiz.boardWords.cards, "card.writing").title || ""
                    note: wiz.byId(wiz.boardWords.cards, "card.writing").text || ""
                }
            }

            // Done
            Column {
                visible: wiz.stage === 2
                width: parent.width
                spacing: 12
                Repeater {
                    model: wiz.doneWords.rows
                    InfoRow { icon: wiz.iconFor(modelData.icon); text: modelData.text }
                }
            }
        }
    }

    // ── Footer ──
    Item {
        id: footer
        anchors.bottom: parent.bottom
        width: parent.width
        height: 56
        Rectangle { width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.07) }
        // Always there, one click, and it never comes back
        AppButton {
            x: 24
            anchors.verticalCenter: parent.verticalCenter
            text: wiz.buttons.skip || ""
            onClicked: wiz.finished()
        }
        Row {
            anchors.right: parent.right
            anchors.rightMargin: 24
            anchors.verticalCenter: parent.verticalCenter
            spacing: 8
            AppButton {
                visible: wiz.stage > 0 && !wiz.installing
                text: wiz.buttons.back || ""
                onClicked: wiz.stage--
            }
            AppButton {
                visible: wiz.showsContinue
                text: (wiz.stage === wiz.stages.length - 1 ? wiz.buttons.finish : wiz.buttons["continue"]) || ""
                // The install button owns the emphasis while it is on screen
                primary: !(wiz.stage === 1 && !!wiz.card.action && wiz.card.actionPrimary)
                onClicked: wiz.advance()
            }
        }
    }
}
