import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// The hub asks for its PIN (the Mac's PairingPrompt, the iOS pairing sheet).
// It opens by itself while the session's phase is needs_pairing, over the
// window, and does not hold the app up: the window behind it keeps updating.
// Six digits or nothing reaches the hub; a wrong PIN leaves it open with the
// core's reason under the field. Cancel leaves the hub.
// -- WP SECQT -- Over TLS it shows the key the hub presented, to compare with
// the one the hub shows beside its PIN; the pairing pins that key. Without
// TLS it says the PIN crosses the network unencrypted, in red and in words.
Dialog {
    id: dialog
    objectName: "PairingDialog"
    title: qsTr("Pair with %1").arg(LinkController.hubName.length ? LinkController.hubName : qsTr("the hub"))

    readonly property bool asked: LinkController.phase === "needs_pairing"
    readonly property bool complete: /^[0-9]{6}$/.test(pin.text)

    // Pair, if the PIN is whole (Enter, the button)
    function submit() {
        if (!complete || LinkController.busy) return
        LinkController.pair(pin.text)
    }

    onAskedChanged: asked ? open() : close()
    Component.onCompleted: if (asked) open()
    onAboutToShow: pin.text = ""
    onOpened: pin.forceActiveFocus()
    onRejected: if (asked) LinkController.closeSession()

    Connections {
        target: LinkController
        // A refused PIN: the field empties for another try
        function onErrorChanged() {
            if (LinkController.errorCode === "wrong_pin") pin.text = ""
        }
    }

    modal: true
    focus: true
    closePolicy: Popup.CloseOnEscape
    readonly property real hostWidth: parent ? parent.width : 480
    readonly property real hostHeight: parent ? parent.height : 320
    width: Math.min(400, hostWidth - 2 * Theme.spacingLarge)
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

    header: RowLayout {
        spacing: Theme.spacingSmall
        Item { implicitWidth: Theme.spacingLarge - Theme.spacingSmall }
        Icon {
            Layout.topMargin: Theme.spacingLarge
            name: "key-round"
            color: Theme.accentText
        }
        Text {
            Layout.fillWidth: true
            Layout.topMargin: Theme.spacingLarge
            Layout.rightMargin: Theme.spacingLarge
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
        Accessible.role: Accessible.Dialog
        Accessible.name: dialog.title

        // The core's sentence for this phase: where the PIN is to be found
        Text {
            objectName: "PairingDialog.text"
            Layout.fillWidth: true
            text: LinkController.phaseText
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        // -- WP SECQT -- the hub's key, before the PIN goes out
        ColumnLayout {
            objectName: "PairingDialog.key"
            Layout.fillWidth: true
            visible: LinkController.encrypted && LinkController.hubKey.length > 0
            spacing: 2
            Text {
                Layout.fillWidth: true
                text: qsTr("Hub key")
                font: Theme.fontSmall
                color: Theme.textMuted
            }
            TextEdit {
                objectName: "PairingDialog.hubKey"
                Layout.fillWidth: true
                text: LinkController.hubKey
                readOnly: true
                selectByMouse: true
                font: Qt.font({ family: Shell.monospaceFamily, pixelSize: 15 })
                color: Theme.text
                selectionColor: Theme.selection
                selectedTextColor: Theme.text
                wrapMode: TextEdit.WrapAnywhere
                Accessible.role: Accessible.StaticText
                Accessible.name: qsTr("Hub key")
                Accessible.description: text
            }
            Text {
                Layout.fillWidth: true
                text: qsTr("Check that the hub shows the same key next to its PIN.")
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
        }
        RowLayout {
            objectName: "PairingDialog.unsecured"
            Layout.fillWidth: true
            visible: LinkController.unsecured
            spacing: 2
            Text {
                Layout.fillWidth: true
                text: qsTr("This connection is not encrypted (%1): the PIN crosses the network as plain text.")
                      .arg(LinkController.unsecuredLabel)
                font: Theme.fontSmall
                color: Theme.dangerText
                wrapMode: Text.Wrap
            }
            UnsecuredInfoButton {
                objectName: "PairingDialog.unsecuredInfo"
            }
        }
        TextField {
            id: pin
            objectName: "PairingDialog.pin"
            Layout.alignment: Qt.AlignHCenter
            implicitWidth: 180
            implicitHeight: 40
            placeholderText: "000000"
            horizontalAlignment: TextInput.AlignHCenter
            font: Qt.font({ family: Shell.monospaceFamily, pixelSize: 24, letterSpacing: 4 })
            color: Theme.text
            placeholderTextColor: Theme.textMuted
            selectionColor: Theme.selection
            selectedTextColor: Theme.text
            maximumLength: 6
            inputMethodHints: Qt.ImhDigitsOnly
            validator: RegularExpressionValidator { regularExpression: /[0-9]{0,6}/ }
            enabled: !LinkController.busy
            onAccepted: dialog.submit()
            background: Rectangle {
                radius: Theme.radiusSmall
                color: Theme.control
                border.width: pin.activeFocus ? 2 : 1
                border.color: pin.activeFocus ? Theme.focusRing : Theme.border
            }
            Accessible.role: Accessible.EditableText
            Accessible.name: qsTr("Pairing code")
        }
        Text {
            objectName: "PairingDialog.error"
            Layout.fillWidth: true
            visible: LinkController.errorText.length > 0
            text: LinkController.errorText
            font: Theme.fontSmall
            color: Theme.warning
            wrapMode: Text.Wrap
            horizontalAlignment: Text.AlignHCenter
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
                objectName: "PairingDialog.cancel"
                text: qsTr("Cancel")
                onClicked: dialog.reject()
            }
            AppButton {
                objectName: "PairingDialog.pair"
                text: qsTr("Pair")
                primary: true
                enabled: dialog.complete && !LinkController.busy
                onClicked: dialog.submit()
            }
        }
    }
}
