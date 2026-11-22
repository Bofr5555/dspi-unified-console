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
// -- WP SECFIX -- After a dspi://pair link (LinkController.openPairLink) the
// link's PIN is filled in: the hub's name and key are on screen, and Pair is
// the user's yes. A hub on this machine (LinkController.local) is not
// unsecured: no red line.
Dialog {
    id: dialog
    objectName: "PairingDialog"
    title: qsTr("Pair with %1").arg(LinkController.hubName.length ? LinkController.hubName : qsTr("the hub"))

    // -- WP LAY5b -- Modern's Devices > Hubs asks on its page
    readonly property bool asked: LinkController.phase === "needs_pairing" && !(Shell.pageHost && Shell.pageHost.pairsInline === true)
    readonly property bool complete: content.complete

    function submit() { content.submit() }

    onAskedChanged: asked ? open() : close()
    Component.onCompleted: if (asked) open()
    onAboutToShow: content.start()
    onOpened: content.takeFocus()
    onRejected: if (asked) LinkController.closeSession()

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
            textFormat: Text.PlainText
            text: dialog.title
            font: Theme.fontTitle
            color: Theme.text
            wrapMode: Text.Wrap
            Accessible.role: Accessible.Heading
            Accessible.name: text
        }
    }

    contentItem: PairingContent {
        id: content
        Accessible.role: Accessible.Dialog
        Accessible.name: dialog.title
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
