import QtQuick 2.15
import QtQuick.Controls 2.15
import QtQuick.Window 2.15
import "components"

// What's New: the release notes (from the core), newest first. Shown once,
// automatically, after Console is updated, and from Help.
AppWindow {
    id: win
    title: qsTr("What's New")
    visible: false
    width: 460
    height: 420 + titlebarHeight
    minimumWidth: width
    maximumWidth: width
    minimumHeight: height
    maximumHeight: height

    property var releases: []

    function load() {
        var list = onboardingText.releaseNotes()
        // Newest first, whatever the file's order
        list.sort(function (a, b) { return firmware.compareVersions(b.version, a.version) })
        releases = list
    }
    // Notes newer than `shown` and no newer than this build. Empty on a new
    // install: there is nothing to catch up on.
    function unread(shown) {
        if (!shown) return []
        if (releases.length === 0) load()
        var current = Qt.application.version
        return releases.filter(function (r) {
            return firmware.compareVersions(r.version, shown) > 0 && firmware.compareVersions(r.version, current) <= 0
        })
    }
    onVisibleChanged: if (visible && releases.length === 0) load()

    Flickable {
        id: scroller
        width: parent.width
        height: parent.height - 52
        contentHeight: notes.height + 40
        clip: true
        boundsBehavior: Flickable.StopAtBounds
        ScrollBar.vertical: ScrollBar {}

        Column {
            id: notes
            x: 20
            y: 20
            width: scroller.width - 40
            spacing: 24

            Repeater {
                model: win.releases
                Column {
                    width: notes.width
                    spacing: 10
                    Item {
                        width: parent.width
                        height: headline.height
                        Text {
                            id: headline
                            width: parent.width - version.width - 12
                            wrapMode: Text.WordWrap
                            text: modelData.headline
                            font.pixelSize: 13
                            font.weight: Font.DemiBold
                            color: isMacOS ? MacColors.label : "white"
                        }
                        Text {
                            id: version
                            anchors.right: parent.right
                            text: modelData.version
                            font.pixelSize: 11
                            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45)
                        }
                    }
                    Repeater {
                        model: modelData.items
                        Row {
                            width: notes.width
                            spacing: 8
                            Text { text: "•"; font.pixelSize: 12; color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.45) }
                            Text {
                                width: parent.width - 16
                                wrapMode: Text.WordWrap
                                text: modelData
                                font.pixelSize: 12
                                lineHeight: 1.1
                                color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.85)
                            }
                        }
                    }
                }
            }
            Text {
                visible: win.releases.length === 0
                text: qsTr("No release notes are available in this build.")
                font.pixelSize: 12
                color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
            }
        }
    }

    Item {
        anchors.bottom: parent.bottom
        width: parent.width
        height: 52
        Rectangle { width: parent.width; height: 1; color: isMacOS ? MacColors.separator : Qt.rgba(1, 1, 1, 0.07) }
        AppButton {
            anchors.right: parent.right
            anchors.rightMargin: 16
            anchors.verticalCenter: parent.verticalCenter
            text: qsTr("Done")
            primary: true
            onClicked: win.close()
        }
    }
    // -- WP QD36 -- Only while this window has the focus: Qt 6 counts a hidden window as
    // active when its transient parent is, and its Escape then matched the main
    // window's keys ambiguously, so none of them ran (review c4h F5)
    Shortcut { sequences: ["Escape", "Return", "Enter"]; enabled: win.activeFocusItem !== null; onActivated: win.close() }
}
