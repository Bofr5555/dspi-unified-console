import QtQuick
import QtQuick.Layouts
import UConsole

// Settings > Updates (behaviour.md §17; the Mac's SoftwareUpdateSettingsTab
// for the wording). This page is only the view: the checker is QP17's
// AppUpdater, handed over with SettingsController::setUpdater(). What it reads
// of the checker (each optional, a missing one reads as unset):
//   feedAvailable (bool)   false: a build without a feed, which never checks
//   automatic (bool, written by the switch), checking (bool)
//   lastChecked (date, invalid before the first check), currentVersion (string)
//   available (bool), releaseName, releaseVersion, releaseNotes (plain text),
//   releaseUrl, firmwareIncluded (bool), error (string)
//   feedHost, userAgent (strings, for the privacy line)
//   checkNow(), openRelease(), skipVersion()
// Without a checker the page shows the no-feed state.
SettingsPage {
    id: page
    objectName: "UpdatesPage"
    readonly property var updater: SettingsController.updater
    readonly property bool hasFeed: updater !== null && updater.feedAvailable === true
    readonly property bool checking: hasFeed && updater.checking === true
    readonly property bool available: hasFeed && updater.available === true
    readonly property string version: updater && updater.currentVersion ? updater.currentVersion : AppIdentity.version
    readonly property bool checked: hasFeed && updater.lastChecked instanceof Date && !isNaN(updater.lastChecked.getTime())

    readonly property string statusTitle: {
        if (!hasFeed) return qsTr("Version %1").arg(version)
        if (checking) return qsTr("Checking for updates...")
        if (available) return qsTr("%1 %2 is available").arg(AppIdentity.displayName).arg(updater.releaseVersion || "")
        return checked ? qsTr("%1 is up to date").arg(AppIdentity.displayName) : qsTr("Not checked yet")
    }
    readonly property string statusDetail: {
        if (!hasFeed) return qsTr("This build has no release feed, so it never checks.")
        if (updater.error) return updater.error
        const v = qsTr("Version %1").arg(version)
        if (!checked || checking) return v
        return qsTr("%1 - last checked %2").arg(v).arg(updater.lastChecked.toLocaleString(Qt.locale(), Locale.ShortFormat))
    }

    SettingsGroup {
        title: qsTr("Status")
        iconName: "download"
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacing
            Icon {
                Layout.alignment: Qt.AlignTop
                name: page.available ? "download" : page.hasFeed ? "circle-check" : "info"
                size: 28
                color: page.available ? Theme.accentText : page.hasFeed ? Theme.success : Theme.textMuted
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 4
                Text {
                    objectName: "Updates.status"
                    Layout.fillWidth: true
                    text: page.statusTitle
                    font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
                    color: Theme.text
                    wrapMode: Text.Wrap
                }
                Text {
                    objectName: "Updates.detail"
                    Layout.fillWidth: true
                    text: page.statusDetail
                    font: Theme.fontSmall
                    color: page.hasFeed && page.updater.error ? Theme.warning : Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
        }
        AppButton {
            objectName: "Updates.checkNow"
            text: qsTr("Check Now")
            enabled: page.hasFeed && !page.checking
            onClicked: page.updater.checkNow()
            Layout.alignment: Qt.AlignRight
        }
    }

    SettingsGroup {
        title: qsTr("Automatic Checks")
        iconName: "clock"
        SettingsToggle {
            objectName: "Updates.automatic"
            title: qsTr("Check for Updates Automatically")
            enabled: page.hasFeed
            checked: page.hasFeed && page.updater.automatic === true
            onToggled: on => page.updater.automatic = on
        }
        Text {
            objectName: "Updates.privacy"
            Layout.fillWidth: true
            text: qsTr("A check asks %1 for the newest release. The request carries only what every web request carries, your IP address, and this app's version in its user agent (%2); nothing about your devices, presets or settings. With automatic checks on, the app checks at launch and once a day.")
                  .arg(page.updater && page.updater.feedHost ? page.updater.feedHost : qsTr("the release feed"))
                  .arg(page.updater && page.updater.userAgent ? page.updater.userAgent : "DSPiConsole/" + AppIdentity.version)
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    SettingsGroup {
        objectName: "Updates.availableCard"
        visible: page.available
        title: qsTr("Available")
        iconName: "download"
        footer: page.available && page.updater.firmwareIncluded === true
                ? qsTr("This release includes matching DSPi firmware, which the app offers to install when your device is connected.") : ""
        Text {
            Layout.fillWidth: true
            text: page.available ? (page.updater.releaseName || "") + "  " + (page.updater.releaseVersion || "") : ""
            font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
            color: Theme.text
            wrapMode: Text.Wrap
        }
        // The notes as plain text: no markup is interpreted
        TextEdit {
            objectName: "Updates.notes"
            Layout.fillWidth: true
            visible: text.length > 0
            text: page.available ? (page.updater.releaseNotes || "") : ""
            textFormat: TextEdit.PlainText
            readOnly: true
            selectByMouse: true
            wrapMode: TextEdit.Wrap
            font: Theme.fontSmall
            color: Theme.text
            selectionColor: Theme.selection
            selectedTextColor: Theme.text
            Accessible.role: Accessible.StaticText
            Accessible.name: qsTr("Release notes")
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "Updates.skip"
                text: qsTr("Skip This Version")
                onClicked: skipQuestion.open()
            }
            Item {
                implicitHeight: 1
                Layout.fillWidth: true
            }
            AppButton {
                objectName: "Updates.viewRelease"
                text: qsTr("View Release")
                primary: true
                iconName: "external-link"
                onClicked: page.updater.openRelease()
            }
        }
    }

    QuestionDialog {
        id: skipQuestion
        objectName: "Updates.skipQuestion"
        parent: page.Window.window ? page.Window.window.contentItem : page
        title: qsTr("Skip This Version")
        text: qsTr("Automatic checks will stay quiet about %1 %2. Check Now still shows it.")
              .arg(AppIdentity.displayName).arg(page.available ? page.updater.releaseVersion || "" : "")
        acceptText: qsTr("Skip")
        onAnswered: answer => { if (answer === "accept" && page.updater) page.updater.skipVersion() }
    }
}
