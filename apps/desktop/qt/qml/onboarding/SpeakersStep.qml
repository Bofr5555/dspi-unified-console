import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP ROLEQ -- The Getting Started step "Which output drives what?"
// (assets/onboarding/speakers.json): the proposal, or the stored layout,
// pre-filled; Confirm keeps it as the person's layout. GettingStarted shows
// the stage's title and paragraphs above it.
// -- WP UA4 -- (UXP A4, owner D15) The wizard's footer is the one primary:
// Later while the table is as it came, Confirm and Continue once the person
// edited it (`touched`); Confirm here is the quiet way to keep it and stay.
ColumnLayout {
    id: step
    objectName: "SpeakersStep"
    property var stage: ({})
    readonly property var sc: SpeakersController
    readonly property var buttons: stage.buttons || {}
    readonly property bool confirmed: sc.status === "confirmed" && !sc.edited
    // -- WP UA4 -- the person changed the table on this step since it was
    // last kept (or since the step showed)
    property bool touched: false
    onConfirmedChanged: if (confirmed) touched = false
    Connections {
        target: DeviceRegistry
        function onActiveIdChanged() { step.touched = false }
    }
    spacing: Theme.spacing

    Component.onCompleted: sc.reload()

    Text {
        objectName: "SpeakersStep.noDevice"
        visible: !DeviceModel.connected
        Layout.fillWidth: true
        text: step.stage.no_device || ""
        font: Theme.fontBody
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
    SpeakersTable {
        visible: DeviceModel.connected
        Layout.fillWidth: true
        onChangedByUser: step.touched = true   // -- WP UA4 --
    }
    Text {
        objectName: "SpeakersStep.state"
        visible: DeviceModel.connected
        Layout.fillWidth: true
        textFormat: Text.PlainText
        text: step.confirmed ? step.stage.confirmed || "" : step.sc.saidError ? step.sc.said : step.sc.bannerText(true)
        font: Theme.fontBody
        color: step.sc.saidError && !step.confirmed ? Theme.dangerText : step.confirmed ? Theme.success : Theme.textMuted
        wrapMode: Text.Wrap
    }
    RowLayout {
        visible: DeviceModel.connected
        spacing: Theme.spacingSmall
        AppButton {
            objectName: "SpeakersStep.confirm"
            text: step.buttons.confirm || ""
            enabled: step.sc.editable && step.sc.loaded && !step.confirmed
            onClicked: step.sc.confirm()
        }
        AppButton {
            objectName: "SpeakersStep.suggestion"
            text: step.buttons.suggestion || ""
            enabled: step.sc.editable && step.sc.loaded
            onClicked: {
                step.sc.useSuggestion()
                step.touched = true   // -- WP UA4 -- the person chose it
            }
        }
        // -- INT016 -- (UB1's follow-up, UXP B1) the next step once the layout
        // is confirmed, as Settings › Speakers offers it
        AppButton {
            objectName: "SpeakersStep.fromLayout"
            visible: step.confirmed
            text: qsTr("Crossovers and Routing…")
            enabled: LayoutSetupController.writable
            toolTip: qsTr("Starting crossovers and routes from this layout, in one undo step")
            onClicked: fromLayout.open()
        }
    }
    Text {
        objectName: "SpeakersStep.later"   // -- WP UA4 --
        visible: !step.confirmed && DeviceModel.connected
        Layout.fillWidth: true
        text: step.stage.later || ""
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }

    // -- INT016 --
    CrossoverFromLayout {
        id: fromLayout
        parent: step.Window.window ? step.Window.window.contentItem : null
        crossoversWanted: true
        routingWanted: true
    }
}
