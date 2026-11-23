import QtQuick
import UConsole

// -- WP AIVIS -- Over a control: a frame for a moment when an AI agent has just
// changed what it shows (AiController.touched), on the device shown. `path` is a
// journal path ("out.gain") or the start of one ("out."); `indices` the first
// indices it must have ([] any). One made just after the change shows it too. It
// takes no clicks.
Rectangle {
    id: frame
    property string path
    property var indices: []
    // Instead of a path: any change on this device (a device's tab)
    property string device
    // Or any in this section of Modern's
    property string section
    readonly property bool lit: timer.running
    // How often it lit up, for the smoke driver
    property int times: 0

    anchors.fill: parent
    visible: lit
    z: 10
    color: "transparent"
    radius: Theme.radiusSmall
    border.width: 2
    border.color: Theme.accent

    function matches(dev, p, ix) {
        if (device.length > 0) return dev === device
        if (dev !== DeviceRegistry.activeId) return false
        if (section.length > 0) return AiController.sectionOf(p) === section
        if (!(path.endsWith(".") ? p.startsWith(path) : p === path)) return false
        for (let i = 0; i < indices.length; ++i)
            if (ix.length <= i || ix[i] !== indices[i]) return false
        return true
    }
    function light() {
        ++times
        timer.restart()
        pulse.restart()
    }
    Connections {
        target: AiController
        function onTouched(dev, p, ix) { if (frame.matches(dev, p, ix)) frame.light() }
    }
    Component.onCompleted: {
        const late = AiController.recentTouches
        for (let i = 0; i < late.length; ++i)
            if (matches(late[i].device, late[i].path, late[i].indices)) {
                light()
                return
            }
    }
    Timer {
        id: timer
        interval: 3000
    }
    SequentialAnimation on opacity {
        id: pulse
        running: false
        loops: 3
        NumberAnimation { to: 1; duration: 150 }
        NumberAnimation { to: 0.35; duration: 700; easing.type: Easing.InOutSine }
    }
    Accessible.ignored: true
}
