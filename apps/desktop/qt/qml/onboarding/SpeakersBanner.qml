import QtQuick
import UConsole

// -- WP ROLEQ -- On the Measure tabs while the speaker layout is not
// confirmed: imported with a configuration, kept unconfirmed, or none yet
// (only the suggestion). Each points to Settings › Speakers.
Banner {
    id: banner
    objectName: "SpeakersBanner"
    readonly property var sc: SpeakersController
    readonly property string status: sc.status
    readonly property string words: status, sc.loaded, sc.bannerText(false)
    kind: status === "imported" ? "warning" : "info"
    text: words
    visible: words.length > 0
    AppButton {
        objectName: "SpeakersBanner.open"
        text: qsTr("Settings › Speakers…")
        onClicked: banner.sc.openSettings()
    }
}
