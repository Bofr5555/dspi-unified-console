import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP LAY2 -- The smoke driver's inspector (SmokeLay2.cpp): each popover's
// content in an InspectorSection, as Modern's inspector will host it, beside
// the Classic popovers that host the same content.
ToolWindow {
    id: host
    objectName: "SmokeLay2Window"
    windowId: "smokeLay2"
    title: "Inspector sections"
    defaultWidth: 760
    defaultHeight: 1000

    property int channel: -1
    property int band: 0
    property int output: 0
    property int devicesDone: 0
    property int graphDone: 0

    RowLayout {
        anchors.fill: parent
        anchors.margins: Theme.spacing
        spacing: Theme.spacingLarge

        ColumnLayout {
            Layout.preferredWidth: 340
            Layout.alignment: Qt.AlignTop
            spacing: Theme.spacing
            InspectorSection {
                id: limiter
                objectName: "Lay2.limiter"
                Layout.fillWidth: true
                title: "Limiter"
                LimiterSettingsContent {
                    objectName: "Lay2.limiter.content"
                    output: host.output
                    active: limiter.shown
                }
            }
            InspectorSection {
                id: linkwitz
                objectName: "Lay2.linkwitz"
                Layout.fillWidth: true
                title: "Linkwitz transform"
                LinkwitzContent {
                    objectName: "Lay2.linkwitz.content"
                    channel: host.channel
                    band: host.band
                    active: linkwitz.shown && host.channel >= 0
                }
            }
            InspectorSection {
                objectName: "Lay2.dashboard"
                Layout.fillWidth: true
                title: "Dashboard layout"
                DashboardLayoutContent {
                    objectName: "Lay2.dashboard.content"
                }
            }
        }
        ColumnLayout {
            Layout.preferredWidth: 340
            Layout.alignment: Qt.AlignTop
            spacing: Theme.spacing
            InspectorSection {
                objectName: "Lay2.graph"
                Layout.fillWidth: true
                title: "Graph options"
                GraphOptionsContent {
                    objectName: "Lay2.graph.content"
                    prefix: "Lay2.graphOptions"
                    onDone: host.graphDone++
                }
            }
            InspectorSection {
                objectName: "Lay2.devices"
                Layout.fillWidth: true
                title: "Devices"
                DevicesContent {
                    objectName: "Lay2.devices.content"
                    Layout.preferredHeight: 420
                    onDone: host.devicesDone++
                }
            }
        }
    }
}
