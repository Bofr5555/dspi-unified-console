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
        // -- WP LAY1b -- measure, analyse, library, system, devices and settings
        {
            action: "tools.measure", panel: "MeasurePanel", windowId: "measure",
            section: "measure", tab: "measure", title: qsTr("Measure"), icon: "audio-waveform",
            width: 940, height: 720, minWidth: 720, minHeight: 520
        },
        {
            action: "tools.signalGenerator", panel: "SiggenPanel", windowId: "signalGenerator",
            section: "analyse", tab: "signalGenerator", title: qsTr("Signal Generator"), icon: "audio-lines",
            width: 780, height: 500, minWidth: 740, minHeight: 500
        },
        {
            action: "tools.spectrum", panel: "SpectrumPanel", windowId: "spectrum",
            section: "analyse", tab: "spectrum", title: qsTr("Spectrum Analyser"), icon: "chart-column",
            width: 860, height: 560, minWidth: 620, minHeight: 420
        },
        {
            action: "device.snapshots", panel: "SnapshotsPanel", windowId: "snapshots",
            section: "library", tab: "snapshots", title: qsTr("Snapshots"), icon: "save",
            width: 680, height: 720, minWidth: 520, minHeight: 460
        },
        {
            action: "edit.history", panel: "HistoryPanel", windowId: "history",
            section: "library", tab: "history", title: qsTr("History"), icon: "clock",
            width: 460, height: 420, minWidth: 360, minHeight: 240
        },
        {
            action: "autoeq.browse", panel: "AutoEqPanel", windowId: "autoeq",
            section: "library", tab: "autoeq", title: qsTr("AutoEQ - Browse Profiles"), icon: "headphones",
            width: 600, height: 640, minWidth: 450, minHeight: 460
        },
        {
            action: "device.events", panel: "EventsPanel", windowId: "events",
            section: "system", tab: "events", title: qsTr("Events"), icon: "list-ordered",
            width: 560, height: 480, minWidth: 400, minHeight: 280
        },
        {
            // minHeight grows while a note shows (FirmwarePanel.minimumHeight)
            action: "device.firmwareUpdate", panel: "FirmwarePanel", windowId: "firmware",
            section: "devices", tab: "firmware", title: qsTr("Firmware Update"), icon: "cpu",
            width: 460, height: 470, minWidth: 460, minHeight: 470
        },
        {
            // Modern: the app menu opens it; it has no section of its own (MODLAY 3.3)
            action: "help.whatsNew", panel: "WhatsNewPanel", windowId: "whatsNew",
            section: "settings", tab: "whatsNew", title: qsTr("What's New"), icon: "star",
            width: 460, height: 420, minWidth: 360, minHeight: 280
        },
        {
            // The one Settings host (MODLAY 4.3's SettingsHost): the window and the page
            action: "edit.settings", panel: "SettingsPanel", windowId: "settings",
            section: "settings", tab: "settings", title: qsTr("Settings"), icon: "settings",
            width: 860, height: 630, minWidth: 640, minHeight: 480
        }
    ]

    // The row of `action`, or null
    function row(action) {
        for (const p of panels)
            if (p.action === action) return p
        return null
    }
}
