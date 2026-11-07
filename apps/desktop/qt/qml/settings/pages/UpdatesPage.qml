import QtQuick 2.15
import "../"
import "../../components"

// WP QD16: checking for a newer DSPi Console (AppUpdater). Opt-in: nothing is
// fetched until automatic checks are turned on or Check Now is pressed. The
// privacy line says what a check sends. Check and link only (OD8): the release
// page opens in the browser.
SettingsPage {
    id: page
    title: "Updates"
    writesToDevice: false
    objectName: "updatesPage"

    readonly property color secondary: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
    readonly property color primary: isMacOS ? MacColors.label : "white"

    SettingsSection {
        footnote: appUpdater.privacyLine
        SettingsSwitchRow {
            objectName: "updatesAutomatic"
            title: "Check for Updates Automatically"
            detail: "Off until you turn it on."
            enabled: appUpdater.hasFeed
            checked: appUpdater.automatic
            onToggled: function (on) { appUpdater.automatic = on }
        }
        SettingsButtonRow {
            objectName: "updatesCheckNow"
            title: "Version " + appUpdater.currentVersion
            detail: appUpdater.checking ? "Checking…"
                    : appUpdater.lastChecked !== "" ? "Last checked " + appUpdater.lastChecked
                    : "Not checked yet"
            buttonText: "Check Now"
            enabled: appUpdater.hasFeed && !appUpdater.checking
            onClicked: appUpdater.checkNow()
        }
    }
    SettingsStatusLine {
        objectName: "updatesStatus"
        text: appUpdater.checking ? ""
              : appUpdater.result === "current" ? "DSPi Console " + appUpdater.currentVersion + " is the newest version."
              : appUpdater.result === "failed" ? "Couldn't check for updates: " + appUpdater.failure
              : ""
        error: appUpdater.result === "failed"
    }

    SettingsSection {
        objectName: "updatesAvailable"
        visible: appUpdater.result === "available"
        title: "Available"
        footnote: "The new version includes matching DSPi firmware, which it offers to install when your device is connected."
        SettingsButtonRow {
            objectName: "updatesOpenPage"
            title: appUpdater.availableTitle
            verbatim: true
            detail: "Version " + appUpdater.availableVersion
            buttonText: "View Release"
            onClicked: appUpdater.openPage()
        }
        SettingsRow {
            objectName: "updatesNotes"
            visible: appUpdater.notes !== ""
            title: "Release Notes"
            detail: appUpdater.notes
            verbatim: true
        }
        SettingsButtonRow {
            objectName: "updatesSkip"
            title: "Skip This Version"
            detail: "Automatic checks won't mention " + appUpdater.availableVersion + " again."
            buttonText: "Skip"
            onClicked: appUpdater.skipVersion()
        }
    }
}
