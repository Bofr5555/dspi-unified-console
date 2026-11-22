import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP ROLEQ -- The Getting Started step "Which output drives what?"
// (assets/onboarding/speakers.json): the proposal, or the stored layout,
// pre-filled; Confirm keeps it as the person's layout. GettingStarted shows
// the stage's title and paragraphs above it.
ColumnLayout {
    id: step
    objectName: "SpeakersStep"
    property var stage: ({})
    readonly property var sc: SpeakersController
    readonly property var buttons: stage.buttons || {}
    readonly property bool confirmed: sc.status === "confirmed" && !sc.edited
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
    }
    Text {
        objectName: "SpeakersStep.state"
        visible: DeviceModel.connected
        Layout.fillWidth: true
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
            primary: !step.confirmed
            enabled: step.sc.editable && step.sc.loaded && !step.confirmed
            onClicked: step.sc.confirm()
        }
        AppButton {
            objectName: "SpeakersStep.suggestion"
            text: step.buttons.suggestion || ""
            enabled: step.sc.editable && step.sc.loaded
            onClicked: step.sc.useSuggestion()
        }
    }
    Text {
        visible: !step.confirmed
        Layout.fillWidth: true
        text: step.stage.later || ""
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
}
