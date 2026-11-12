import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A question that needs an answer before anything changes (the Mac's
// confirmation alerts: unsaved changes, the PDM / Core 1 questions, factory
// reset). Up to three answers: `acceptText` (the default, Enter),
// `rejectText` (Escape, or closing it) and an optional `alternativeText`
// ("Don't Save"). `answered(answer)` gives "accept", "reject" or "alternative",
// exactly once per question.
//
//     QuestionDialog {
//         id: ask
//         title: qsTr("Save changes?")
//         text: qsTr("...")
//         acceptText: qsTr("Save")
//         alternativeText: qsTr("Don't Save")
//         onAnswered: answer => ...
//     }
//     ask.open()
Dialog {
    id: dialog
    property string text
    property string acceptText: qsTr("OK")
    property string rejectText: qsTr("Cancel")
    property string alternativeText
    property bool destructive: false
    signal answered(string answer)

    // Set when a button answered, so the close that follows is not taken
    // for a refusal
    property string pending
    function finish(answer) {
        pending = answer
        close()
    }
    onAboutToShow: pending = ""
    onOpened: acceptButton.forceActiveFocus()
    onClosed: answered(pending.length ? pending : "reject")

    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    // Centred in the window it belongs to
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 320
    width: Math.min(420, hostWidth - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: Math.max(Theme.spacingLarge, (hostHeight - implicitHeight) / 3)
    padding: Theme.spacingLarge
    topPadding: Theme.spacingSmall

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.color: Theme.border
    }

    header: Text {
        leftPadding: Theme.spacingLarge
        rightPadding: Theme.spacingLarge
        topPadding: Theme.spacingLarge
        text: dialog.title
        font: Theme.fontTitle
        color: Theme.text
        wrapMode: Text.Wrap
        Accessible.role: Accessible.Heading
        Accessible.name: text
    }

    // A Popup is not an Item: the body carries the dialog's accessible role
    contentItem: Text {
        text: dialog.text
        font: Theme.fontBody
        color: Theme.text
        wrapMode: Text.Wrap
        Accessible.role: Accessible.AlertMessage
        Accessible.name: dialog.title
        Accessible.description: dialog.text
    }

    footer: Item {
        implicitHeight: answers.implicitHeight + 2 * Theme.spacing
        AppButton {
            objectName: "QuestionDialog.alternative"
            x: Theme.spacingLarge
            y: Theme.spacing
            visible: dialog.alternativeText.length > 0
            text: dialog.alternativeText
            onClicked: dialog.finish("alternative")
        }
        Row {
            id: answers
            x: parent.width - width - Theme.spacingLarge
            y: Theme.spacing
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "QuestionDialog.reject"
                text: dialog.rejectText
                onClicked: dialog.finish("reject")
            }
            AppButton {
                id: acceptButton
                objectName: "QuestionDialog.accept"
                text: dialog.acceptText
                primary: !dialog.destructive
                destructive: dialog.destructive
                onClicked: dialog.finish("accept")
                Keys.onReturnPressed: dialog.finish("accept")
                Keys.onEnterPressed: dialog.finish("accept")
            }
        }
    }
}
