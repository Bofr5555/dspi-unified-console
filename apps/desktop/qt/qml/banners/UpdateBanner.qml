import QtQuick
import UConsole

// -- WP QP17 -- A newer release was found by an automatic check (behaviour.md
// §2 item 5, §17): Details opens Settings > Updates, Later hides it until a
// check finds one again. Nothing is downloaded.
Banner {
    objectName: "UpdateBanner"
    kind: "info"
    iconName: "download"
    //: %1 is the app's name, %2 the new version, %3 the running one
    text: qsTr("%1 %2 is available. You have %3.").arg(AppIdentity.displayName)
          .arg(AppUpdater.releaseVersion).arg(AppUpdater.currentVersion)
    visible: AppUpdater.announce

    AppButton {
        objectName: "UpdateBanner.details"
        text: qsTr("Details")
        primary: true
        onClicked: AppUpdater.details()
    }
    AppButton {
        objectName: "UpdateBanner.later"
        text: qsTr("Later")
        onClicked: AppUpdater.later()
    }
}
