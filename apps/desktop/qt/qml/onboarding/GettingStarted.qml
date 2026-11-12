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
                  speaker: "volume-2", help: "circle-question-mark" })[name] || "info"
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
        case "writing": return ["card.writing", "download", Theme.accentText, false, ""]
        case "waitingForDevice": return ["card.restarting", "refresh-cw", Theme.accentText, true, ""]
        case "verified": return ["card.verified", "badge-check", Theme.success, false, ""]
        case "failed": return [installer && installer.mundane ? "card.failed_mundane" : "card.failed",
                               "triangle-alert", Theme.warning, false, ""]
        case "checking": case "question": return ["card.checking", "search", Theme.accentText, true, ""]
        case "inBootloader": return ["card.in_bootloader", "hard-drive", Theme.warning, false, "install"]
        case "unconfirmed": return ["card.unconfirmed", "circle-alert", Theme.warning, false, ""]
        }
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
    function tryAgain() {
        installChosen = false
        rebootAsked = false
        if (installer) installer.reset()
    }
    function advance() {
        if (onLast) OnboardingController.finishSetup()
        else current = current + 1
    }

    Component.onCompleted: if (installer && installer.beginWatching) installer.beginWatching()
    Component.onDestruction: if (installer && installer.stopWatching) installer.stopWatching()
    onInstallerChanged: if (installer && installer.beginWatching) installer.beginWatching()

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
                    visible: !wizard.onBoard
                    Layout.fillWidth: true
                    spacing: Theme.spacing
                    Repeater {
                        model: wizard.onBoard ? [] : wizard.stage.rows || []
                        delegate: InfoRow {
                            required property var modelData
                            objectName: "GettingStarted.row"
                            iconName: wizard.iconFor(modelData.icon)
                            text: wizard.wording(modelData)
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
                        progress: wizard.installState === "writing" && wizard.installer ? Number(wizard.installer.fraction) || 0 : -1
                        onAction: wizard.cardSpec[4] === "connected" ? wizard.installOnConnected() : wizard.beginInstall()
                    }
                    AppButton {
                        objectName: "GettingStarted.tryAgain"
                        visible: wizard.installState === "failed"
                        Layout.alignment: Qt.AlignHCenter
                        text: (wizard.words.buttons || {}).try_again || ""
                        onClicked: wizard.tryAgain()
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
                text: wizard.onLast ? (wizard.words.buttons || {}).finish || "" : (wizard.words.buttons || {}).continue || ""
                onClicked: wizard.advance()
            }
        }
    }

    Shortcut {
        sequences: ["Enter", "Return"]
        enabled: wizard.visible && wizard.showsContinue && wizard.continueIsDefault
        onActivated: wizard.advance()
    }

    Accessible.role: Accessible.Pane
    Accessible.name: words.title || ""
}
