import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP UB7 -- (UXP B7, C3) The speaker layout in one line where it is used,
// not edited: Measure and Auto-tune ("L: woofer Out 1, tweeter Out 2 · R: …
// · confirmed"), with Edit in Speakers… to the one table (Settings › Speakers).
// What each run measures stays the page's own tick boxes.
RowLayout {
    id: summary
    objectName: "SpeakersSummary"
    readonly property var sc: SpeakersController
    spacing: Theme.spacingSmall
    visible: sc.summary.length > 0

    Text {
        objectName: "SpeakersSummary.text"
        Layout.fillWidth: true
        textFormat: Text.PlainText
        text: summary.sc.summary
        font: Theme.fontBody
        color: Theme.text
        wrapMode: Text.WordWrap
        Accessible.role: Accessible.StaticText
        //: %1 the speaker layout in one line ("L: woofer Out 1, tweeter Out 2 · confirmed")
        Accessible.name: qsTr("Speaker layout: %1").arg(text)
    }
    AppButton {
        objectName: "SpeakersSummary.edit"
        Layout.alignment: Qt.AlignTop
        text: qsTr("Edit in Speakers…")
        onClicked: summary.sc.openSettings()
    }
}
