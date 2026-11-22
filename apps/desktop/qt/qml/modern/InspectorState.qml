pragma Singleton
import QtQuick
import UConsole

// -- WP LAY4 -- Whether the Modern layout's inspector shows, shared by the
// pages that have one and the button in the row over them. It sits beside the
// page, shown or hidden as the person chose (LayoutController.arrangement);
// in a window narrower than `foldWidth` it is folded away and opens over the
// page instead, for as long as it is wanted there.
QtObject {
    id: state
    readonly property int foldWidth: 1280
    readonly property int width: 320
    // The main window's width (the pages set it)
    property real windowWidth: foldWidth
    readonly property bool narrow: windowWidth < foldWidth
    property bool over: false
    readonly property bool open: narrow ? over : LayoutController.arrangement.inspector !== "hidden"
    onNarrowChanged: over = false

    function show() {
        if (narrow) over = true
        else LayoutController.arrangement.inspector = "shown"
    }
    function toggle() {
        if (narrow) over = !over
        else LayoutController.arrangement.inspector = open ? "hidden" : "shown"
    }

    // The sections folded open or shut, by section, over pages and channels
    property var expanded: ({})
    function isExpanded(section, fallback) { return expanded[section] !== undefined ? expanded[section] : fallback }
    function setExpanded(section, on) {
        const all = Object.assign({}, expanded)
        all[section] = on
        expanded = all
    }
}
