import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// Settings > About (the Mac's GeneralSettingsTab): the app, its version for
// this platform and the credit lines; the makers and the project; the links
// with their note; the credits; and the third-party notices, each opened in a
// text viewer (behaviour.md §27). The wording is the core's About table
// (AboutController); sections other packages add (the licence) go below.
SettingsPage {
    id: page
    objectName: "AboutPage"

    Component.onCompleted: AboutController.rescanNotices()

    SettingsGroup {
        RowLayout {
            Layout.fillWidth: true
            spacing: 14
            Icon {
                Layout.alignment: Qt.AlignTop
                name: "volume-2"
                size: 32
                color: Theme.accentText
            }
            ColumnLayout {
                Layout.fillWidth: true
                spacing: 2
                Text {
                    objectName: "About.name"
                    text: AppIdentity.displayName
                    font: Qt.font({ pixelSize: Theme.fontBody.pixelSize, weight: Font.DemiBold })
                    color: Theme.text
                }
                Text {
                    text: qsTr("USB Audio DSP Controller")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
                Text {
                    objectName: "About.version"
                    text: qsTr("Version %1 %2").arg(AppIdentity.version).arg(AboutController.platformLabel)
                    font: Theme.fontSmall
                    color: Theme.textMuted
                }
                Repeater {
                    model: AboutController.creditLines
                    delegate: Text {
                        required property string modelData
                        required property int index
                        objectName: "About.credit." + index
                        Layout.fillWidth: true
                        text: modelData
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        wrapMode: Text.Wrap
                    }
                }
            }
        }
    }

    // -- WP PGQT -- The open device's protocol (read once per connection), and
    // the raw request by command id
    SettingsGroup {
        objectName: "About.device"
        title: qsTr("Connected Device")
        iconName: "cpu"
        DeviceInfoRows {}
        RowLayout {
            Layout.fillWidth: true
            Item { Layout.fillWidth: true }
            AppButton {
                objectName: "About.raw"
                text: qsTr("Raw Request…")
                enabled: DeviceModel.connected
                onClicked: rawDialog.open()
            }
        }
    }

    RawRequestDialog {
        id: rawDialog
        parent: page.Window.window ? page.Window.window.contentItem : page
    }

    SettingsGroup {
        title: qsTr("Made By")
        iconName: "heart"
        Repeater {
            model: AboutController.makers
            delegate: ColumnLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 2
                LinkRow {
                    title: modelData.name
                    url: modelData.url
                }
                Text {
                    Layout.fillWidth: true
                    text: modelData.made.join(", ")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
        }
        LinkRow {
            objectName: "About.project"
            title: qsTr("%1 on GitHub").arg(AppIdentity.displayName)
            url: AppIdentity.projectUrl
        }
    }

    SettingsGroup {
        title: AboutController.linksTitle.length ? AboutController.linksTitle : qsTr("Links & Support")
        iconName: "external-link"
        Text {
            visible: text.length > 0
            Layout.fillWidth: true
            text: AboutController.linksNote
            font: Theme.fontSmall
            color: Theme.textMuted
            wrapMode: Text.Wrap
        }
        Repeater {
            model: AboutController.links
            delegate: LinkRow {
                required property var modelData
                objectName: "About.link." + modelData.id
                title: modelData.title
                detail: modelData.detail
                url: modelData.url
            }
        }
    }

    SettingsGroup {
        title: qsTr("Credits")
        iconName: "star"
        Repeater {
            model: AboutController.credits
            delegate: ColumnLayout {
                required property var modelData
                Layout.fillWidth: true
                spacing: 1
                Text {
                    Layout.fillWidth: true
                    text: modelData.name
                    font: Theme.fontBody
                    color: Theme.text
                }
                Text {
                    Layout.fillWidth: true
                    text: modelData.what
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                }
            }
        }
    }

    SettingsExtensions {
        page: "about"
    }

    SettingsGroup {
        objectName: "About.notices"
        title: qsTr("Third-Party Notices")
        iconName: "file-down"
        footer: AboutController.notices.length ? "" : qsTr("This build carries no notice files; a package lists them here.")
        Repeater {
            model: AboutController.notices
            delegate: AbstractButton {
                id: notice
                required property var modelData
                required property int index
                objectName: "About.notice." + index
                Layout.fillWidth: true
                implicitHeight: Theme.rowHeight
                hoverEnabled: true
                focusPolicy: Qt.TabFocus
                onClicked: {
                    AboutController.openNotice(index)
                    viewer.open()
                }
                background: Rectangle {
                    radius: Theme.radiusSmall
                    color: notice.hovered ? Theme.controlHover : "transparent"
                    border.width: notice.visualFocus ? 2 : 0
                    border.color: Theme.focusRing
                }
                contentItem: RowLayout {
                    spacing: Theme.spacingSmall
                    Icon {
                        name: "info"
                        size: 14
                        color: Theme.accentText
                    }
                    Text {
                        Layout.fillWidth: true
                        text: notice.modelData.title
                        font: Theme.fontBody
                        color: Theme.accentText
                        elide: Text.ElideRight
                    }
                    Text {
                        text: notice.modelData.detail
                        font: Theme.fontSmall
                        color: Theme.textMuted
                        elide: Text.ElideLeft
                        Layout.maximumWidth: 220
                    }
                }
                Accessible.role: Accessible.Link
                Accessible.name: modelData.title
                Accessible.description: modelData.detail
                Accessible.onPressAction: notice.clicked()
            }
        }
    }

    NoticeViewer {
        id: viewer
        objectName: "About.viewer"
        parent: page.Window.window ? page.Window.window.contentItem : page
    }

    // A web link row: title (and detail) opening the page in the browser
    component LinkRow: AbstractButton {
        id: link
        property string title
        property string detail
        property string url
        Layout.fillWidth: true
        implicitHeight: Theme.rowHeight
        hoverEnabled: true
        focusPolicy: Qt.TabFocus
        enabled: url.length > 0
        onClicked: AboutController.openLink(url)
        background: Rectangle {
            radius: Theme.radiusSmall
            color: link.hovered ? Theme.controlHover : "transparent"
            border.width: link.visualFocus ? 2 : 0
            border.color: Theme.focusRing
        }
        contentItem: RowLayout {
            spacing: Theme.spacingSmall
            Icon {
                name: "external-link"
                size: 14
                color: Theme.accentText
            }
            Text {
                text: link.title
                font: Theme.fontBody
                color: Theme.accentText
            }
            Text {
                Layout.fillWidth: true
                text: link.detail
                font: Theme.fontSmall
                color: Theme.textMuted
                elide: Text.ElideRight
            }
        }
        HelpTip {
            text: link.url
            visible: link.hovered
        }
        Accessible.role: Accessible.Link
        Accessible.name: title
        Accessible.description: url
        Accessible.onPressAction: link.clicked()
    }
}
