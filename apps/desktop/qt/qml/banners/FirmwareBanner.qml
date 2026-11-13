import QtQuick
import UConsole

// -- WP QP16 -- The connected device runs other firmware than this app
// carries (the Mac's FirmwareMismatchBanner). Hidden while the versions agree,
// while too little is known, on read-only firmware (ReadOnlyBanner says it
// there), and after Hide until the app is next started.
Banner {
    objectName: "FirmwareBanner"
    kind: "warning"
    text: FirmwareController.mismatchText
    visible: FirmwareController.mismatchShown

    AppButton {
        objectName: "FirmwareBanner.update"
        text: FirmwareController.match === "newer" ? qsTr("Details…") : qsTr("Update…")
        onClicked: FirmwareController.openWindow()
    }
    AppButton {
        objectName: "FirmwareBanner.hide"
        text: qsTr("Hide")
        toolTip: qsTr("Hide this warning until %1 is next started").arg(AppIdentity.displayName)
        onClicked: FirmwareController.hideMismatch()
    }
}
