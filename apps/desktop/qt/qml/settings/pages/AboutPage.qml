import QtQuick 2.15
import "../"
import "../../components"

SettingsPage {
    writesToDevice: false   // -- WP QD6 --
    title: qsTr("About")

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
            Text { text: qsTr("DSPi uConsole"); font.pixelSize: 22; font.weight: Font.Bold; color: isMacOS ? MacColors.label : "white" }
            Text { text: qsTr("Version %1 for %2").arg(Qt.application.version).arg(isMacOS ? "macOS" : isWindows ? "Windows" : "Linux"); font.pixelSize: 13; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.6) }
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
            { title: qsTr("Made by"), note: "", rows: aboutInfo.makers },
            { title: aboutInfo.linksTitle, note: aboutInfo.linksNote, rows: aboutInfo.links },
            { title: qsTr("Credits"), note: "", rows: aboutInfo.credits }
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

    // -- WP LIC1 -- The project's licence and its section 7 terms, as the table words
    // them (assets/about.json `licence`). The terms require them here, with the credit
    // lines above and the licence texts below.
    SettingsSection {
        objectName: "aboutLicence"
        visible: aboutInfo.licenceStatement !== ""
        title: aboutInfo.licenceTitle
        note: {
            var lines = []
            for (var i = 0; i < aboutInfo.licenceCopyright.length; i++) lines.push(aboutInfo.licenceCopyright[i])
            lines.push(aboutInfo.licenceStatement)
            return lines.join("\n\n")
        }
        Repeater {
            model: [
                { title: qsTr("GNU General Public License, version 3"), detail: aboutInfo.licenceSpdx, file: "GPL-3.0.txt" },
                { title: qsTr("Qt: GNU Lesser General Public License, version 3"), detail: "LGPL-3.0-only", file: "LGPL-3.0.txt" },
                { title: qsTr("Rust crates: licences and notices"), detail: qsTr("The crates the core links"), file: "rust-crates.txt" }
            ]
            SettingsRow {
                objectName: "licenceTextRow"
                title: modelData.title
                detail: modelData.detail
                verbatim: true
                clickable: true
                onActivated: { licenceText.heading = modelData.title; licenceText.file = modelData.file; licenceText.open() }
            }
        }
        footnote: aboutInfo.licenceNotices
    }
    SettingsSection {
        objectName: "aboutTerms"
        visible: aboutInfo.terms.length > 0
        title: aboutInfo.termsTitle
        note: aboutInfo.termsNote
        Repeater {
            model: aboutInfo.terms
            SettingsRow {
                objectName: "licenceTermRow"
                title: (index + 1) + ". " + modelData.title
                detail: modelData.detail
                verbatim: true
            }
        }
        footnote: aboutInfo.licenceFreedoms
    }
    LicenceTextPopup { id: licenceText }
}
