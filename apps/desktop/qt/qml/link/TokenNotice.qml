import QtQuick
import UConsole

// A hub's pairing could only be saved in a file, because no secret store
// answered (Credential Manager, the Secret Service). Shown once, until OK.
Banner {
    objectName: "TokenNotice"
    kind: "info"
    iconName: "key-round"
    text: LinkController.tokenWarning
    visible: LinkController.tokenWarning.length > 0

    AppButton {
        objectName: "TokenNotice.ok"
        text: qsTr("OK")
        onClicked: LinkController.dismissTokenWarning()
    }
}
