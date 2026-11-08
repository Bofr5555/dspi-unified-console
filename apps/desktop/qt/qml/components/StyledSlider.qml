import QtQuick 2.15
import QtQuick.Controls 2.15

// Thin-track slider with a round knob.
Slider {
    id: sl
    property color accent: isMacOS ? MacColors.sliderFill : "#0a7cff"
    implicitHeight: 22
    topPadding: 0
    bottomPadding: 0

    // -- WP QD41 -- settled(): a change by the user has ended, commit it now.
    // The mouse's release, as pressedChanged was (a press that moved nothing
    // too); the release of a step key that moved the value (not an
    // auto-repeat); 400 ms after the last accessibility increase or decrease.
    // Once per change. Every keyboard and accessibility step sends moved(),
    // as a drag does, and leaves pressed alone, so pressed means the pointer.
    // Left/Right and Up/Down step once, PageUp/PageDown a tenth of the range,
    // Home/End go to the ends.
    signal settled()
    property bool stepPending: false
    function settle() {
        settleTimer.stop()
        if (!stepPending) return
        stepPending = false
        settled()
    }
    // Steps through increase()/decrease(), which keep a bound value bound
    function stepBy(n) {
        const old = value
        for (let i = 0; i < Math.abs(n); ++i) {
            const before = value
            if (n > 0) increase(); else decrease()
            if (value === before) break
        }
        if (value === old) return false
        stepPending = true
        moved()
        return true
    }
    readonly property int steps: Math.max(1, Math.ceil(Math.abs(to - from) / (stepSize > 0 ? stepSize : 0.1)))
    onPressedChanged: if (pressed) { stepPending = false; settleTimer.stop() } else settled()
    onActiveFocusChanged: if (!activeFocus) settle()
    Keys.onPressed: function (event) {
        const back = mirrored ? Qt.Key_Right : Qt.Key_Left
        const fwd = mirrored ? Qt.Key_Left : Qt.Key_Right
        const n = event.key === fwd || event.key === Qt.Key_Up ? 1
                : event.key === back || event.key === Qt.Key_Down ? -1
                : event.key === Qt.Key_PageUp ? Math.max(1, Math.round(steps / 10))
                : event.key === Qt.Key_PageDown ? -Math.max(1, Math.round(steps / 10))
                : event.key === Qt.Key_End ? steps
                : event.key === Qt.Key_Home ? -steps : 0
        if (n === 0) return
        // -- review c74 N1 -- during a pointer drag a step key does nothing: taken
        // here, so Qt's own handler cannot clear `pressed` and lose the drag's commit.
        event.accepted = true
        if (!sl.pressed) stepBy(n)
    }
    Keys.onReleased: function (event) {
        switch (event.key) {
        case Qt.Key_Left: case Qt.Key_Right: case Qt.Key_Up: case Qt.Key_Down:
        case Qt.Key_PageUp: case Qt.Key_PageDown: case Qt.Key_Home: case Qt.Key_End:
            event.accepted = true
            if (!sl.pressed && !event.isAutoRepeat) settle()
        }
    }
    Accessible.onIncreaseAction: if (stepBy(1)) settleTimer.restart()
    Accessible.onDecreaseAction: if (stepBy(-1)) settleTimer.restart()
    Timer { id: settleTimer; interval: 400; onTriggered: sl.settle() }

    background: Rectangle {
        x: sl.leftPadding
        y: (sl.height - height) / 2
        width: sl.availableWidth
        height: 4
        radius: 2
        color: isMacOS ? (sl.enabled ? Qt.rgba(1, 1, 1, 0.1) : Qt.rgba(1, 1, 1, 0.05)) : Qt.rgba(1, 1, 1, 0.14)
        Rectangle {
            width: sl.visualPosition * parent.width
            height: parent.height
            radius: 2
            color: isMacOS ? (sl.enabled ? sl.accent : MacColors.opacity(sl.accent, 0.5)) : sl.enabled ? sl.accent : Qt.rgba(1, 1, 1, 0.3)
        }
    }
    handle: Rectangle {
        x: sl.leftPadding + sl.visualPosition * (sl.availableWidth - width)
        y: (sl.height - height) / 2
        width: 18
        height: 18
        radius: 9
        color: isMacOS ? (sl.enabled ? Qt.rgba(0.6, 0.6, 0.6, 1) : Qt.rgba(0.408, 0.408, 0.408, 1)) : sl.pressed ? "#e8e8e8" : "white"
        border.color: Qt.rgba(0, 0, 0, 0.25)
        // -- WP QD22 -- Give the slider an Accessible.name
        FocusRing { radius: 11; shown: sl.visualFocus }
    }
}
