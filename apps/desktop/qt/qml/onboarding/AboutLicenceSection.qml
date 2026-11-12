import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP17 -- About's licence block (the Mac's AboutLicenceSections): the
// copyright lines, the statement, the GPL's text, the note on the published
// source, then the additional terms. Every word is the core's About table
// (OnboardingController.licence); nothing is written here.
ColumnLayout {
    id: section
    objectName: "About.licence"
    readonly property var licence: OnboardingController.licence
    // The staged GPL text is one of the notices below; without it, the web page
    readonly property bool gplBundled: AboutController.notices.some(n => String(n.path || "").endsWith("GPL-3.0.txt"))

    Layout.fillWidth: true
    spacing: Theme.spacingLarge
    visible: licence.title !== undefined

    SettingsGroup {
        title: section.licence.title || ""
        iconName: "file-text"
        Repeater {
            model: section.licence.copyright || []
            delegate: Text {
                required property string modelData
                objectName: "About.licenceCopyright"
                Layout.fillWidth: true
                text: modelData
                font: Theme.fontSmall
                color: Theme.textMuted
                wrapMode: Text.Wrap
            }
        }
        TextEdit {
            objectName: "About.licenceStatement"
            Layout.fillWidth: true
            text: section.licence.statement || ""
            textFormat: TextEdit.PlainText
            readOnly: true
            selectByMouse: true
            wrapMode: TextEdit.Wrap
            font: Theme.fontBody
            color: Theme.textMuted
            selectionColor: Theme.selection
            selectedTextColor: Theme.text
            Accessible.role: Accessible.StaticText
            Accessible.name: text
        }
        AppButton {
            objectName: "About.licenceGpl"
            visible: !section.gplBundled
            text: qsTr("GNU General Public License, version 3")
            iconName: "external-link"
            onClicked: Shell.openUrl("https://www.gnu.org/licenses/gpl-3.0.html")
        }
        Text {
            objectName: "About.licenceNotices"
            Layout.fillWidth: true
            text: section.licence.notices || ""
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }

    SettingsGroup {
        title: section.licence.terms_title || ""
        iconName: "badge-check"
        Text {
            Layout.fillWidth: true
            text: section.licence.terms_note || ""
            font: Theme.fontBody
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        Repeater {
            model: section.licence.terms || []
            delegate: TextEdit {
                required property var modelData
                required property int index
                objectName: "About.licenceTerm." + modelData.id
                Layout.fillWidth: true
                text: (index + 1) + ". " + modelData.title + ". " + modelData.text
                textFormat: TextEdit.PlainText
                readOnly: true
                selectByMouse: true
                wrapMode: TextEdit.Wrap
                font: Theme.fontBody
                color: Theme.text
                selectionColor: Theme.selection
                selectedTextColor: Theme.text
                Accessible.role: Accessible.StaticText
                Accessible.name: text
            }
        }
        Text {
            Layout.fillWidth: true
            text: section.licence.freedoms || ""
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
    }
}
