import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP SECQT -- A failure only pairing again can answer (PLAN-SEC 4.3): the
// hub's key changed, could not be verified, or the hub refused or required
// encryption. It offers pairing again and nothing else: there is no "connect
// anyway", and no token went to the hub. Pair Again dials with no stored
// token, so the hub asks for its PIN and the prompt shows its key.
// -- WP PINTLS -- (owner decision 2026-11-17) Pair Again keeps a hub that was
// pinned TLS-only. When such a hub refuses TLS (tls_required_pinned) the one
// offer is Forget Hub, the only way back to an unencrypted pairing, and the
// words say so.
QuestionDialog {
    id: dialog
    objectName: "RePairDialog"
    readonly property string code: LinkController.errorCode
    readonly property bool asked: LinkController.rePairOffered
    readonly property bool forgets: code === "tls_required_pinned"   // -- WP PINTLS --

    title: forgets ? qsTr("This hub was paired with encryption")
         : code === "hub_key_changed" ? qsTr("This hub's key changed")
         : code === "hub_key_unverified" ? qsTr("The hub's key does not match")
         : code === "tls_refused" ? qsTr("The hub refused an encrypted connection")
         : qsTr("The hub requires encryption")
    text: [LinkController.errorText,
           forgets
               ? qsTr("Nothing was sent to it, not even the PIN. Turn TLS on in the hub's settings if you can. Forget Hub deletes its pairing and key; it can then be paired without encryption, and on this network the PIN and settings could be read or changed.")
           : code === "hub_key_changed" || code === "hub_key_unverified"
               ? qsTr("Nothing was sent to it. Pair again only if you know why the key changed, for example because the hub was set up again. Otherwise another machine may be posing as this hub. Pairing again uses an encrypted connection only.")
               : qsTr("Nothing was sent to it. Pairing again starts over with the hub's PIN over an encrypted connection only; check the key the hub shows.")]
          .filter(line => line.length > 0).join("\n\n")
    acceptText: forgets ? qsTr("Forget Hub") : qsTr("Pair Again")
    rejectText: qsTr("Cancel")

    onAskedChanged: if (asked) open()
    Component.onCompleted: if (asked) open()
    onAnswered: answer => {
        if (answer === "accept") LinkController.rePair()
        else LinkController.dismissRePair()
    }
}
