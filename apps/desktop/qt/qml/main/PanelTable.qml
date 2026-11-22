pragma Singleton
import QtQuick
import UConsole

// -- WP LAY0 -- Every panel and where it lives (MODLAY 4.1, MPLAN 4.1): one
// row per action whose tool is a Panel. Shell opens a row's panel in a
// PanelWindow (Classic, and "Open in window"), or hands the row to the page
// host when the loaded shell has one (Modern: its section and tab). The
// action keeps its Registrar::Window line in its controller, which names the
// window (its objectName) and makes Shell handle the action; the row is the
// rest, so converting a window never touches a controller's file.
//
// A row: {
//   action     the menu action id (MenuTable), registered with Registrar::Window
//   panel      the Panel's QML type in module UConsole
//   windowId   its window's remembered place and size (windows/<windowId>):
//              the old window's, so a converted window opens where it was
//   section    its Modern section (MODLAY 2: overview, equaliser, routing,
//              effects, measure, analyse, library, system, devices, settings)
//   tab        its tab's id in that section
//   title      the window's title and the tab's name, translated
//   icon       a Lucide name (icons/lucide)
//   width, height        the window's default size
//   minWidth, minHeight  the window's least size (optional)
// }
// Work packages add their rows under their own marker, one block each.
QtObject {
    readonly property var panels: [
        // -- WP LAY0 -- the pilots (Analyse)
        {
            action: "tools.stats", panel: "StatsPanel", windowId: "stats",
            section: "analyse", tab: "stats", title: qsTr("System Statistics"), icon: "info",
            width: 960, height: 660, minWidth: 900, minHeight: 480
        },
        {
            action: "tools.interruptMonitor", panel: "MonitorPanel", windowId: "interruptMonitor",
            section: "analyse", tab: "interruptMonitor", title: qsTr("Interrupt Monitor"), icon: "activity",
            width: 900, height: 480, minWidth: 720, minHeight: 320
        },
        // -- WP INSTQ -- the audio link's installation (System)
        {
            action: "window.system", panel: "SystemPanel", windowId: "system",
            section: "system", tab: "status", title: qsTr("System"), icon: "network",
            width: 860, height: 640, minWidth: 560, minHeight: 360
        }
    ]

    // The row of `action`, or null
    function row(action) {
        for (const p of panels)
            if (p.action === action) return p
        return null
    }
}
