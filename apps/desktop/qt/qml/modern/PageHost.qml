import QtQuick
import QtQuick.Controls.Basic
import UConsole

// -- WP LAY3 -- The Modern layout's page area: one page per tab (SectionTable),
// made on its first visit and kept, the current one shown. It is the shell's
// page host (Shell.pageHost): an action whose tool is a panel goes to its page
// (showPanel), from a menu, a shortcut or a button alike.
//
// A panel is shown in one place at a time: while its window is open ("Open in
// window", or left from Classic; Shell.shownWindows) its page says so and the
// page's panel is not shown.
Item {
    id: host
    objectName: "PageHost"

    property string section: "overview"
    property string tab: "overview"
    readonly property var entry: SectionTable.tab(section, tab)
    // -- WP LAY5b -- The key of the page that is current (its tab's, or the one
    // page of its panel's parts)
    readonly property string pageKey: entry ? entry.page : ""
    // The shell's PresetDialogs, for the pages (ContextMenus.slot)
    property Item dialogs: null
    // Devices > Hubs shows the hub's PIN and key itself: no dialog for them
    readonly property bool pairsInline: onScreen && section === "devices" && tab === "hubs"
    // The panel's action while "Show Here" takes it back from its window: it
    // moves, it is not closed (Measure runs on)
    property string moving: ""
    // A tab that went (Measure's while a connection may not play): its section's first
    readonly property bool entryShows: entry === null || entry.quiet || SectionTable.shows(section, tab)
    onEntryShowsChanged: if (!entryShows) Qt.callLater(leaveHidden)
    function leaveHidden() {
        if (entryShows || released) return
        const want = SectionTable.tab(section, "")
        if (want) show(want)
    }

    // The current page's panel action, "" for a page that is no panel
    readonly property string action: entry && entry.row ? entry.row.action : ""
    // Whether the person can see a page at all (not in the tray, not under the
    // wizard); pages poll and animate only then
    readonly property bool onScreen: visible && !released && Window.visibility !== Window.Hidden
    // The shell goes: no page is shown any more
    property bool released: false
    // Where the person was before, newest last: back() returns there
    property var trail: []
    // The tab last shown of each section
    property var lastTabs: ({})

    // Section `id`, on `tabId`, or on the tab it was last on ("")
    function go(id, tabId) {
        const want = SectionTable.tab(id, tabId || SectionTable.now(id) || lastTabs[id] || "")   // -- WP LAY5b -- now
        if (!want) return false
        if (want.section === section && want.tab === tab) {
            visit(want)
            return true
        }
        const before = trail.slice(-19)
        before.push({ section: section, tab: tab })
        trail = before
        show(want)
        return true
    }
    // The page the person came from (a panel that asks to be closed)
    function back() {
        const list = trail.slice()
        const at = list.pop() || { section: "overview", tab: "overview" }
        trail = list
        const want = SectionTable.tab(at.section, at.tab)
        if (want) show(want)
    }
    function show(want) {
        // The page is made before it is current: it shows in the same frame
        visit(want)
        section = want.section
        tab = want.tab
        const tabs = Object.assign({}, lastTabs)
        if (!want.quiet) tabs[want.section] = want.tab
        lastTabs = tabs
    }
    function visit(want) {
        for (let i = 0; i < visited.count; ++i)
            if (visited.get(i).key === want.page) return
        visited.append({ key: want.page, sectionId: want.section, tabId: want.tab })   // -- WP LAY5b -- page
    }
    // Shell.trigger: PanelTable's `row`; false lets a window open
    function showPanel(row) {
        if (released || OnboardingController.wizardShown || Shell.shownWindows.indexOf(row.action) >= 0) return false
        // -- WP LAY5b -- of a panel's parts, the one it was on
        const parts = SectionTable.tabs(row.section).filter(t => t.row && t.row.action === row.action)
        const want = SectionTable.part(parts, lastTabs[row.section])
        return want ? go(row.section, want.tab) : false
    }
    // The page of `key` ("effects/tube"), null before its first visit
    function page(key) {
        for (let i = 0; i < pages.count; ++i) {
            const p = pages.itemAt(i)
            if (p && p.key === key) return p
        }
        return null
    }
    // The shell goes for good: panels end what they started, as with their
    // windows' close (Panel.closing)
    function release() {
        if (released) return
        released = true
        for (let i = 0; i < pages.count; ++i) {
            const p = pages.itemAt(i)
            if (p) p.release()
        }
    }

    Component.onCompleted: {
        visit(SectionTable.tab(section, tab))
        Shell.pageHost = host
    }
    Binding {
        target: Shell
        property: "pageAction"
        value: host.onScreen ? host.action : ""
        when: !host.released
        restoreMode: Binding.RestoreNone
    }
    Connections {
        target: LayoutController
        // Before the shell's Loader drops it: afterwards no binding runs any more
        function onLayoutChanging() {
            host.release()
            Shell.pageAction = ""
            if (Shell.pageHost === host) Shell.pageHost = null
        }
    }

    ListModel { id: visited }
    Repeater {
        id: pages
        model: visited
        delegate: HostedPage {
            required property string key
            required property string sectionId
            required property string tabId
            objectName: "PageHost.page." + key
            anchors.fill: parent
            pageHost: host
            entry: SectionTable.tab(sectionId, tabId)
            current: key === host.pageKey   // -- WP LAY5b --
        }
    }

    // One page: a panel, or a section's own content
    component HostedPage: Control {
        id: page
        property Item pageHost: null
        property var entry: null
        property bool current: false
        readonly property var row: entry ? entry.row : null
        readonly property string action: row ? row.action : ""
        readonly property Item content: body.item
        // Its window is open: the panel is there, not here
        readonly property bool inWindow: action.length > 0 && Shell.shownWindows.indexOf(action) >= 0
        readonly property bool shown: current && pageHost !== null && pageHost.onScreen && !inWindow
        readonly property bool locked: action.length > 0 && DeviceInfoController.sender && !DeviceInfoController.senderKeeps(action)

        // -- WP LAY5b -- Which of its panel's parts; kept while the page is not current
        readonly property string wanted: current && pageHost.entry ? pageHost.entry.part : ""
        onWantedChanged: if (wanted.length && content) content.part = wanted

        function release() { if (row && content && !inWindow) content.closing() }

        visible: current
        padding: 0
        // A panel's window has the app's font, or its own (Panel.windowFont)
        font: row ? (content && content.windowFont !== undefined ? content.windowFont : Qt.application.font) : Theme.fontBody
        background: Rectangle { color: Theme.window }

        contentItem: Item {
            Flickable {
                id: scroll
                objectName: page.objectName + ".scroll"
                anchors.fill: parent
                anchors.rightMargin: aside.width   // -- WP LAY5a --
                // A panel keeps its window's least size; a smaller page scrolls
                contentWidth: Math.max(width, page.row ? page.row.minWidth || 0 : 0)
                contentHeight: Math.max(height, page.row ? Math.max(page.row.minHeight || 0, page.content ? page.content.minimumHeight : 0) : 0)
                clip: true
                boundsBehavior: Flickable.StopAtBounds
                interactive: contentHeight > height || contentWidth > width
                ScrollBar.vertical: AppScrollBar { visible: scroll.contentHeight > scroll.height }
                ScrollBar.horizontal: AppScrollBar { visible: scroll.contentWidth > scroll.width }

                Loader {
                    id: body
                    // A narrow form (the leveller, the upmixer) keeps near its window's width
                    readonly property real most: !page.row ? Infinity
                                                : page.row.maxWidth ? page.row.maxWidth
                                                : page.row.width < 520 ? Math.max(page.row.width, 560) : Infinity
                    width: Math.min(scroll.contentWidth, most)
                    height: scroll.contentHeight
                    x: Math.max(0, (scroll.contentWidth - width) / 2)
                    visible: !page.inWindow
                    enabled: !page.locked
                    Component.onCompleted: {
                        const type = page.row ? page.row.panel : page.entry ? page.entry.content : ""
                        if (!type.length) return
                        sourceComponent = Qt.createComponent("UConsole", type)
                    }
                    onLoaded: {
                        if (!page.row) {
                            if (item.host !== undefined) item.host = page.pageHost
                            item.shown = Qt.binding(() => page.shown)
                            return
                        }
                        item.actionId = page.action
                        item.inPage = true
                        if (page.wanted.length) item.part = page.wanted   // -- WP LAY5b --
                        item.locked = Qt.binding(() => page.locked)
                        item.shown = Qt.binding(() => page.shown)
                    }
                    onStatusChanged: if (status === Loader.Error) console.warn("PageHost: the page", page.key, "did not load")
                }
                Connections {
                    target: page.row ? body.item : null
                    // Done, Cancel, a close button: back to where the person was
                    function onCloseRequested() { if (page.current) page.pageHost.back() }
                    // -- WP LAY5b -- Another part of the panel: its tab
                    function onPartChosen(part) { if (page.current) page.pageHost.go(page.entry.section, part) }
                }
            }

            // -- WP LAY5a -- The section's inspector, at the panel's right
            // (or over it, in a narrow window: it takes no width then)
            Loader {
                id: aside
                objectName: page.objectName + ".aside"
                readonly property string type: page.entry && page.entry.aside ? page.entry.aside : ""
                anchors.right: parent.right
                width: item ? item.implicitWidth : 0
                height: parent.height
                enabled: !page.locked
                active: type.length > 0
                Component.onCompleted: if (type.length) sourceComponent = Qt.createComponent("UConsole", type)
                onLoaded: {
                    if (item.host !== undefined) item.host = page.pageHost
                    if (item.tab !== undefined) item.tab = page.entry.tab
                    item.shown = Qt.binding(() => page.shown)
                }
                onStatusChanged: if (status === Loader.Error) console.warn("PageHost: the aside of", page.key, "did not load")
            }

            // The panel is in its own window
            Column {
                objectName: page.objectName + ".inWindow"
                anchors.centerIn: parent
                visible: page.inWindow
                spacing: Theme.spacing
                Text {
                    anchors.horizontalCenter: parent.horizontalCenter
                    //: %1 is a page's name
                    text: page.row ? qsTr("%1 is open in its own window").arg(page.row.tabTitle || page.row.title) : ""   // -- WP LAY5b -- the panel's, not a part's
                    font: Theme.fontTitle
                    color: Theme.textMuted
                }
                Row {
                    anchors.horizontalCenter: parent.horizontalCenter
                    spacing: Theme.spacingSmall
                    AppButton {
                        objectName: page.objectName + ".raise"
                        text: qsTr("Go to Window")
                        onClicked: Shell.openWindow(page.action)
                    }
                    AppButton {
                        objectName: page.objectName + ".bringBack"
                        text: qsTr("Show Here")
                        primary: true
                        onClicked: {
                            page.pageHost.moving = page.action   // -- WP LAY5b --
                            Shell.closeWindow(page.action)
                            page.pageHost.moving = ""
                        }
                    }
                }
            }
        }
    }
}
