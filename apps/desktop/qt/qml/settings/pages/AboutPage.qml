import QtQuick 2.15
import "../"
import "../../components"

SettingsPage {
    writesToDevice: false   // -- WP QD6 --
    title: "About"

    // Who made what, the links and the credits come from assets/about.json through
    // aboutInfo (AboutController), as written, so any wording fits: rows and the card
    // grow with it, and a list it leaves empty is not shown.

    // App identity
    Rectangle {
        objectName: "aboutIdentity"
        width: parent.width
        height: Math.max(116, identity.height + 32)
        radius: isMacOS ? 5 : 12
        color: isMacOS ? "#2b2b2b" : Qt.rgba(1, 1, 1, 0.045)
        border.color: isMacOS ? Qt.rgba(1, 1, 1, 0.11) : Qt.rgba(1, 1, 1, 0.07)

        Rectangle {
            id: appTile
            x: 20
            anchors.verticalCenter: parent.verticalCenter
            width: 72; height: 72; radius: 18
            gradient: Gradient {
                GradientStop { position: 0; color: "#3a96ff" }
                GradientStop { position: 1; color: "#6b4cf0" }
            }
            Icon { anchors.centerIn: parent; name: "sliders"; size: 38; color: "white" }
        }
        Column {
            id: identity
            anchors.left: appTile.right
            anchors.leftMargin: 18
            anchors.right: parent.right
            anchors.rightMargin: 20
            anchors.verticalCenter: parent.verticalCenter
            spacing: 4
            Text { text: "DSPi Console"; font.pixelSize: 22; font.weight: Font.Bold; color: isMacOS ? MacColors.label : "white" }
            Text { text: "Version " + Qt.application.version + " for " + (isMacOS ? "macOS" : isWindows ? "Windows" : "Linux"); font.pixelSize: 13; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.6) }
            Repeater {
                model: aboutInfo.creditLines
                Text {
                    width: parent.width
                    wrapMode: Text.Wrap
                    textFormat: Text.PlainText
                    text: modelData
                    font.pixelSize: 13
                    color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45)
                }
            }
        }
    }

    // The links section's heading and note are the table's (WP CG29): they say whose
    // links these are. The note sits above the rows, as it must be read before them.
    Repeater {
        model: [
            { title: "Made by", note: "", rows: aboutInfo.makers },
            { title: aboutInfo.linksTitle, note: aboutInfo.linksNote, rows: aboutInfo.links },
            { title: "Credits", note: "", rows: aboutInfo.credits }
        ]
        SettingsSection {
            title: modelData.title
            note: modelData.note
            visible: modelData.rows.length > 0
            Repeater {
                model: modelData.rows
                SettingsRow {
                    readonly property bool hasLink: modelData.url !== ""
                    title: modelData.title
                    titleColor: isMacOS ? (hasLink ? Qt.rgba(0.72, 0.72, 0.72, 1) : MacColors.label) : "white"
                    detail: modelData.detail
                    verbatim: true
                    clickable: hasLink
                    onActivated: Qt.openUrlExternally(modelData.url)
                    // A Repeater, as an Icon that is only hidden would still reserve its width
                    Repeater {
                        model: hasLink ? 1 : 0
                        Icon { name: "external"; size: 16; color: Qt.rgba(1, 1, 1, 0.5) }
                    }
                }
            }
        }
    }

    // Not in about.json: the wording of this paragraph is pending with the owner
    Text {
        width: parent.width
        wrapMode: Text.WordWrap
        font.pixelSize: 12
        color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
        text: "DSPi Firmware and Console are free, open-source software developed in spare time. "
            + "Contributions of any kind - code, feedback, funding, or otherwise - are always immensely appreciated."
    }
}
