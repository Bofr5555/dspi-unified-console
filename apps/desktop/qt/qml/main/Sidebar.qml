import QtQuick
import QtQuick.Controls.Basic
import QtQuick.Layouts
import UConsole

// -- WP QP1 -- The sidebar (ContentView.swift's sidebar column): INPUTS and
// OUTPUTS, one ChannelRow per channel; under them the quick strip, the preset /
// source / volume controls, the C0 and C1 loads and the device button.
//
// The selection lives in SidebarController (selectedChannel, selectedKind,
// selectedIndex), which the right pane reads; `selectedChannel` and
// `channelSelected` mirror it for the main window.
Rectangle {
    id: sidebar
    objectName: "Sidebar"
    color: Theme.sidebar
    property int selectedChannel: -1
    signal channelSelected(int channel)

    // The row being renamed, -1 for none (one at a time)
    property int renamingChannel: -1

    // Tighter rows when many inputs show, as the Mac does at 6 and 8
    readonly property int rowHeight: DeviceModel.numInputs >= 8 ? 23 : DeviceModel.numInputs >= 6 ? 27 : 28
    // Wider names while an output shows a derived label ("Out 2 · R · High")
    readonly property int nameWidth: SidebarController.derivedLabels ? 112 : 80

    Connections {
        target: SidebarController
        function onSelectionChanged() {
            sidebar.renamingChannel = -1
            sidebar.channelSelected(SidebarController.selectedChannel)
        }
    }
    // Someone else moved the main window's selection: follow it
    onSelectedChannelChanged: if (selectedChannel !== SidebarController.selectedChannel) SidebarController.select(selectedChannel)

    ColumnLayout {
        anchors.fill: parent
        spacing: 0

        ListView {
            id: list
            objectName: "Sidebar.list"
            Layout.fillWidth: true
            Layout.fillHeight: true
            clip: true
            model: DeviceModel.channels
            // Faded out, not emptied, with no device (the Mac's crossfade)
            opacity: DeviceModel.connected ? 1 : 0
            enabled: DeviceModel.connected
            Behavior on opacity { NumberAnimation { duration: 300; easing.type: Easing.InOutQuad } }
            ScrollBar.vertical: AppScrollBar {}
            section.property: "kind"
            section.delegate: Text {
                required property string section
                leftPadding: Theme.spacing
                topPadding: Theme.spacing
                bottomPadding: 4
                text: section === "input" ? qsTr("INPUTS") : qsTr("OUTPUTS")
                font: Theme.fontHeading
                color: Theme.textMuted
                Accessible.role: Accessible.Heading
                Accessible.name: text
            }
            delegate: ChannelRow {
                rowHeight: sidebar.rowHeight
                nameWidth: sidebar.nameWidth
                renaming: sidebar.renamingChannel === channel
                onRenameRequested: sidebar.renamingChannel = channel
                onRenameFinished: if (sidebar.renamingChannel === channel) sidebar.renamingChannel = -1
            }
            Accessible.role: Accessible.List
            Accessible.name: qsTr("Channels")
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }

        QuickStrip {
            Layout.fillWidth: true
            Layout.topMargin: Theme.spacingSmall
            Layout.bottomMargin: Theme.spacingSmall
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }

        GlobalControls {
            Layout.fillWidth: true
            Layout.margins: Theme.spacing
        }

        Rectangle {
            Layout.fillWidth: true
            implicitHeight: 1
            color: Theme.separator
        }

        // The cores' loads (Components.swift's CpuSection), a dash with no device
        RowLayout {
            Layout.fillWidth: true
            Layout.leftMargin: Theme.spacing
            Layout.rightMargin: Theme.spacing
            Layout.topMargin: Theme.spacingSmall
            CpuMeter { core: 0; load: Poller.cpu0 }
            Item { Layout.fillWidth: true }
            CpuMeter { core: 1; load: Poller.cpu1 }
        }

        DeviceStatus {
            Layout.fillWidth: true
            Layout.margins: Theme.spacingSmall
        }
    }

    component CpuMeter: RowLayout {
        id: meter
        required property int core
        required property int load
        readonly property bool live: Poller.running
        objectName: "Sidebar.cpu" + core
        readonly property string shownName: live ? load + "%" : "–"
        spacing: 4
        Text {
            text: "C" + meter.core + ":"
            font: Theme.fontCaption
            color: Theme.textMuted
        }
        Rectangle {
            implicitWidth: 40
            implicitHeight: 6
            radius: 2
            color: Theme.separator
            Rectangle {
                width: meter.live ? parent.width * Math.max(0, Math.min(100, meter.load)) / 100 : 0
                height: parent.height
                radius: 2
                color: meter.load > 90 ? Theme.danger : Theme.accent
            }
        }
        Text {
            text: meter.shownName
            font: Qt.font({ family: Theme.fontMono.family, pixelSize: Theme.fontCaption.pixelSize })
            color: Theme.text
        }
        Accessible.role: Accessible.ProgressBar
        //: %1 is the core's number, 0 or 1
        Accessible.name: qsTr("Core %1 load").arg(core)
        Accessible.description: shownName
    }
}
