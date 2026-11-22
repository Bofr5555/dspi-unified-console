import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP3 -- The graph's gear and its popover (the Mac's GraphOptionsButton
// and GraphOptionsPanel): the analyser's part (GraphController.RtaOptions,
// QP9's), Graph Setup, and Pop Out Graph; the setup page holds the scale, the
// grid and labels, and the curves, the same preferences as the Graphing
// settings page. The popover is the gear's child, so a click on the gear while
// it is open closes it once and does not reopen it (behaviour.md §26).
// -- WP LAY2 -- The popover's content is GraphOptionsContent, which Modern
// shows as well.
IconButton {
    id: gear
    readonly property bool open: panel.opened
    // In the pop-out window: no Pop Out row, and the follow switch
    property bool popOut: false

    objectName: (popOut ? "GraphWindow" : "GraphHost") + ".options"
    iconName: "settings"
    iconColor: open ? Theme.text : Theme.textMuted
    toolTip: GraphController.selectedChannel < 0 ? qsTr("Graph options for the dashboard")
                                                 : qsTr("Graph options for this channel page")
    onClicked: panel.opened ? panel.close() : panel.open()

    PopoverPanel {
        id: panel
        // The setup page shows; reopening starts on the main page
        property alias setup: content.setup

        objectName: gear.objectName + ".popover"
        title: qsTr("Graph options")
        onAboutToShow: content.setup = false
        x: gear.width - width
        y: gear.height + 4
        width: 280
        padding: 0
        topPadding: 6
        bottomPadding: 6

        contentItem: GraphOptionsContent {
            id: content
            prefix: gear.objectName
            popOut: gear.popOut
            onDone: panel.close()
        }
    }
}
