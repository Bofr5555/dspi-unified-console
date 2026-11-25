import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP17 -- The Getting Started wizard (the Mac's GettingStartedView). It
// takes the main window's place of the console while
// OnboardingController.wizardShown, so the menus stay live and there is one
// window. Its one goal is a board running the firmware this build ships; the
// install happens on the board step itself. Every word comes from the core's
// getting_started.json (OnboardingController.gettingStarted), resolved for
// this platform; `{name}` holes are filled here.
// -- WP UA4 -- (UXP A4) No dead end: the board step has a way on without a
// board ("My DSPi already runs the firmware…"); the Speakers step's one
// primary is Later until the table is edited, then Confirm and Continue
// (owner D15); rows may carry an action (the demo, Settings › This Computer)
// and a layout (OnboardingController.h).
Rectangle {
    id: wizard
    objectName: "GettingStarted"
    color: Theme.window

    readonly property var words: OnboardingController.gettingStarted
    readonly property var stages: words.stages || []
    property int current: 0
    readonly property var stage: stages[current] || ({})
    readonly property bool onBoard: stage.id === "board"
    readonly property bool onLast: current === stages.length - 1
    // The Mac's step column: at most 560 wide, 28 around it
    readonly property int columnWidth: 560
    readonly property int gutter: 28

    // ---- the board step's state (the Mac's, over the firmware package's
    // installer when there is one: OnboardingController.h) ----
    readonly property var installer: OnboardingController.installer
    readonly property string installState: installer && installer.state ? installer.state : "idle"
    // The user's one decision to install; nothing is written before it
    property bool installChosen: false
    // The open device was told to restart into its bootloader
    property bool rebootAsked: false
    readonly property bool idleish: installState === "idle" || installState === "waitingForBoard"
    // "match", "older", "newer" for a connected board running firmware, while
    // nothing is under way; "" otherwise
    readonly property string match: !installChosen && idleish && DeviceModel.connected
                                    ? OnboardingController.firmwareMatch(DeviceModel.firmwareVersion) : ""
    readonly property bool installInFlight: installState === "writing" || installState === "waitingForDevice"
    // -- WP QPI3 -- how much is written, when the installer knows (the core
    // writes in one call: then there is no bar, and the card's glyph turns)
    readonly property real written: installState === "writing" && installer ? Number(installer.fraction) || 0 : 0
    readonly property bool wrote: written > 0
    readonly property string bundledVersion: OnboardingController.bundledFirmware || qsTr("unknown")
    readonly property var holes: ({
        chip: installer && installer.chip ? installer.chip : "",
        volume: installer && installer.volume ? installer.volume : "",
        version: installer && installer.version ? installer.version : bundledVersion,
        bundled_version: bundledVersion,
        device_version: DeviceModel.firmwareVersion || qsTr("unknown"),
        failure: installer && installer.failure ? installer.failure : "",
        current: current + 1,
        total: stages.length
    })

    function fill(text) {
        return (text || "").replace(/\{([a-z_]+)\}/g, (all, name) => holes[name] !== undefined ? String(holes[name]) : all)
    }
    function byId(list, id) {
        for (const item of list || []) if (item.id === id) return item
        return ({})
    }
    function wording(item) { return typeof item === "string" ? item : item.text || "" }
    function iconFor(name) {
        return ({ connect: "plug-zap", verified: "badge-check", sliders: "sliders-horizontal", outputs: "cable",
                  speaker: "volume-2", help: "circle-question-mark",
                  // -- WP UA4 --
                  measure: "mic", agent: "bot", demo: "play", checklist: "circle-check" })[name] || "info"
    }
    // -- WP UA4 -- the rows of this layout: one that names a layout shows in it only
    readonly property string layoutName: LayoutController.modern ? "modern" : "classic"
    function shownRows(list) {
        return (list || []).filter(r => typeof r === "string" || !r.layout || r.layout === layoutName)
    }
    // -- WP UA4 -- a row's button: what it does, and whether it is offered now
    function rowOffered(row) {
        if (typeof row === "string" || !row.action) return false
        if (row.id === "welcome.demo") return !DeviceModel.connected
        return true
    }
    function rowAction(row) {
        if (row.id === "welcome.demo") {
            DemoController.openDemo()
        } else if (row.id === "done.sound_pc") {
            // The last step: the console comes back, on Settings › This Computer
            OnboardingController.finishSetup()
            SettingsController.showPage("hostaudio")
        }
    }

    // -- WP UA4 -- The Speakers step (owner D15: Later by default): Later
    // leaves the layout as it is; once the person edited the table the
    // primary confirms it and goes on, after the core kept it
    readonly property bool onSpeakers: stage.id === "speakers"
    readonly property var speakersStep: onSpeakers && speakersLoader.item ? speakersLoader.item : null
    readonly property string speakersAction: !speakersStep || !DeviceModel.connected || speakersStep.confirmed ? "continue"
                                           : speakersStep.touched ? "confirm_continue" : "later"
    property bool confirmThenGo: false
    Connections {
        target: SpeakersController
        enabled: wizard.confirmThenGo
        function onStoredChanged() { wizard.goOnIfConfirmed() }
        function onRowsChanged() { wizard.goOnIfConfirmed() }
        function onSaidChanged() { if (SpeakersController.saidError) wizard.confirmThenGo = false }
    }
    function goOnIfConfirmed() {
        if (!confirmThenGo || SpeakersController.status !== "confirmed" || SpeakersController.edited) return
        confirmThenGo = false
        current = current + 1
    }

    readonly property string headerId: {
        switch (installState) {
        case "verified": return "board.verified"
        case "failed": return "board.failed"
        case "writing": case "waitingForDevice": return "board.installing"
        case "question": return "board.question"
        case "inBootloader": return "board.in_bootloader"
        case "unconfirmed": return "board.unconfirmed"
        }
        return match === "match" ? "board.current" : match === "older" ? "board.older"
             : match === "newer" ? "board.newer" : "board.prepare"
    }
    readonly property var header: byId(stage.headers, headerId)

    // The card for the state: [card id, icon, tint, spinning, action ("install",
    // "connected", "")]
    readonly property var cardSpec: {
        switch (installState) {
        case "waitingForVolume": return ["card.volume_wait", "hard-drive", Theme.accentText, true, ""]
        case "ready": return installChosen ? ["card.preparing", "hard-drive", Theme.accentText, true, ""]
                                       : ["card.ready", "hard-drive", Theme.success, false, "install"]
        case "writing": return ["card.writing", "download", Theme.accentText, !wrote, ""]
        case "waitingForDevice": return ["card.restarting", "refresh-cw", Theme.accentText, true, ""]
        case "verified": return ["card.verified", "badge-check", Theme.success, false, ""]
        case "failed": return [installer && installer.mundane ? "card.failed_mundane" : "card.failed",
                               "triangle-alert", Theme.warning, false, ""]
        case "checking": case "question": return ["card.checking", "search", Theme.accentText, true, ""]
        case "inBootloader": return ["card.in_bootloader", "hard-drive", Theme.warning, false, "install"]
        case "unconfirmed": return ["card.unconfirmed", "circle-alert", Theme.warning, false, ""]
        }
        // -- WP QPI3 -- the open device restarts by itself: no BOOTSEL to hold
        if (installChosen && rebootAsked) return ["card.waiting_for_drive", "refresh-cw", Theme.accentText, true, ""]
        if (installChosen) return ["card.looking", "search", Theme.accentText, true, ""]
        if (match === "match") return ["card.already_installed", "badge-check", Theme.success, false, ""]
        if (match === "older") return ["card.update_available", "circle-arrow-up", Theme.accentText, false, "connected"]
        if (match === "newer") return ["card.downgrade", "circle-arrow-down", Theme.warning, false, "connected"]
        return ["card.waiting", "cable", Theme.accentText, true, ""]
    }
    readonly property var card: byId(stage.cards, cardSpec[0])

    // No Continue while the step still has work to do: "nothing is asked of
    // you" rather than a disabled button
    readonly property bool showsContinue: !onBoard || installState === "verified" || match === "match"
    // The card's install button owns Return while it shows
    readonly property bool continueIsDefault: !(onBoard && cardSpec[4] === "install" && !installChosen)

    function beginInstall() {
        if (!installer) {
            Shell.trigger("device.firmwareUpdate")
            return
        }
        installChosen = true
        installer.installWhenReady()
    }
    function installOnConnected() {
        if (!installer) {
            // Until the firmware package hands its installer over, the update
            // happens in its own window
            Shell.trigger("device.firmwareUpdate")
            return
        }
        installChosen = true
        installer.installWhenReady()
        if (rebootAsked) return
        rebootAsked = true
        installer.installConnected()
    }
    function cardAction() {
        if (cardSpec[4] === "connected") installOnConnected()
        else beginInstall()
    }
    function tryAgain() {
        installChosen = false
        rebootAsked = false
        if (installer) installer.reset()
    }
    function advance() {
        if (onLast) {
            OnboardingController.finishSetup()
        } else if (onSpeakers && speakersAction === "confirm_continue") {
            // -- WP UA4 -- kept first; a refusal stays on the step, in its words
            confirmThenGo = true
            SpeakersController.confirm()
        } else {
            current = current + 1
        }
    }
    onCurrentChanged: confirmThenGo = false   // -- WP UA4 --

    // -- WP QPI3 -- the installer this page began watching is the one it stops,
    // also when another is handed over meanwhile
    property var watched: null
    function watchInstaller() {
        if (watched === installer) return
        if (watched && watched.stopWatching) watched.stopWatching()
        watched = installer && installer.beginWatching ? installer : null
        if (watched) watched.beginWatching()
    }
    Component.onCompleted: watchInstaller()
    Component.onDestruction: if (watched && watched.stopWatching) watched.stopWatching()
    onInstallerChanged: watchInstaller()

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // Header: the cap, the title, "Step n of m", the dots
        ColumnLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            Layout.topMargin: 14
            Layout.bottomMargin: 14
            spacing: 14
            RowLayout {
                spacing: Theme.spacing
                Icon {
                    name: "graduation-cap"
                    size: 20
                    color: Theme.accentText
                }
                ColumnLayout {
                    spacing: 2
                    Text {
                        text: wizard.words.title || ""
                        font: Theme.fontTitle
                        color: Theme.text
                        Accessible.role: Accessible.Heading
                        Accessible.name: text
                    }
                    Text {
                        objectName: "GettingStarted.counter"
                        text: wizard.fill(wizard.words.step_counter)
                        font: Theme.fontSmall
                        color: Theme.textMuted
                    }
                }
            }
            // The dots, at most 460 wide, centred under the header
            Item {
                Layout.fillWidth: true
                implicitHeight: dotStrip.implicitHeight
                StepDots {
                    id: dotStrip
                    objectName: "GettingStarted.steps"
                    anchors.horizontalCenter: parent.horizontalCenter
                    width: Math.min(460, parent.width)
                    steps: wizard.stages.map(s => s.label || "")
                    at: wizard.current
                }
            }
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }

        Flickable {
            id: scroller
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            contentHeight: body.implicitHeight + 2 * wizard.gutter
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}

            ColumnLayout {
                id: body
                objectName: "GettingStarted.body"
                x: Math.max(wizard.gutter, (scroller.width - width) / 2)
                y: wizard.gutter
                width: Math.min(wizard.columnWidth, scroller.width - 2 * wizard.gutter)
                spacing: 18

                Text {
                    objectName: "GettingStarted.title"
                    Layout.fillWidth: true
                    text: wizard.onBoard ? wizard.header.title || "" : wizard.stage.title || ""
                    font: Qt.font({ pixelSize: 22, weight: Font.DemiBold })
                    color: Theme.text
                    wrapMode: Text.Wrap
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
                // The blurb: the board's header, or the stage's paragraphs
                Repeater {
                    model: wizard.onBoard ? [wizard.header.blurb || ""] : (wizard.stage.paragraphs || []).map(wizard.wording)
                    delegate: Text {
                        required property string modelData
                        objectName: "GettingStarted.paragraph"
                        Layout.fillWidth: true
                        text: modelData
                        font: Theme.fontBody
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                }
                ColumnLayout {
                    // -- WP ROLEQ -- none on a stage without rows (the Speakers
                    // step): an emptied column kept its old rows' height
                    visible: !wizard.onBoard && (wizard.stage.rows || []).length > 0
                    Layout.fillWidth: true
                    spacing: Theme.spacing
                    Repeater {
                        model: wizard.onBoard ? [] : wizard.shownRows(wizard.stage.rows)   // -- WP UA4 --
                        // -- WP UA4 -- a row with an action has its button at the right
                        delegate: RowLayout {
                            id: infoLine
                            required property var modelData
                            Layout.fillWidth: true
                            spacing: Theme.spacing
                            InfoRow {
                                objectName: "GettingStarted.row"
                                iconName: wizard.iconFor(infoLine.modelData.icon)
                                text: wizard.wording(infoLine.modelData)
                            }
                            AppButton {
                                objectName: "GettingStarted.rowAction." + (infoLine.modelData.id || "")
                                visible: wizard.rowOffered(infoLine.modelData)
                                Layout.alignment: Qt.AlignVCenter
                                text: infoLine.modelData.action || ""
                                onClicked: wizard.rowAction(infoLine.modelData)
                            }
                        }
                    }
                }
                // -- WP ROLEQ -- the Speakers step's table and its Confirm
                // (assets/onboarding/speakers.json, OnboardingController)
                Loader {
                    id: speakersLoader
                    objectName: "GettingStarted.speakers"
                    active: wizard.stage.id === "speakers"
                    visible: active
                    Layout.fillWidth: true
                    sourceComponent: Component {
                        SpeakersStep {
                            stage: wizard.stage
                        }
                    }
                }
                ColumnLayout {
                    visible: wizard.onBoard
                    Layout.fillWidth: true
                    Layout.minimumHeight: 190
                    spacing: Theme.spacing
                    InstallCard {
                        objectName: "GettingStarted.card"
                        property string cardId: wizard.cardSpec[0]
                        glyph: wizard.cardSpec[1]
                        tint: wizard.cardSpec[2]
                        busy: wizard.cardSpec[3]
                        glyphSize: cardId === "card.verified" || cardId === "card.already_installed" ? 36 : 28
                        heading: wizard.fill(wizard.card.title)
                        detail: wizard.fill(wizard.card.text)
                        actionText: wizard.cardSpec[4].length ? wizard.card.action || "" : ""
                        actionPrimary: cardId !== "card.downgrade"
                        progress: wizard.wrote ? wizard.written : -1
                        onAction: wizard.cardAction()
                    }
                    AppButton {
                        objectName: "GettingStarted.tryAgain"
                        visible: wizard.installState === "failed"
                        Layout.alignment: Qt.AlignHCenter
                        text: (wizard.words.buttons || {}).try_again || ""
                        onClicked: wizard.tryAgain()
                    }
                    // -- WP UA4 -- the way on without a board in front of it: a
                    // DSPi that runs the firmware, or none here yet
                    LinkButton {
                        objectName: "GettingStarted.later"
                        visible: !wizard.showsContinue && !wizard.installInFlight && text.length > 0
                        Layout.alignment: Qt.AlignHCenter
                        text: wizard.stage.later || ""
                        onClicked: wizard.current = wizard.current + 1
                    }
                }
            }
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }
        // Footer: Skip always, Back, Continue / the last step's button
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: 24
            Layout.rightMargin: 24
            Layout.topMargin: Theme.spacingLarge
            Layout.bottomMargin: Theme.spacingLarge
            spacing: Theme.spacingSmall
            // A skip that comes back next launch is not a skip
            AppButton {
                objectName: "GettingStarted.skip"
                text: (wizard.words.buttons || {}).skip || ""
                onClicked: OnboardingController.finishSetup()
            }
            Item {
                Layout.fillWidth: true
                implicitHeight: 1
            }
            AppButton {
                objectName: "GettingStarted.back"
                visible: wizard.current > 0 && !wizard.installInFlight
                text: (wizard.words.buttons || {}).back || ""
                onClicked: wizard.current = wizard.current - 1
            }
            AppButton {
                id: continueButton
                objectName: "GettingStarted.continue"
                visible: wizard.showsContinue
                primary: wizard.continueIsDefault
                // -- WP UA4 -- the Speakers step's own words: Later, Confirm and Continue
                text: wizard.onLast ? (wizard.words.buttons || {}).finish || ""
                    : wizard.onSpeakers && wizard.speakersAction !== "continue" ? (wizard.stage.buttons || {})[wizard.speakersAction] || ""
                    : (wizard.words.buttons || {}).continue || ""
                enabled: !wizard.confirmThenGo
                onClicked: wizard.advance()
            }
        }
    }

    Shortcut {
        sequences: ["Enter", "Return"]
        enabled: wizard.visible && wizard.showsContinue && wizard.continueIsDefault
        onActivated: wizard.advance()
    }
    // -- WP QPI3 -- the card's button is the default one while Continue is not
    // (the Mac's .defaultAction)
    Shortcut {
        sequences: ["Enter", "Return"]
        enabled: wizard.visible && wizard.onBoard && wizard.cardSpec[4].length > 0
                 && !(wizard.showsContinue && wizard.continueIsDefault)
        onActivated: wizard.cardAction()
    }

    // -- WP UA4 -- a quiet way on: accent text, underlined on hover, the focus ring
    component LinkButton: AbstractButton {
        id: link
        hoverEnabled: true
        focusPolicy: Qt.StrongFocus
        padding: 4
        contentItem: Text {
            textFormat: Text.PlainText
            text: link.text
            font.pixelSize: Theme.fontBody.pixelSize
            font.underline: link.hovered
            color: Theme.accentText
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
        }
        background: Rectangle {
            radius: Theme.radiusControl
            color: "transparent"
            border.width: link.visualFocus ? Theme.focusRingWidth : 0
            border.color: Theme.focusRing
        }
        Accessible.role: Accessible.Button
        Accessible.name: text
    }

    Accessible.role: Accessible.Pane
    Accessible.name: words.title || ""
}
