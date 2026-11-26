import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP LAY3 -- The Modern layout (MODLAY rev 3, variant B): the top bar with
// the device tabs, the status strip, the rail of sections beside the page and
// its tabs, and the dock. No menu bar: every action is in the app menu or
// where it acts, and the menu table's shortcuts work as in Classic. The alink
// sender's page and Getting Started take the page area, as in Classic.
// MainWindow loads it (LayoutController); state is the controllers', so a
// switch of layouts loses nothing.
//
// LayoutController.arrangement decides the rail (or tabs across the top) and
// where the device tabs are, live.
Item {
    id: shell
    objectName: "ModernShell"
    required property ApplicationWindow mainWindow
    // No menu bar (MainWindow takes a shell's menuBar)
    readonly property Item menuBar: null

    readonly property bool topTabs: LayoutController.arrangement.navigation === "tabs"
    readonly property bool railDevices: !topTabs && LayoutController.arrangement.deviceTabs === "rail"
    // Several devices get a row of their own
    readonly property bool deviceRow: !railDevices && DeviceRegistry.devices.length > 1
    readonly property bool wizard: OnboardingController.wizardShown
    // The alink sender has no DSP screens; what it keeps stays reachable
    readonly property bool senderPage: DeviceInfoController.sender && !wizard
        && !(pages.action.length ? DeviceInfoController.senderKeeps(pages.action) : pages.section === "devices")
    readonly property bool devicesPage: pages.visible && pages.section === "devices" && (pages.tab === "devices" || pages.tab === "hubs")   // -- WP LAY5b -- hubs

    // -- WP LAY3 -- KEEPOPEN: several devices at once (a chosen device opens in a
    // tab beside the others). -- WP INT3 -- on: every controller follows the
    // active device (MDQT2a-c, with INT2's follow-up 1 in MDQT2b).
    readonly property bool severalDevices: true
    Binding {
        target: DeviceRegistry
        property: "keepOpen"
        value: true
        when: shell.severalDevices
        restoreMode: Binding.RestoreValue
    }

    // Classic's selection stays, and shows
    Component.onCompleted: {
        if (SidebarController.selectedChannel >= 0) pages.go("equaliser", "peq")
        if (SectionTable.homeShown) pages.go("overview", "home")   // -- WP UB2 --
        pages.trail = []
    }
    // -- WP UB2 -- (UXP B2, D4) Simple, or the installer's lock, opens on Home
    Connections {
        target: SectionTable
        function onHomeShownChanged() { if (SectionTable.homeShown && !shell.wizard) pages.go("overview", "home") }
    }
    Connections {
        target: SidebarController
        function onSelectionChanged() { shell.mainWindow.selectedChannel = SidebarController.selectedChannel }
    }
    Connections {
        target: shell.mainWindow
        function onSelectedChannelChanged() {
            if (shell.mainWindow.selectedChannel !== SidebarController.selectedChannel)
                SidebarController.select(shell.mainWindow.selectedChannel)
        }
    }

    // The menu bar's shortcuts (Classic's menus have them there)
    TableShortcuts {
        window: shell.mainWindow
    }

    // -- WP UA5 -- F6 and Shift+F6: the next or previous area that can take the
    // focus, in the order top bar (and status strip), sections, page (its tabs,
    // then the page), inspector, dock. The sections give the focus to the one on
    // show; another area to its first control. Ctrl+/ (and help.shortcuts) shows
    // every key.
    function areaOf(item) {
        for (let i = item; i; i = i.parent) {
            if (i.objectName === "Inspector") return "inspector"
            if (i === top || i === strip || i === deviceBar) return "top"
            if (i === rail || i === railTabs) return "rail"
            if (i === tabs || i === pages) return "page"
            if (i === dock) return "dock"
        }
        return ""
    }
    // The first control of `item` that takes the focus by Tab, depth first in
    // the order of its children (the Tab order); `skip` an item to leave out
    function firstStop(item, skip) {
        if (!item || !item.visible || !item.enabled || item === skip) return null
        if (item.activeFocusOnTab && item.width > 0 && item.height > 0) return item
        for (const child of item.children) {
            const found = firstStop(child, skip)
            if (found) return found
        }
        return null
    }
    // The inspector on show: the page's item named Inspector that is open
    function openInspector(item) {
        if (!item || !item.visible) return null
        if (item.objectName === "Inspector") return item.open ? item : null
        for (const child of item.children) {
            const found = openInspector(child)
            if (found) return found
        }
        return null
    }
    function focusArea(area) {
        let stop = null
        if (area === "top") stop = firstStop(deviceBar) || firstStop(top) || firstStop(strip)
        else if (area === "rail") return (rail.visible && rail.focusCurrent()) || (railTabs.visible && railTabs.focusCurrent())
        else if (area === "page") {
            const inspector = openInspector(pages)
            stop = firstStop(tabs) || firstStop(pages, inspector)
        } else if (area === "inspector") stop = firstStop(openInspector(pages))
        else if (area === "dock") stop = firstStop(dock)
        if (!stop) return false
        stop.forceActiveFocus(Qt.TabFocusReason)
        return stop.activeFocus
    }
    function cycleAreas(step) {
        const areas = ["top", "rail", "page", "inspector", "dock"]
        const now = areas.indexOf(areaOf(shell.mainWindow.activeFocusItem))
        let at = now < 0 ? (step > 0 ? -1 : 0) : now
        for (let i = 0; i < areas.length; ++i) {
            at = (at + step + areas.length) % areas.length
            if (focusArea(areas[at])) return
        }
    }
    Shortcut {
        sequences: ["F6"]
        context: Qt.WindowShortcut
        enabled: !shell.wizard
        onActivated: shell.cycleAreas(1)
    }
    Shortcut {
        sequences: ["Shift+F6"]
        context: Qt.WindowShortcut
        enabled: !shell.wizard
        onActivated: shell.cycleAreas(-1)
    }
    Shortcut {
        sequences: ["Ctrl+/"]
        context: Qt.WindowShortcut
        onActivated: keyList.open()
    }
    Connections {
        target: SettingsController
        function onShortcutsRequested() { keyList.open() }
    }

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // ---- The device tabs of several devices
        Rectangle {
            id: deviceBar   // -- WP UA5 --
            Layout.fillWidth: true
            visible: shell.deviceRow
            implicitHeight: 38
            color: Theme.sidebar
            DeviceTabs {
                objectName: "DeviceRow"
                anchors.fill: parent
                anchors.leftMargin: Theme.spacingSmall
                anchors.rightMargin: Theme.spacingSmall
                visible: shell.deviceRow
            }
        }
        TopBar {
            id: top
            Layout.fillWidth: true
            window: shell.mainWindow
            dialogs: presetDialogs
            showDevices: !shell.deviceRow && !shell.railDevices
            notifications: drawer.waiting
            onBellClicked: drawer.toggle()
            onSearchRequested: PaletteController.open()   // -- WP LAY6 --
        }
        NavRail {
            id: railTabs   // -- WP UA5 --
            Layout.fillWidth: true
            visible: shell.topTabs && !shell.wizard
            objectName: "NavTabs"
            horizontal: true
            current: pages.section
            onChosen: section => pages.go(section, "")
        }
        StatusStrip {
            id: strip   // -- WP UA5 --
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            NavRail {
                id: rail   // -- WP UA5 --
                Layout.fillHeight: true
                visible: !shell.topTabs && !shell.wizard
                current: pages.section
                onChosen: section => pages.go(section, "")
                headerShown: shell.railDevices
                header: DeviceTabs {
                    objectName: "DeviceRail"
                    visible: shell.railDevices
                    vertical: true
                    Layout.fillWidth: true
                }
            }
            ColumnLayout {
                Layout.fillWidth: true
                Layout.fillHeight: true
                visible: !shell.wizard
                spacing: 0
                PageTabs {
                    id: tabs   // -- WP UA5 --
                    Layout.fillWidth: true
                    visible: !shell.senderPage
                    section: pages.section
                    tab: pages.tab
                    pills: shell.topTabs
                    onChosen: tab => pages.go(pages.section, tab)
                }
                // -- WP UB2 -- (UXP B2) a page the installer's lock shows read-only says so once
                NoticeRow {
                    objectName: "ModernShell.lockRead"
                    Layout.fillWidth: true
                    Layout.leftMargin: Theme.spacing
                    Layout.rightMargin: Theme.spacing
                    visible: pages.lockRead && !shell.senderPage
                    iconName: "lock"
                    text: qsTr("Locked by the installer: this page is shown read-only. Volume, mute and presets are on Home.")
                    actionText: qsTr("Go to Home")
                    onAction: pages.go("overview", "home")
                }
                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    PageHost {
                        id: pages
                        anchors.fill: parent
                        dialogs: presetDialogs   // -- WP LAY5b --
                        visible: !shell.senderPage
                    }
                    // -- WP PGQT -- the alink sender's page in place of the DSP screens
                    Loader {
                        objectName: "ModernShell.sender"
                        anchors.fill: parent
                        active: shell.senderPage
                        visible: active
                        sourceComponent: SenderPage {}
                    }
                }
            }
            // -- WP QP17 -- Getting Started replaces the console while it runs
            Loader {
                objectName: "ModernShell.wizard"
                Layout.fillWidth: true
                Layout.fillHeight: true
                active: shell.wizard
                visible: active
                sourceComponent: GettingStarted {}
            }
        }

        Dock {
            id: dock
            Layout.fillWidth: true
            visible: !shell.wizard && !DeviceInfoController.sender
            dialogs: presetDialogs
        }
    }

    // ---- Over the pages
    // A notice, while the drawer is closed
    Rectangle {
        objectName: "ModernShell.toast"
        visible: toast.text.length > 0 && !drawer.open
        anchors.right: parent.right
        anchors.bottom: parent.bottom
        anchors.rightMargin: Theme.spacingLarge
        anchors.bottomMargin: (dock.visible ? dock.height : 0) + Theme.spacing
        width: Math.min(460, parent.width - 2 * Theme.spacingLarge)
        height: toast.implicitHeight
        radius: Theme.radiusSmall
        clip: true
        color: Theme.panel
        NoticeBanner {
            id: toast
            objectName: "ModernShell.toast.banner"
            width: parent.width
            height: implicitHeight
        }
    }
    NotificationDrawer {
        id: drawer
        anchors.fill: parent
        anchors.topMargin: top.y + top.height
        anchors.bottomMargin: dock.visible ? dock.height : 0
    }
    PresetDialogs {
        id: presetDialogs
    }
    // -- WP UA5 -- every key (Ctrl+/, help.shortcuts)
    ShortcutsDialog {
        id: keyList
        parent: Overlay.overlay
    }
    // -- WP LAY6 -- Ctrl+K and the top bar's search (PaletteController.opening)
    CommandPalette {
        window: shell.mainWindow
        pages: pages
    }

    // ---- The device switcher (Device > Devices…, the tabs' "+")
    DeviceSwitcher {
        id: switcher
        parent: shell
        x: shell.railDevices ? 84 : Theme.spacingSmall + (shell.deviceRow ? 0 : 100)
        y: shell.railDevices ? top.y + top.height + 4 : (shell.deviceRow ? 40 : top.y + top.height + 2)
        onClosed: if (!shell.devicesPage) DevicesController.listShown = false
        onManage: {
            switcher.close()
            pages.go("devices", "devices")
        }
        onHubs: pages.go("devices", "hubs")   // -- INTC (DL5 fold) --
    }
    Connections {
        target: DevicesController
        function onListShownChanged() {
            if (!DevicesController.listShown) switcher.close()
            else if (!shell.devicesPage) switcher.open()
        }
    }
    // The Devices page shows the same list: live while it is on show
    onDevicesPageChanged: {
        if (devicesPage && switcher.opened) switcher.close()
        DevicesController.listShown = devicesPage
    }
    // -- WP ODEC -- the page follows what an agent changes (AiController's gate:
    // never while the user is at a control or a dialog, rate-limited)
    Connections {
        target: AiController
        // -- WP SHOWFIX -- (finding 3) on the tab of the change (PEQ, Crossover, Delay & Gain), "" for the last one
        function onFollowSection(section, tab) {
            if (!shell.wizard && !shell.senderPage && section.length > 0 && (pages.section !== section || (tab.length > 0 && pages.tab !== tab)))
                pages.go(section, tab)
        }
    }
    Connections {
        target: LayoutController
        function onLayoutChanging() {
            DevicesController.listShown = false
            // -- WP INT3 -- Classic replaces again: the Binding above is destroyed
            // with the shell, and a destroyed Binding restores nothing
            DeviceRegistry.keepOpen = false
        }
    }
}
