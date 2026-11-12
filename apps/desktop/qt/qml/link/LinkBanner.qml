import QtQuick
import UConsole

// The hub's connection is lost and the core is reconnecting: shown at once,
// until the session is bound again or left. The text and the button are the
// core's (the reconnecting phase's wording); the button leaves the hub, also
// in the middle of a dial. Meanwhile the app shows the last values read and a
// change fails at once.
Banner {
    objectName: "LinkBanner"
    kind: "warning"
    iconName: "wifi"
    text: LinkController.hubName.length
          ? LinkController.hubName + ": " + LinkController.phaseText
          : LinkController.phaseText
    visible: LinkController.phase === "reconnecting"

    AppButton {
        objectName: "LinkBanner.disconnect"
        text: LinkController.actionText.length ? LinkController.actionText : qsTr("Disconnect")
        onClicked: LinkController.closeSession()
    }
}
