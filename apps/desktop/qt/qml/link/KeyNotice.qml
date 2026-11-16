import QtQuick
import UConsole

// -- WP SECQT -- The hub's key was set up for this pairing (the first
// encrypted connection of a pairing made before TLS, or a rotation the old key
// signed): its fingerprint, once, until OK. Compare it with the one the hub
// shows (its Networking settings, or `uconsole hub tls fingerprint`).
Banner {
    objectName: "KeyNotice"
    kind: "info"
    iconName: "lock"
    text: LinkController.keyNotice
    visible: LinkController.keyNotice.length > 0

    AppButton {
        objectName: "KeyNotice.ok"
        text: qsTr("OK")
        onClicked: LinkController.dismissKeyNotice()
    }
}
