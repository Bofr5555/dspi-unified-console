import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP ROLEQ -- Settings › Speakers (MPLAN 2.2): which output drives what,
// the person's speaker layout for the open DSPi. A SettingsPage, so Classic
// and Modern show the same page. Confirm is the only way a layout becomes
// confirmed; the Measure tables write their edits back without confirming.
SettingsPage {
    id: page
    objectName: "SpeakersPage"
    readonly property var sc: SpeakersController
    readonly property bool confirmed: sc.status === "confirmed" && !sc.edited

    Component.onCompleted: sc.reload()

    SettingsGroup {
        objectName: "Speakers.layout"
        title: qsTr("Speaker layout")
        iconName: "volume-2"
        footer: qsTr("An output with an active high-pass at 1 kHz or above is measured as a tweeter, whatever its role here.")

        Text {
            Layout.fillWidth: true
            text: qsTr("Which output drives what? The measurement and its safety checks take each output's role from here.")
            font: Theme.fontBody
            color: Theme.text
            wrapMode: Text.Wrap
        }
        Banner {
            objectName: "SpeakersPage.banner"
            Layout.fillWidth: true
            readonly property string words: page.sc.status, page.sc.loaded, page.sc.bannerText(true)
            kind: page.sc.status === "imported" ? "warning" : "info"
            text: words
            visible: words.length > 0
        }
        Text {
            objectName: "SpeakersPage.readOnly"
            visible: DeviceModel.connected && !page.sc.editable
            Layout.fillWidth: true
            text: qsTr("This connection may not change the DSPi: the speaker layout is shown, not changed.")
            font: Theme.fontBody
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        SpeakersTable {
            Layout.fillWidth: true
        }
        RowLayout {
            Layout.fillWidth: true
            spacing: Theme.spacingSmall
            AppButton {
                objectName: "SpeakersPage.confirm"
                text: qsTr("Confirm")
                primary: true
                enabled: page.sc.editable && page.sc.loaded && !page.confirmed
                onClicked: page.sc.confirm()
            }
            AppButton {
                objectName: "SpeakersPage.suggestion"
                text: qsTr("Use the Suggestion")
                enabled: page.sc.editable && page.sc.loaded
                onClicked: page.sc.useSuggestion()
            }
            Text {
                objectName: "SpeakersPage.state"
                Layout.fillWidth: true
                text: page.confirmed ? qsTr("Confirmed.")
                      : page.sc.edited && page.sc.status !== "none" ? qsTr("Changed: not kept until you confirm.") : ""
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
        }
        Text {
            objectName: "SpeakersPage.said"
            visible: text.length > 0
            Layout.fillWidth: true
            text: page.sc.said
            font: Theme.fontSmall
            color: page.sc.saidError ? Theme.dangerText : Theme.textMuted
            wrapMode: Text.Wrap
        }
    }
}
