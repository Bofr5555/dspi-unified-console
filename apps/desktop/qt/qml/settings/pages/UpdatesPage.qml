import QtQuick 2.15
import "../"
import "../../components"

// WP QD16: checking for a newer DSPi Console (AppUpdater). Opt-in: nothing is
// fetched until automatic checks are turned on or Check Now is pressed. The
// privacy line says what a check sends. Check and link only (OD8): the release
// page opens in the browser.
SettingsPage {
    id: page
    title: qsTr("Updates")
    writesToDevice: false
    objectName: "updatesPage"

    readonly property color secondary: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
    readonly property color primary: isMacOS ? MacColors.label : "white"

    SettingsSection {
        footnote: appUpdater.privacyLine
        SettingsSwitchRow {
            objectName: "updatesAutomatic"
            title: qsTr("Check for Updates Automatically")
            detail: qsTr("Off until you turn it on.")
            enabled: appUpdater.hasFeed
            checked: appUpdater.automatic
            onToggled: function (on) { appUpdater.automatic = on }
        }
        SettingsButtonRow {
            objectName: "updatesCheckNow"
            title: qsTr("Version %1").arg(appUpdater.currentVersion)
            detail: appUpdater.checking ? qsTr("Checking…")
                    : appUpdater.lastChecked !== "" ? qsTr("Last checked %1").arg(appUpdater.lastChecked)
                    : qsTr("Not checked yet")
            buttonText: qsTr("Check Now")
            enabled: appUpdater.hasFeed && !appUpdater.checking
            onClicked: appUpdater.checkNow()
        }
    }
    SettingsStatusLine {
        objectName: "updatesStatus"
        text: appUpdater.checking ? ""
              : appUpdater.result === "current" ? qsTr("DSPi Console %1 is the newest version.").arg(appUpdater.currentVersion)
              : appUpdater.result === "failed" ? qsTr("Couldn't check for updates: %1").arg(appUpdater.failure)
              : ""
        error: appUpdater.result === "failed"
    }

    SettingsSection {
        objectName: "updatesAvailable"
        visible: appUpdater.result === "available"
        title: qsTr("Available")
        footnote: qsTr("The new version includes matching DSPi firmware, which it offers to install when your device is connected.")
        SettingsButtonRow {
            objectName: "updatesOpenPage"
            title: appUpdater.availableTitle
            verbatim: true
            detail: qsTr("Version %1").arg(appUpdater.availableVersion)
            buttonText: qsTr("View Release")
            onClicked: appUpdater.openPage()
        }
        SettingsRow {
            objectName: "updatesNotes"
            visible: appUpdater.notes !== ""
            title: qsTr("Release Notes")
            detail: appUpdater.notes
            verbatim: true
        }
        SettingsButtonRow {
            objectName: "updatesSkip"
            title: qsTr("Skip This Version")
            detail: qsTr("Automatic checks won't mention %1 again.").arg(appUpdater.availableVersion)
            buttonText: qsTr("Skip")
            onClicked: appUpdater.skipVersion()
        }
    }
}
