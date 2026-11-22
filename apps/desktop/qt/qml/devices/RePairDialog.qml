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
    // -- WP LAY5b -- Modern's Devices > Hubs asks on its page
    readonly property bool asked: LinkController.rePairOffered && !(Shell.pageHost && Shell.pageHost.pairsInline === true)
    readonly property string code: words.code
    readonly property bool forgets: words.forgets   // -- WP PINTLS --
    readonly property RePairWords words: RePairWords {}

    title: words.title
    text: words.text
    acceptText: words.acceptText
    rejectText: qsTr("Cancel")

    onAskedChanged: if (asked) open()
    Component.onCompleted: if (asked) open()
    onAnswered: answer => {
        if (answer === "accept") LinkController.rePair()
        else LinkController.dismissRePair()
    }
}
