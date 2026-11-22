import QtQuick
import UConsole

// -- WP LAY0 -- A panel: a tool's content without its host (MODLAY 4.1). A
// host shows it: PanelWindow in a window of its own (Classic, and "Open in
// window" in Modern), or a Modern page. The host sets what it knows; the
// panel never looks at its window. Its title, icon, section and sizes are its
// row in PanelTable, not properties here. Item names keep the window's prefix
// ("StatsWindow.platform"), so the smoke driver finds them in either host.
Item {
    // Whether the person can see it now: its window shows, or its page is the
    // current one. A panel polls or animates only while shown.
    property bool shown: false
    // Its content does nothing on the open device (ToolWindow.senderLocked:
    // the alink sender); its own shortcuts are off while locked
    property bool locked: false
    // The action that opened it (its PanelTable row's), "" until a host sets it
    property string actionId: ""
    // The scrolling Flickables a window fits its height to (ToolWindow.fitBodies)
    property var fitBodies: []
}
