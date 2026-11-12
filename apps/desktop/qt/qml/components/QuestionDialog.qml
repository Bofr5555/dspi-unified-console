import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// A question that needs an answer before anything changes (the Mac's
// confirmation alerts: unsaved changes, the PDM / Core 1 questions, factory
// reset). Up to three answers: `acceptText` (the default, Enter),
// `rejectText` (Escape) and an optional `alternativeText` ("Don't Save").
// `answered(answer)` gives "accept", "reject" or "alternative".
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
Popup {
    id: dialog
    property string title
    property string text
    property string acceptText: qsTr("OK")
    property string rejectText: qsTr("Cancel")
    property string alternativeText
    property bool destructive: false
    signal answered(string answer)

    // Escape and a close from outside answer "reject"
    property bool _answered: false
    function answer(a) {
        _answered = true
        close()
        answered(a)
    }

    parent: Overlay.overlay
    anchors.centerIn: parent
    width: Math.min(420, parent ? parent.width - 40 : 420)
    modal: true
    focus: true
    padding: Theme.spacingLarge
    closePolicy: Popup.CloseOnEscape
    onClosed: if (!_answered) answer("reject")
    onOpened: {
        _answered = false
        acceptButton.forceActiveFocus()
    }

    Overlay.modal: Rectangle { color: Qt.rgba(0, 0, 0, 0.35) }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.width: 1
        border.color: Theme.border
    }

    // A Popup is not an Item: its content carries the accessible role and name
    contentItem: ColumnLayout {
        spacing: Theme.spacing
        Accessible.role: Accessible.AlertMessage
        Accessible.name: dialog.title
        Accessible.description: dialog.text
        Text {
            Layout.fillWidth: true
            text: dialog.title
            font: Theme.fontTitle
            color: Theme.text
            wrapMode: Text.Wrap
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
        Text {
            Layout.fillWidth: true
            visible: dialog.text.length > 0
            text: dialog.text
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.topMargin: Theme.spacingSmall
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "QuestionDialog.alternative"
                visible: dialog.alternativeText.length > 0
                text: dialog.alternativeText
                onClicked: dialog.answer("alternative")
            }
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "QuestionDialog.reject"
                text: dialog.rejectText
                onClicked: dialog.answer("reject")
            }
            AppButton {
                id: acceptButton
                objectName: "QuestionDialog.accept"
                text: dialog.acceptText
                primary: !dialog.destructive
                destructive: dialog.destructive
                onClicked: dialog.answer("accept")
                Keys.onReturnPressed: dialog.answer("accept")
                Keys.onEnterPressed: dialog.answer("accept")
            }
        }
    }
}
