import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP17 -- What's New (the Mac's WhatsNewView, 460×420): every release's
// headline, version and items from the core's notes, newest first, and Done.
// Help > What's New opens it; it also opens by itself once after an update.
// Showing it counts the notes of this build's firmware as read.
ToolWindow {
    id: window
    objectName: "WhatsNewWindow"
    windowId: "whatsNew"
    defaultWidth: 460
    defaultHeight: 420
    minimumWidth: 360
    minimumHeight: 280
    title: qsTr("What's New")

    onVisibleChanged: if (visible) OnboardingController.markWhatsNewRead()

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        Flickable {
            id: scroller
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            contentHeight: notes.implicitHeight + 40
            boundsBehavior: Flickable.StopAtBounds
            ScrollBar.vertical: AppScrollBar {}

            ColumnLayout {
                id: notes
                x: 20
                y: 20
                width: scroller.width - 40
                spacing: 24

                Repeater {
                    model: OnboardingController.releases
                    delegate: ColumnLayout {
                        id: release
                        required property var modelData
                        required property int index
                        objectName: "WhatsNew.release." + index
                        Layout.fillWidth: true
                        spacing: 10
                        RowLayout {
                            Layout.fillWidth: true
                            Text {
                                objectName: "WhatsNew.headline"
                                Layout.fillWidth: true
                                text: release.modelData.headline || ""
                                font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
                                color: Theme.text
                                wrapMode: Text.Wrap
                                Accessible.role: Accessible.Heading
                                Accessible.name: text
                            }
                            Text {
                                Layout.alignment: Qt.AlignTop
                                text: release.modelData.version || ""
                                font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontSmall.pixelSize })
                                color: Theme.textMuted
                            }
                        }
                        Repeater {
                            model: release.modelData.items || []
                            delegate: RowLayout {
                                required property string modelData
                                Layout.fillWidth: true
                                spacing: 8
                                Text {
                                    Layout.alignment: Qt.AlignTop
                                    text: "•"
                                    font: Theme.fontBody
                                    color: Theme.textMuted
                                }
                                Text {
                                    objectName: "WhatsNew.item"
                                    Layout.fillWidth: true
                                    text: modelData
                                    font: Theme.fontBody
                                    color: Theme.text
                                    wrapMode: Text.Wrap
                                }
                            }
                        }
                    }
                }
                Text {
                    objectName: "WhatsNew.empty"
                    visible: OnboardingController.releases.length === 0
                    text: qsTr("No release notes are available in this build.")
                    font: Theme.fontBody
                    color: Theme.textMuted
                }
            }
        }
        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }
        RowLayout {
            Layout.fillWidth: true
            Layout.margins: Theme.spacingLarge
            Item {
                Layout.fillWidth: true
                implicitHeight: 1
            }
            AppButton {
                objectName: "WhatsNew.done"
                text: qsTr("Done")
                primary: true
                onClicked: window.close()
            }
        }
    }

    Shortcut {
        sequences: ["Return", "Enter", "Escape"]
        onActivated: window.close()
    }
}
