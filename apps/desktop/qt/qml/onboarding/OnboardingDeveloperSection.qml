import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP17 -- Advanced > Onboarding (Developer), behind Show Debug
// Information (the Mac's OnboardingDeveloperSection, M62). The Mac's tour and
// hint rows are left out: this app has no tour. The reset applies at the next
// start, the only honest way to test what happens at a start.
SettingsGroup {
    id: section
    objectName: "Advanced.onboarding"
    visible: SettingsController.showDebugInfo
    title: qsTr("Onboarding (Developer)")
    iconName: "graduation-cap"
    property string note

    RowLayout {
        Layout.fillWidth: true
        spacing: Theme.spacing
        ColumnLayout {
            Layout.fillWidth: true
            spacing: 2
            Text {
                text: qsTr("Start as a new user")
                font: Theme.fontBody
                color: Theme.text
            }
            Text {
                Layout.fillWidth: true
                text: qsTr("Forgets everything. The next launch runs setup from the top.")
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
        }
        AppButton {
            objectName: "Advanced.resetOnboarding"
            Layout.preferredWidth: 150
            text: qsTr("Reset Onboarding")
            onClicked: {
                OnboardingController.resetOnboarding()
                section.note = qsTr("Reset. Relaunch to see a first run.")
            }
        }
    }
    ColumnLayout {
        Layout.fillWidth: true
        spacing: 4
        Text {
            objectName: "Advanced.onboardingState"
            Layout.fillWidth: true
            text: OnboardingController.setupDone ? qsTr("Setup: done") : qsTr("Setup: not done yet")
            font: Theme.fontSmall
            color: Theme.textMuted
        }
        Text {
            Layout.fillWidth: true
            text: OnboardingController.whatsNewShown.length
                  ? qsTr("Release notes read up to %1").arg(OnboardingController.whatsNewShown)
                  : qsTr("Release notes: none read yet")
            font: Theme.fontSmall
            color: Theme.textMuted
        }
        Text {
            visible: section.note.length > 0
            text: section.note
            font: Theme.fontSmall
            color: Theme.accentText
        }
    }
}
