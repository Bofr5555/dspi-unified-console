import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP2 -- The smoke driver's host for the channel pages (SmokeChannel.cpp):
// an input page or an output page, as the main window's right pane will show
// them, on the demo device.
ToolWindow {
    id: host
    objectName: "SmokeChannelWindow"
    windowId: "smokeChannel"
    title: "Channel pages"
    defaultWidth: 980
    defaultHeight: 640

    // "input" or "output"
    property string page: "input"
    property int input: 0
    property int output: 0

    StackLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        currentIndex: host.page === "output" ? 1 : 0
        InputDetail { input: host.input }
        OutputDetail { output: host.output }
    }
}
