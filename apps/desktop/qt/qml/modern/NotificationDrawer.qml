import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The notification drawer, from the top bar's bell: the notices
// that wait for the person (the banners that block nothing, Registrar::Notice,
// the NoticeCenter's among them) and, under them, what was said earlier in
// this session. It lies over the page's right side; a press beside it or
// Escape closes it. The banners live here also while it is closed, off the
// window and not enabled, so the bell can count them.
Item {
    id: drawer
    objectName: "NotificationDrawer"
    property bool open: false
    // What waits: the shown banners, and the notices behind the one on show
    readonly property int waiting: banners.bannersShown + NoticeCenter.waiting
    readonly property int kept: earlier.count

    function toggle() { open = !open }

    onOpenChanged: if (open) closeButton.forceActiveFocus()

    MouseArea {
        anchors.fill: parent
        visible: drawer.open
        onPressed: drawer.open = false
    }
    Shortcut {
        sequence: "Escape"
        enabled: drawer.open
        onActivated: drawer.open = false
    }

    Rectangle {
        id: sheet
        objectName: "NotificationDrawer.sheet"
        width: Math.min(420, drawer.width - 40)
        height: drawer.height
        x: drawer.open ? drawer.width - width : drawer.width + 1
        enabled: drawer.open
        color: Theme.panel
        // Presses on the sheet stay on it
        MouseArea { anchors.fill: parent }
        Rectangle {
            width: 1
            height: parent.height
            color: Theme.separator
        }
        Accessible.role: Accessible.Pane
        Accessible.name: qsTr("Notifications")

        ColumnLayout {
            anchors.fill: parent
            anchors.leftMargin: 1
            spacing: 0
            RowLayout {
                Layout.fillWidth: true
                Layout.margins: Theme.spacing
                Text {
                    Layout.fillWidth: true
                    text: qsTr("Notifications")
                    font: Theme.fontTitle
                    color: Theme.text
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
                IconButton {
                    objectName: "NotificationDrawer.events"
                    iconName: "list-ordered"
                    toolTip: MenuTable.textOf("device.events", false)
                    enabled: Shell.allows("device.events")
                    onClicked: {
                        drawer.open = false
                        Shell.trigger("device.events")
                    }
                }
                IconButton {
                    id: closeButton
                    objectName: "NotificationDrawer.close"
                    iconName: "x"
                    toolTip: qsTr("Close")
                    onClicked: drawer.open = false
                }
            }
            BannerHost {
                id: banners
                objectName: "NotificationDrawer.banners"
                Layout.fillWidth: true
                types: Shell.noticeBanners
            }
            Text {
                Layout.fillWidth: true
                Layout.margins: Theme.spacing
                visible: drawer.waiting === 0
                text: qsTr("Nothing waits for you.")
                font: Theme.fontBody
                color: Theme.textMuted
            }
            RowLayout {
                Layout.fillWidth: true
                Layout.leftMargin: Theme.spacing
                Layout.rightMargin: Theme.spacingSmall
                Layout.topMargin: Theme.spacing
                visible: earlier.count > 0
                SectionHeading {
                    Layout.fillWidth: true
                    text: qsTr("Earlier")
                }
                AppButton {
                    objectName: "NotificationDrawer.clear"
                    text: qsTr("Clear")
                    onClicked: earlier.clear()
                }
            }
            ListView {
                objectName: "NotificationDrawer.earlier"
                Layout.fillWidth: true
                Layout.fillHeight: true
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                ScrollBar.vertical: AppScrollBar {}
                model: earlier
                delegate: RowLayout {
                    required property string text
                    required property string kind
                    required property string time
                    width: ListView.view.width
                    spacing: Theme.spacingSmall
                    Icon {
                        Layout.leftMargin: Theme.spacing
                        Layout.alignment: Qt.AlignTop
                        Layout.topMargin: 4
                        name: kind === "info" ? "circle-check" : kind === "error" ? "circle-alert" : "triangle-alert"
                        size: 14
                        color: kind === "error" ? Theme.dangerText : kind === "warning" ? Theme.warning : Theme.textMuted
                    }
                    Text {
                        Layout.fillWidth: true
                        Layout.topMargin: 3
                        Layout.bottomMargin: 3
                        text: parent.text
                        textFormat: Text.PlainText
                        font: Theme.fontSmall
                        color: Theme.text
                        wrapMode: Text.Wrap
                    }
                    Text {
                        Layout.rightMargin: Theme.spacing
                        Layout.alignment: Qt.AlignTop
                        Layout.topMargin: 3
                        text: time
                        font: Theme.fontSmall
                        color: Theme.textMuted
                    }
                }
            }
        }
    }

    // Every notice that was on show, newest first
    ListModel { id: earlier }
    property int lastSerial: -1
    Connections {
        target: NoticeCenter
        function onChanged() {
            const n = NoticeCenter.current
            if (n.text === undefined || !n.text.length || n.serial === drawer.lastSerial) return
            drawer.lastSerial = n.serial
            earlier.insert(0, {
                text: n.text, kind: n.kind || "info", time: new Date().toLocaleTimeString(Qt.locale(), Locale.ShortFormat)
            })
            if (earlier.count > 50) earlier.remove(50, earlier.count - 50)
        }
    }
}
