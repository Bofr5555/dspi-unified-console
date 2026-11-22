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
    readonly property bool devicesPage: pages.visible && pages.section === "devices" && pages.tab === "devices"

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
        pages.trail = []
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

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        // ---- The device tabs of several devices
        Rectangle {
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
            Layout.fillWidth: true
            visible: shell.topTabs && !shell.wizard
            objectName: "NavTabs"
            horizontal: true
            current: pages.section
            onChosen: section => pages.go(section, "")
        }
        StatusStrip {
            Layout.fillWidth: true
        }

        RowLayout {
            Layout.fillWidth: true
            Layout.fillHeight: true
            spacing: 0

            NavRail {
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
                    Layout.fillWidth: true
                    visible: !shell.senderPage
                    section: pages.section
                    tab: pages.tab
                    pills: shell.topTabs
                    onChosen: tab => pages.go(pages.section, tab)
                }
                Item {
                    Layout.fillWidth: true
                    Layout.fillHeight: true
                    PageHost {
                        id: pages
                        anchors.fill: parent
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
