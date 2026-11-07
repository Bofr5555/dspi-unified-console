import QtQuick 2.15
import QtQuick.Controls 2.15

// Sidebar footer: the load of each core above, connection status below.
Item {
    id: cpuRoot
    height: 54

    property int cpu0: 0
    property int cpu1: 0
    readonly property bool connected: bridge.connected
    // A DSPi is plugged in but this user may not open it (no udev rule)
    readonly property bool noAccess: !connected && bridge.availableSerials.length === 0 && bridge.inaccessibleDevices > 0

    readonly property string serialTail: bridge.selectedSerial.slice(-8)
    readonly property string fullName: bridge.selectedSerial.length >= 8 ? "DSPi (" + serialTail + ")" : "DSPi"

    // One core: its name, a bar (at most 40 px, as on the Mac) and its load
    component CoreMeter: Row {
        id: meter
        property int core: 0
        property int load: 0
        readonly property real trackWidth: track.width
        readonly property real fillWidth: fill.visible ? fill.width : 0
        readonly property string valueText: value.text
        spacing: 5
        Text {
            anchors.verticalCenter: parent.verticalCenter
            text: "C" + meter.core + ":"
            font.pixelSize: 10
            color: isMacOS ? MacColors.secondaryLabel : Qt.rgba(1, 1, 1, 0.5)
        }
        Rectangle {
            id: track
            anchors.verticalCenter: parent.verticalCenter
            width: cpuRoot.barWidth
            height: 6
            radius: 2
            color: isMacOS ? MacColors.opacity(MacColors.gray, 0.3) : Qt.rgba(1, 1, 1, 0.12)
            Rectangle {
                id: fill
                visible: cpuRoot.connected
                width: parent.width * Math.min(1, Math.max(0, meter.load / 100))
                height: parent.height
                radius: 2
                color: isMacOS ? (meter.load > 90 ? MacColors.red : MacColors.blue) : meter.load > 80 ? "#ff9f0a" : "#0a7cff"
            }
        }
        Text {
            id: value
            anchors.verticalCenter: parent.verticalCenter
            width: 28
            text: cpuRoot.connected ? meter.load + "%" : "—"
            font.pixelSize: 10
            color: isMacOS ? MacColors.label : Qt.rgba(1, 1, 1, 0.65)
        }
    }
    // Two meters side by side, what's left after their labels, values and margins
    readonly property real barWidth: Math.max(20, Math.min(40, (width - 32 - 12) / 2 - 56))

    CoreMeter {
        objectName: "cpuCore0"
        x: 16
        y: 8
        core: 0
        load: cpuRoot.cpu0
    }
    CoreMeter {
        objectName: "cpuCore1"
        anchors.right: parent.right
        anchors.rightMargin: 16
        y: 8
        core: 1
        load: cpuRoot.cpu1
    }

    Row {
        id: status
        objectName: "cpuStatus"   // -- WP QD37 -- the smoke driver reads statusTip
        anchors.right: parent.right
        anchors.rightMargin: 16
        anchors.bottom: parent.bottom
        anchors.bottomMargin: 8
        spacing: 6

        // Status dot with a faint halo when connected
        Item {
            width: 12; height: 12
            anchors.verticalCenter: parent.verticalCenter
            Rectangle {
                anchors.centerIn: parent
                width: 12; height: 12; radius: 6
                color: "#32d74b"
                opacity: 0.18
                visible: cpuRoot.connected
            }
            Rectangle {
                anchors.centerIn: parent
                width: 6; height: 6; radius: 3
                color: isMacOS ? (cpuRoot.connected ? MacColors.green : MacColors.red) : cpuRoot.connected ? "#32d74b" : "#ff453a"
            }
        }

        Text {
            anchors.verticalCenter: parent.verticalCenter
            // Short device name, as in the Windows Console: "DSPi (last 8 of the serial)"
            text: {
                if (!cpuRoot.connected)
                    return bridge.availableSerials.length > 0 ? "Disconnected"
                         : cpuRoot.noAccess ? "No Permission" : "No Devices"
                return cpuRoot.fullName
            }
            font.pixelSize: 11
            color: isMacOS ? (!cpuRoot.connected && bridge.availableSerials.length === 0 ? MacColors.red : MacColors.label) : cpuRoot.connected ? Qt.rgba(1, 1, 1, 0.65)
                 : bridge.availableSerials.length === 0 ? "#ff453a" : Qt.rgba(1, 1, 1, 0.45)
        }

        ToolTip.visible: statusHover.containsMouse
        ToolTip.delay: 500
        ToolTip.text: statusTip
        // -- WP QD37 -- (review c4o N1) During a hub session a right-click does nothing
        // (bridge.rescan) and devices are chosen in Device > Devices…
        readonly property string statusTip: bridge.hubSession
                      ? (cpuRoot.connected ? "Connected through a hub · Serial " + bridge.selectedSerial : "On a hub, no device chosen yet.")
                        + "\nChoose devices in Device > Devices…"
                      : cpuRoot.connected
                      ? "Connected · Serial " + bridge.selectedSerial
                        + (bridge.availableSerials.length > 1 ? "\nClick to switch device. Right-click to reconnect." : "\nRight-click to reconnect.")
                      : cpuRoot.noAccess ? ("A DSPi is connected, but this app isn't allowed to open it.\n" + (isLinux ? "Click to install its udev rule." : "Click for help."))
                      : (bridge.connectionError || "Not connected. Right-click the device name to retry.")
    }

    // Several DSPis: click to choose one. Right-click rescans and reconnects.
    MouseArea {
        id: statusHover
        anchors.fill: status
        anchors.margins: -4
        hoverEnabled: true
        acceptedButtons: Qt.LeftButton | Qt.RightButton
        cursorShape: bridge.availableSerials.length > 1 || cpuRoot.noAccess ? Qt.PointingHandCursor : Qt.ArrowCursor
        onClicked: {
            if (mouse.button === Qt.RightButton) { bridge.rescan(); return }
            if (cpuRoot.noAccess) { root.showUdevHelp(); return }
            if (bridge.availableSerials.length > 1) deviceMenu.toggleAt(status)
        }
    }
    ChoiceMenu {
        id: deviceMenu
        parent: Overlay.overlay
        alignRight: true
        heading: "Devices"
        currentValue: bridge.selectedSerial
        options: bridge.availableSerials.map(function (s) { return { value: s, text: "DSPi " + s.slice(-8) } })
        onChosen: if (value !== bridge.selectedSerial) root.switchDevice(value)
    }
}
