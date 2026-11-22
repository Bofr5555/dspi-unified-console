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
Panel {
    id: panel
    objectName: "WhatsNewPanel"

    onShownChanged: if (shown) OnboardingController.markWhatsNewRead()

    ListView {
        id: list
        objectName: "WhatsNew.list"
        anchors {
            left: parent.left
            right: parent.right
            top: parent.top
            bottom: bar.top
        }
        clip: true
        spacing: 24
        topMargin: 20
        bottomMargin: 20
        leftMargin: 20
        rightMargin: 20
        boundsBehavior: Flickable.StopAtBounds
        model: OnboardingController.releases
        ScrollBar.vertical: AppScrollBar {}

        delegate: ColumnLayout {
            id: release
            required property var modelData
            required property int index
            objectName: "WhatsNew.release." + index
            width: ListView.view.width - ListView.view.leftMargin - ListView.view.rightMargin
            spacing: 10
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
        }
    }

    Text {
        objectName: "WhatsNew.empty"
        visible: list.count === 0
        anchors {
            left: parent.left
            top: parent.top
            margins: 20
        }
        color: Theme.textMuted
        font: Theme.fontBody
        text: qsTr("No release notes are available in this build.")
    }

    // Done, under a line
    Rectangle {
        id: bar
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
