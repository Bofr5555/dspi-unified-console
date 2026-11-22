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
    // -- WP LAY1a -- The size its window takes each time it shows, within the
    // screen (the matrix: its table), instead of fitting `fitBodies`; empty for none
    property size showSize: Qt.size(0, 0)
    // -- WP LAY1a -- Shown, and its window not minimised: while a person can
    // see it, what polls at a meter's rate may (-- WP QPI3 --)
    readonly property bool onScreen: shown && Window.visibility !== Window.Minimized

    // -- WP LAY1b -- What the old windows used of their window, through the host
    // Its window is minimised (a page host leaves it false); shown stays true
    property bool minimized: false
    // The least height its content needs now, when that is more than its
    // row's minHeight (0: the row's)
    property int minimumHeight: 0
    // The font its Controls and popups take from their window (a window's
    // `font`); undefined leaves the host's. A page host sets it on the page.
    property var windowFont: undefined
    // The person closes it (its window's close); a page host emits it when it
    // takes the panel away for good
    signal closing()
    // The panel asks its host to close it: the window closes, a page host
    // goes back or ignores it
    signal closeRequested()
    function close() { closeRequested() }
}
