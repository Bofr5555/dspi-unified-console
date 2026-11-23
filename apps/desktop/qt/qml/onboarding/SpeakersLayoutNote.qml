import QtQuick
import UConsole

// -- WP ADETQ -- On the main screen (Classic's banners, Modern's status strip):
// a speaker layout kept but not confirmed (detected by the speaker detection,
// imported, or kept unconfirmed) in one line, pointing to Settings › Speakers,
// as the web's LayoutNote (ADETW). Only the suggestion, with nothing kept, is
// not shown here.
Banner {
    id: note
    objectName: "SpeakersLayoutNote"
    readonly property var sc: SpeakersController
    readonly property string status: sc.status
    readonly property bool kept: ["autoDetected", "unconfirmed", "imported"].indexOf(status) >= 0
    readonly property string words: status, sc.loaded, sc.stampText, kept ? sc.bannerText(false) : ""
    kind: status === "imported" ? "warning" : "info"
    iconName: "volume-2"
    text: words
    visible: words.length > 0
    AppButton {
        objectName: "SpeakersLayoutNote.open"
        text: qsTr("Settings › Speakers…")
        onClicked: note.sc.openSettings()
    }
}
