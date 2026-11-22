import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY5b -- What pairing with a hub asks (split from PairingDialog, with its
// rules): where the PIN is to be found, the hub's key to compare, whether the
// PIN crosses the network unencrypted, and the PIN. The dialog shows it, and
// Modern's Devices > Hubs in a card of its own. `names` is its items' prefix.
ColumnLayout {
    id: content
    property string names: "PairingDialog"
    readonly property bool complete: /^[0-9]{6}$/.test(pin.text)
    spacing: Theme.spacing

    // Pair, if the PIN is whole (Enter, the button)
    function submit() {
        if (!complete || LinkController.busy) return
        LinkController.pair(pin.text)
    }
    // The prompt comes up: "" but after a pairing link
    function start() { pin.text = LinkController.pairLinkPin }   // -- WP SECFIX --
    function takeFocus() { pin.forceActiveFocus() }

    Connections {
        target: LinkController
        // A refused PIN: the field empties for another try
        function onErrorChanged() {
            if (LinkController.errorCode === "wrong_pin") pin.text = ""
        }
    }

    // The core's sentence for this phase: where the PIN is to be found
    Text {
        objectName: content.names + ".text"
        Layout.fillWidth: true
        textFormat: Text.PlainText
        text: LinkController.phaseText
        font: Theme.fontBody
        color: Theme.text
        wrapMode: Text.Wrap
    }
    // -- WP SECQT -- the hub's key, before the PIN goes out
    ColumnLayout {
        objectName: content.names + ".key"
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
            objectName: content.names + ".hubKey"
            Layout.fillWidth: true
            text: LinkController.hubKey
            readOnly: true
            selectByMouse: true
            font: Qt.font({ family: Shell.monospaceFamily, pixelSize: 15 })
            color: Theme.text
            selectionColor: Theme.textSelection
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
        objectName: content.names + ".unsecured"
        Layout.fillWidth: true
        visible: LinkController.unsecured
        spacing: 2
        Text {
            Layout.fillWidth: true
            textFormat: Text.PlainText
            text: qsTr("This connection is not encrypted (%1): the PIN crosses the network as plain text.")
                  .arg(LinkController.unsecuredLabel)
            font: Theme.fontSmall
            color: Theme.dangerText
            wrapMode: Text.Wrap
        }
        UnsecuredInfoButton {
            objectName: content.names + ".unsecuredInfo"
        }
    }
    // -- WP SECFIX -- the PIN came with a pairing link: say so, and what Pair means
    Text {
        objectName: content.names + ".fromLink"
        Layout.fillWidth: true
        visible: LinkController.pairLinkPin.length > 0
        text: qsTr("The PIN comes from the pairing link. Pair if this is your hub and the key matches.")
        font: Theme.fontSmall
        color: Theme.textMuted
        wrapMode: Text.Wrap
    }
    TextField {
        id: pin
        objectName: content.names + ".pin"
        Layout.alignment: Qt.AlignHCenter
        implicitWidth: 180
        implicitHeight: 40
        placeholderText: "000000"
        horizontalAlignment: TextInput.AlignHCenter
        font: Qt.font({ family: Shell.monospaceFamily, pixelSize: 24, letterSpacing: 4 })
        color: Theme.text
        placeholderTextColor: Theme.textMuted
        selectionColor: Theme.textSelection
        selectedTextColor: Theme.text
        maximumLength: 6
        inputMethodHints: Qt.ImhDigitsOnly
        validator: RegularExpressionValidator { regularExpression: /[0-9]{0,6}/ }
        enabled: !LinkController.busy
        onAccepted: content.submit()
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
        objectName: content.names + ".error"
        Layout.fillWidth: true
        visible: LinkController.errorText.length > 0
        textFormat: Text.PlainText
        text: LinkController.errorText
        font: Theme.fontSmall
        color: Theme.warning
        wrapMode: Text.Wrap
        horizontalAlignment: Text.AlignHCenter
    }
}
