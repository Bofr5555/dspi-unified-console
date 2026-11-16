import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP SECQT -- A failure only pairing again can answer (PLAN-SEC 4.3): the
// hub's key changed, could not be verified, or the hub refused or required
// encryption. It offers pairing again and nothing else: there is no "connect
// anyway", and no token went to the hub. Pair Again dials with no stored
// credential, so the hub asks for its PIN and the prompt shows its key.
QuestionDialog {
    id: dialog
    objectName: "RePairDialog"
    readonly property string code: LinkController.errorCode
    readonly property bool asked: LinkController.rePairOffered

    title: code === "hub_key_changed" ? qsTr("This hub's key changed")
         : code === "hub_key_unverified" ? qsTr("The hub's key does not match")
         : code === "tls_refused" ? qsTr("The hub refused an encrypted connection")
         : qsTr("The hub requires encryption")
    text: [LinkController.errorText,
           code === "hub_key_changed" || code === "hub_key_unverified"
               ? qsTr("Nothing was sent to it. Pair again only if you know why the key changed, for example because the hub was set up again. Otherwise another machine may be posing as this hub.")
               : qsTr("Nothing was sent to it. Pairing again starts over with the hub's PIN; check the key the hub shows.")]
          .filter(line => line.length > 0).join("\n\n")
    acceptText: qsTr("Pair Again")
    rejectText: qsTr("Cancel")

    onAskedChanged: if (asked) open()
    Component.onCompleted: if (asked) open()
    onAnswered: answer => {
        if (answer === "accept") LinkController.rePair()
        else LinkController.dismissRePair()
    }
}
