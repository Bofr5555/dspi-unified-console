import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// The Settings window (the Mac's SettingsView): the search field and the page
// list on the left, grouped and tinted as the Mac's SettingsCategory; on the
// right a title row with back and forward through the pages visited, the page,
// and the bars other packages register along the bottom (the save bar).
// Edit > Settings... opens it; Help > About opens it on About.
// -- WP DL7 -- Modern: no back and forward (the page list is the navigation),
// no lines between the list, the title and the page (tone and spacing), and
// the title left-aligned over the page's 760 px column.
// -- WP LAY1b -- A panel (Panel.qml): PanelTable's row for edit.settings gives
// its window's title and sizes.
Panel {
    id: panel
    objectName: "SettingsPanel"
    // -- WP QTPOP --
    fitBodies: [page.item]
    windowFont: Theme.fontBody   // -- WP LAY1b -- the window's font, as before

    property string query: ""
    readonly property var allRows: {
        SettingsController.revision   // re-read when pages come and go
        return SettingsController.sidebarRows(query)
    }
    // -- WP LAY5b -- Modern's page shows a group (its tab is the heading); a
    // search finds pages in every group
    readonly property var rows: !part.length || query.trim().length ? allRows : allRows.filter(r => r.group === part)
    // Another tab, once it shows: the group's page last shown, else its first.
    // (Not while a window has the panel: its page is not this one's to change.)
    property var lastOf: ({})
    property string asked: ""
    onPartChanged: {
        asked = part
        Qt.callLater(panel.takePart)
    }
    function takePart() {
        if (!shown || !part.length) return
        const want = asked
        asked = ""
        // The page was changed elsewhere meanwhile: its tab
        if (want !== part) return followPage()
        if (SettingsController.groupOf(SettingsController.currentPage) === part) return
        const pages = SettingsController.sidebarRows("").filter(r => r.group === part)
        const last = pages.find(r => r.id === lastOf[part])
        if (last || pages.length) SettingsController.open((last || pages[0]).id)
    }
    // A page of another group (a search, Back, a link): its tab
    function followPage() {
        const group = SettingsController.groupOf(SettingsController.currentPage)
        const of = Object.assign({}, lastOf)
        of[group] = SettingsController.currentPage
        lastOf = of
        if (shown && part.length && group.length && group !== part) partChosen(group)
    }
    Component.onCompleted: followPage()
    Connections {
        target: SettingsController
        function onCurrentPageChanged() { panel.followPage() }
    }
    // The first page the search leaves, for Down and Return
    readonly property string firstMatch: {
        const hit = rows.find(r => r.id !== undefined)
        return hit ? hit.id : ""
    }

    // -- WP UA5 -- the landing page: the last one chosen, else Device › Speakers.
    // After the show (a tab chosen while the page host is moving is not taken)
    function land() { if (shown) SettingsController.land() }
    onShownChanged: {
        if (shown) {
            SettingsController.ensureShown()
            Qt.callLater(panel.land)
        }
        Qt.callLater(panel.takePart)   // -- WP LAY5b --
    }

    RowLayout {
        anchors.fill: parent
        spacing: 0

        // ---- The page list --------------------------------------------------
        Rectangle {
            Layout.fillHeight: true
            Layout.preferredWidth: 215
            color: Theme.sidebar

            ColumnLayout {
                anchors.fill: parent
                anchors.margins: Theme.spacingSmall
                spacing: Theme.spacingSmall

                SettingsSearch {
                    id: search
                    objectName: "Settings.search"
                    Layout.fillWidth: true
                    onTextChanged: panel.query = text
                    // Down and Return open the first page left (behaviour.md §9)
                    onOpenFirst: if (panel.firstMatch.length) SettingsController.open(panel.firstMatch)
                }

                Text {
                    objectName: "Settings.noMatch"
                    Layout.fillWidth: true
                    visible: panel.rows.length === 0
                    text: qsTr("No matching settings")
                    font: Theme.fontSmall
                    color: Theme.textMuted
                    wrapMode: Text.Wrap
                    horizontalAlignment: Text.AlignHCenter
                    topPadding: Theme.spacing
                }

                ListView {
                    id: list
                    objectName: "Settings.sidebar"
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    clip: true
                    model: panel.rows
                    boundsBehavior: Flickable.StopAtBounds
                    ScrollBar.vertical: AppScrollBar {}
                    Accessible.role: Accessible.List
                    Accessible.name: qsTr("Settings pages")

                    delegate: SettingsSidebarRow {
                        required property var modelData
                        width: ListView.view.width
                        row: modelData
                    }
                }
            }
        }
        Rectangle {
            visible: !LayoutController.modern   // -- WP DL7 --
            Layout.fillHeight: true
            implicitWidth: 1
            color: Theme.separator
        }

        // ---- The page ---------------------------------------------------------
        ColumnLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            RowLayout {
                Layout.fillWidth: true
                Layout.margins: Theme.spacingSmall
                spacing: 2
                IconButton {
                    objectName: "Settings.back"
                    visible: !LayoutController.modern   // -- WP DL7 --
                    iconName: "chevron-left"
                    toolTip: qsTr("Back")
                    enabled: SettingsController.canGoBack
                    onClicked: SettingsController.back()
                }
                IconButton {
                    objectName: "Settings.forward"
                    visible: !LayoutController.modern   // -- WP DL7 --
                    iconName: "chevron-right"
                    toolTip: qsTr("Forward")
                    enabled: SettingsController.canGoForward
                    onClicked: SettingsController.forward()
                }
                Text {
                    objectName: "Settings.title"
                    Layout.fillWidth: true
                    // -- WP DL7 -- Modern: over the page's column (SettingsPage's x)
                    Layout.leftMargin: LayoutController.modern ? Theme.spacingLarge - Theme.spacingSmall : Theme.spacingSmall
                    text: SettingsController.currentTitle
                    font: Theme.fontTitle
                    color: Theme.text
                    elide: Text.ElideRight
                    Accessible.role: Accessible.Heading
                    Accessible.name: text
                }
                IconButton {
                    objectName: "Settings.close"
                    visible: !panel.inPage   // -- WP LAY3 -- the rail leaves a page
                    iconName: "x"
                    toolTip: qsTr("Close")
                    onClicked: panel.close()
                }
            }
            Rectangle {
                visible: !LayoutController.modern   // -- WP DL7 --
                Layout.fillWidth: true
                implicitHeight: 1
                color: Theme.separator
            }

            Loader {
                id: page
                objectName: "Settings.page"
                Layout.fillWidth: true
                Layout.fillHeight: true
                // The page's own type, made when shown
                sourceComponent: SettingsController.currentType.length
                                 ? Qt.createComponent("UConsole", SettingsController.currentType) : null
                onStatusChanged: if (status === Loader.Error) console.warn("Settings: the page", SettingsController.currentType, "did not load")
            }

            // Bars other packages register (SettingsRegistry::Footer): on every page
            Repeater {
                model: SettingsController.footers
                delegate: Loader {
                    required property string modelData
                    Layout.fillWidth: true
                    sourceComponent: Qt.createComponent("UConsole", modelData)
                }
            }
        }
    }
}
