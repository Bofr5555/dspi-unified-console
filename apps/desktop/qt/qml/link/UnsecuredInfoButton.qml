import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP SECQT -- The small info button beside "(unsecured)" on a connection
// line: it opens the core's calm explanation (the notice row
// dlink.unsecured.info): the connection is not encrypted, which is fine on a
// trusted local network or over a VPN, and how to turn encryption on.
IconButton {
    id: button
    objectName: "UnsecuredInfoButton"
    iconName: "info"
    iconSize: 12
    iconColor: Theme.dangerText
    toolTip: LinkController.unsecuredInfoTitle
    onClicked: info.open()

    // Shown over the window, wherever the button sits
    property alias dialog: info
    QuestionDialog {
        id: info
        objectName: "UnsecuredInfo"
        parent: Overlay.overlay
        title: LinkController.unsecuredInfoTitle
        text: LinkController.unsecuredInfoText
        acceptText: qsTr("OK")
        rejectText: ""
    }
}
