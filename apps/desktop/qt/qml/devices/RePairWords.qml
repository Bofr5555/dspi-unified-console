import QtQuick
import UConsole

// -- WP LAY5b -- The words of a failure only pairing again can answer (split from
// RePairDialog, which says what it offers and why). The dialog and Modern's
// Devices > Hubs show them.
QtObject {
    readonly property string code: LinkController.errorCode
    readonly property bool forgets: code === "tls_required_pinned"   // -- WP PINTLS --

    readonly property string title: forgets ? qsTr("This hub was paired with encryption")
         : code === "hub_key_changed" ? qsTr("This hub's key changed")
         : code === "hub_key_unverified" ? qsTr("The hub's key does not match")
         : code === "tls_refused" ? qsTr("The hub refused an encrypted connection")
         : qsTr("The hub requires encryption")
    readonly property string text: [LinkController.errorText,
           forgets
               ? qsTr("Nothing was sent to it, not even the PIN. Turn TLS on in the hub's settings if you can. Forget Hub deletes its pairing and key; it can then be paired without encryption, and on this network the PIN and settings could be read or changed.")
           : code === "hub_key_changed" || code === "hub_key_unverified"
               ? qsTr("Nothing was sent to it. Pair again only if you know why the key changed, for example because the hub was set up again. Otherwise another machine may be posing as this hub. Pairing again uses an encrypted connection only.")
               : qsTr("Nothing was sent to it. Pairing again starts over with the hub's PIN over an encrypted connection only; check the key the hub shows.")]
          .filter(line => line.length > 0).join("\n\n")
    readonly property string acceptText: forgets ? qsTr("Forget Hub") : qsTr("Pair Again")
}
