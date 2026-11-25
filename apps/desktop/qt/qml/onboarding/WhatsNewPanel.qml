import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP17 -- What's New (the Mac's WhatsNewView, 460×420): each release's
// headline, version and items from the core's notes, newest first, then Done.
// Help > What's New opens it, and it opens by itself once after an update.
// Showing it counts the notes of this build's firmware as read.
// -- WP LAY1b -- A panel (Panel.qml): PanelTable's row for help.whatsNew gives
// its window's title and sizes.
// -- WP DL7 -- In a Modern page it is a page, not a dialog: no Done, the
// notes in a left-aligned column of Theme.contentMaxWidth, and an EmptyState
// when the build has none. Its window (Classic, or after an update) keeps Done.
// -- WP UFW -- (UXP A8, C8) The app's own notes first (under its name: the core
// marks them `app`), with buttons to the places they name; the firmware's
// notes follow under "DSPi firmware". It opens by itself only for the app's
// news (OnboardingController.showReleaseNotesIfUnread).
Panel {
    id: panel
    objectName: "WhatsNewPanel"
    readonly property bool asPage: inPage && LayoutController.modern   // -- WP DL7 --

    onShownChanged: if (shown) OnboardingController.markWhatsNewRead()

    ListView {
        id: list
        objectName: "WhatsNew.list"
        anchors {
            left: parent.left
            right: parent.right
            top: parent.top
            bottom: panel.asPage ? parent.bottom : bar.top   // -- WP DL7 --
        }
        clip: true
        spacing: 24
        topMargin: 20
        bottomMargin: 20
        leftMargin: panel.asPage ? Theme.spacingLarge : 20   // -- WP DL7 --
        // -- WP DL7 -- a page: the column stops at contentMaxWidth
        rightMargin: panel.asPage ? Math.max(Theme.spacingLarge, width - Theme.spacingLarge - Theme.contentMaxWidth) : 20
        boundsBehavior: Flickable.StopAtBounds
        model: OnboardingController.releases
        ScrollBar.vertical: AppScrollBar {}

        delegate: ColumnLayout {
            id: release
            required property var modelData
            required property int index
            // -- WP UFW -- the first of the app's notes and of the firmware's start their part
            readonly property bool app: modelData.app === true
            readonly property bool starts: index === 0 || app !== (OnboardingController.releases[index - 1].app === true)
            objectName: "WhatsNew.release." + index
            width: ListView.view.width - ListView.view.leftMargin - ListView.view.rightMargin
            spacing: 10
            Text {   // -- WP UFW --
                objectName: "WhatsNew.part." + (release.app ? "app" : "firmware")
                Layout.fillWidth: true
                // Only when there are both parts
                visible: release.starts && OnboardingController.releases.some(r => r.app === true)
                text: release.app ? AppIdentity.displayName : qsTr("DSPi firmware")
                color: Theme.textMuted
                font.pixelSize: Theme.fontSmall.pixelSize
                font.weight: Font.DemiBold
                font.capitalization: Font.AllUppercase
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            RowLayout {
                Layout.fillWidth: true
                spacing: Theme.spacingSmall
                Text {
                    objectName: "WhatsNew.headline"
                    Layout.fillWidth: true
                    text: release.modelData.headline || ""
                    wrapMode: Text.Wrap
                    color: Theme.text
                    font.pixelSize: Theme.fontBody.pixelSize
                    font.weight: Font.DemiBold
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
                Text {
                    Layout.alignment: Qt.AlignTop
                    text: release.modelData.version || ""
                    color: Theme.textMuted
                    font.family: Theme.fontMono.family
                    font.pixelSize: Theme.fontSmall.pixelSize
                }
            }
            Repeater {
                model: release.modelData.items || []
                delegate: Text {
                    required property string modelData
                    objectName: "WhatsNew.item"
                    Layout.fillWidth: true
                    // A hanging bullet: the wrapped lines start under the text
                    leftPadding: 16
                    text: modelData
                    wrapMode: Text.Wrap
                    color: Theme.text
                    font: Theme.fontBody
                    Text {
                        text: "•"
                        color: Theme.textMuted
                        font: Theme.fontBody
                    }
                }
            }
            // -- WP UFW -- the places the notes name, one click away
            Flow {
                Layout.fillWidth: true
                Layout.leftMargin: 16
                visible: (release.modelData.actions || []).length > 0
                spacing: Theme.spacingSmall
                Repeater {
                    model: release.modelData.actions || []
                    delegate: AppButton {
                        required property var modelData
                        objectName: "WhatsNew.action." + modelData.page
                        text: modelData.text
                        onClicked: {
                            SettingsController.showPage(modelData.page)
                            if (!panel.asPage) panel.close()
                        }
                    }
                }
            }
        }
    }

    Text {
        objectName: "WhatsNew.empty"
        visible: list.count === 0 && !panel.asPage   // -- WP DL7 -- a page: the EmptyState below
        anchors {
            left: parent.left
            top: parent.top
            margins: 20
        }
        color: Theme.textMuted
        font: Theme.fontBody
        text: qsTr("No release notes are available in this build.")
    }

    EmptyState {   // -- WP DL7 --
        objectName: "WhatsNew.emptyState"
        visible: list.count === 0 && panel.asPage
        anchors.fill: parent
        iconName: "file-text"
        title: qsTr("No release notes")
        text: qsTr("No release notes are available in this build.")
    }

    // Done, under a line
    Rectangle {
        id: bar
        visible: !panel.asPage   // -- WP DL7 -- a page has no Done
        anchors {
            left: parent.left
            right: parent.right
            bottom: parent.bottom
        }
        height: done.implicitHeight + 2 * Theme.spacingLarge
        color: Theme.window
        Rectangle {
            width: parent.width
            height: 1
            color: Theme.separator
        }
        AppButton {
            id: done
            objectName: "WhatsNew.done"
            anchors {
                right: parent.right
                verticalCenter: parent.verticalCenter
                rightMargin: Theme.spacingLarge
            }
            primary: true
            text: qsTr("Done")
            onClicked: panel.close()
        }
    }

    Shortcut {
        sequences: ["Escape", "Return", "Enter"]
        enabled: panel.shown   // -- WP LAY1b -- only while it shows
        onActivated: panel.close()
    }
}
