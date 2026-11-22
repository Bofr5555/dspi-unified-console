import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A question that needs an answer before anything changes (the Mac's
// confirmation alerts: unsaved changes, the PDM / Core 1 questions, factory
// reset). Up to three answers: `acceptText` (the default, Enter),
// `rejectText` (Escape, or closing it) and an optional `alternativeText`
// ("Don't Save"). `answered(answer)` gives "accept", "reject" or "alternative",
// exactly once per question. An empty `rejectText` leaves one button, for a
// notice that only needs reading (-- WP SECQT --).
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
    // -- WP MDQT2a -- It is about the active device: refused when another one
    // becomes active, since its answer would go there
    property bool ofDevice: false
    signal answered(string answer)

    // Set when a button answered, so the close that follows is not taken
    // for a refusal
    property string pending
    function finish(answer) {
        pending = answer
        close()
    }
    Connections {   // -- WP MDQT2a --
        target: dialog.ofDevice && dialog.visible ? DeviceRegistry : null
        function onActiveLeaving() { dialog.finish("reject") }
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

    // The title and the body are Items sized by their Text's laid-out
    // height. A wrapped Text's implicitHeight is computed on first read and
    // changes while it is read, so the dialog's implicitHeight binding would
    // loop on a text given at creation.
    header: Item {
        implicitHeight: titleText.height
        Text {
            id: titleText
            width: parent.width
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
    }

    // A Popup is not an Item: the body carries the dialog's accessible role
    contentItem: Item {
        implicitHeight: bodyText.height
        Accessible.role: Accessible.AlertMessage
        Accessible.name: dialog.title
        Accessible.description: dialog.text
        Text {
            id: bodyText
            width: parent.width
            text: dialog.text
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
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
                visible: dialog.rejectText.length > 0   // -- WP SECQT -- an OK-only notice
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
