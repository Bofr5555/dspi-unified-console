import QtQuick
import QtQuick.Controls.Basic
import UConsole

// AutoEQ > Update Database... (AutoEQMenuActions.updateDatabase): the
// database in use, and the ways to update it. Reset to Built-in shows only
// while an updated database is in use.
Dialog {
    id: dialog
    objectName: "AutoEqUpdateDialog"
    property string text
    // Rebuild from GitHub was chosen: the host asks to confirm it
    signal rebuildChosen()

    function ask() {
        text = AutoEqController.updateQuestionText()
        open()
    }

    title: qsTr("Update AutoEQ Database")
    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 320
    width: Math.min(360, hostWidth - 2 * Theme.spacingLarge)
    x: (hostWidth - width) / 2
    y: Math.max(Theme.spacingLarge, (hostHeight - implicitHeight) / 3)
    padding: Theme.spacingLarge
    topPadding: Theme.spacingSmall
    onOpened: rebuild.forceActiveFocus()

    Overlay.modal: Rectangle { color: Theme.scrim }
    background: Rectangle {
        radius: Theme.radius
        color: Theme.panel
        border.color: Theme.border
    }

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

    contentItem: Item {
        implicitHeight: body.height
        Accessible.role: Accessible.Dialog
        Accessible.name: dialog.title
        Accessible.description: dialog.text
        Text {
            id: body
            objectName: "AutoEqUpdateDialog.text"
            width: parent.width
            text: dialog.text
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
    }

    // Up to four answers, one per row as a tall alert stacks them
    footer: Item {
        implicitHeight: answers.implicitHeight + 2 * Theme.spacing
        Column {
            id: answers
            x: Theme.spacingLarge
            y: Theme.spacing
            width: parent.width - 2 * Theme.spacingLarge
            spacing: Theme.spacingSmall
            AppButton {
                id: rebuild
                objectName: "AutoEqUpdateDialog.rebuild"
                width: parent.width
                text: qsTr("Rebuild from GitHub")
                primary: true
                onClicked: {
                    dialog.close()
                    dialog.rebuildChosen()
                }
                Keys.onReturnPressed: clicked()
                Keys.onEnterPressed: clicked()
            }
            AppButton {
                objectName: "AutoEqUpdateDialog.import"
                width: parent.width
                text: qsTr("Import File…")
                onClicked: {
                    dialog.close()
                    AutoEqController.chooseDatabaseFile()
                }
            }
            AppButton {
                objectName: "AutoEqUpdateDialog.reset"
                width: parent.width
                visible: AutoEqController.updatedDatabase
                text: qsTr("Reset to Built-in")
                onClicked: {
                    dialog.close()
                    AutoEqController.resetToBuiltIn()
                }
            }
            AppButton {
                objectName: "AutoEqUpdateDialog.cancel"
                width: parent.width
                text: qsTr("Cancel")
                onClicked: dialog.close()
            }
        }
    }
}
