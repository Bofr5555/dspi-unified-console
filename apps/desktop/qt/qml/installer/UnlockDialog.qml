import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP P1QT2 -- The one unlock prompt (P1LK): the installer's PIN or the
// recovery code. One line says where the lock is: on this console, the DSPi
// itself open (OD-P1d), or on the hub the DSPi is reached through
// (LimitsController.scopeText, the core's notice row). The PIN is checked on
// the core's worker; a wrong one says so here, with the wait the core asks.
// Opened only when a PIN is set: with none there is nothing to ask.
Dialog {
    id: dialog
    objectName: "UnlockDialog"
    title: LimitsController.heldByHub ? qsTr("Unlock on the hub") : qsTr("Unlock this console")

    property bool asked: false
    function ask() {
        pin.text = ""
        asked = false
        open()
    }
    onOpened: pin.forceActiveFocus()

    Connections {
        target: LimitsController
        function onUnlockFinished(ok) {
            if (!dialog.opened) return
            if (ok) dialog.close()
            else {
                pin.selectAll()
                pin.forceActiveFocus()
            }
        }
    }

    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
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
    header: Item {
        implicitHeight: titleText.height
        Text {
            id: titleText
            width: parent.width
            leftPadding: Theme.spacingLarge
            rightPadding: Theme.spacingLarge
            topPadding: Theme.spacingLarge
            textFormat: Text.PlainText
            text: dialog.title
            font: Theme.fontTitle
            color: Theme.text
            wrapMode: Text.Wrap
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
    }

    contentItem: ColumnLayout {
        spacing: Theme.spacing
        Text {
            objectName: "UnlockDialog.scope"
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: LimitsController.scopeText
            font: Theme.fontBody
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        BorderedField {
            id: pin
            objectName: "UnlockDialog.pin"
            Layout.fillWidth: true
            label: qsTr("Installer PIN or recovery code")
            placeholderText: qsTr("PIN or recovery code")
            echoMode: TextInput.Password
            enabled: !LimitsController.busy
            onAccepted: unlockButton.clicked()
        }
        Text {
            objectName: "UnlockDialog.error"
            Layout.fillWidth: true
            visible: text.length > 0
            textFormat: Text.PlainText
            text: LimitsController.pinError
            font: Theme.fontSmall
            color: Theme.dangerText
            wrapMode: Text.Wrap
        }
    }

    footer: Item {
        implicitHeight: answers.implicitHeight + 2 * Theme.spacing
        Row {
            id: answers
            x: parent.width - width - Theme.spacingLarge
            y: Theme.spacing
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "UnlockDialog.cancel"
                text: qsTr("Cancel")
                onClicked: dialog.close()
            }
            AppButton {
                id: unlockButton
                objectName: "UnlockDialog.unlock"
                text: qsTr("Unlock")
                primary: true
                enabled: pin.text.length > 0 && !LimitsController.busy
                onClicked: {
                    dialog.asked = true
                    LimitsController.unlock(pin.text)
                }
            }
        }
    }
}
