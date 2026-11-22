import QtQuick
import QtQuick.Controls.Basic
import UConsole

// The app's slider (the Mac's CustomSlider and ParameterRow behaviour): the
// thumb follows every event; `valueMoved` sends values live during a drag, at most
// 30 a second with only the latest kept; `committed` fires once on release, on
// each key step and on a reset. A right-click resets to `resetValue` when one
// is set (the desktop gesture for the Mac's reset, PARITY M51). A drag is one
// undo step (DeviceModel.beginDrag/endDrag); a key step or a reset is its own.
Slider {
    id: slider
    property string label
    property real resetValue: NaN
    signal valueMoved(real value)
    signal committed(real value)

    implicitWidth: 160
    implicitHeight: 18
    focusPolicy: Qt.StrongFocus

    property real _pending: NaN
    Timer {
        id: coalesce
        interval: 33
        onTriggered: {
            if (!isNaN(slider._pending)) {
                const v = slider._pending
                slider._pending = NaN
                slider.valueMoved(v)
                coalesce.start()
            }
        }
    }

    onMoved: {
        if (_left) return   // -- WP MDQT2a --
        if (pressed) {
            if (coalesce.running) {
                _pending = value
            } else {
                valueMoved(value)
                coalesce.start()
            }
        } else {
            // A key step: no drag to wait for
            committed(value)
        }
    }
    // -- WP QTGAP1 -- whether this slider holds DeviceModel's drag mark
    property bool _dragging: false
    onPressedChanged: {
        if (pressed) {
            _dragging = true
            DeviceModel.beginDrag()
        } else if (_left) {   // -- WP MDQT2a --
            _left = false
        } else {
            coalesce.stop()
            _pending = NaN
            committed(value)
            if (_dragging) {
                _dragging = false
                DeviceModel.endDrag()
            }
        }
    }
    Component.onDestruction: if (_dragging) DeviceModel.endDrag()
    // -- WP MDQT2a -- Held while another device becomes the active one: the value
    // is committed to the one left, and nothing of the rest of the hold reaches
    // the next one
    property bool _left: false
    Connections {
        target: slider._dragging ? DeviceRegistry : null
        function onActiveLeaving() {
            if (!slider._dragging) return
            coalesce.stop()
            slider._pending = NaN
            slider.committed(slider.value)
            slider._dragging = false
            slider._left = true
            DeviceModel.endDrag()
        }
    }

    // Above the handle and the track: the Slider takes any press it sees and
    // moves to it, so the right button is caught before it gets there. The
    // left button passes through.
    MouseArea {
        z: 1
        acceptedButtons: Qt.RightButton
        enabled: !isNaN(slider.resetValue)
        anchors.fill: parent
        onClicked: {
            slider.value = slider.resetValue
            slider.committed(slider.value)
        }
    }

    background: Rectangle {
        x: slider.leftPadding
        y: slider.topPadding + slider.availableHeight / 2 - height / 2
        width: slider.availableWidth
        height: 4
        radius: 2
        color: Theme.border
        Rectangle {
            width: slider.visualPosition * parent.width
            height: parent.height
            radius: 2
            color: slider.enabled ? Theme.accent : Theme.textDisabled
        }
    }
    handle: Rectangle {
        x: slider.leftPadding + slider.visualPosition * (slider.availableWidth - width)
        y: slider.topPadding + slider.availableHeight / 2 - height / 2
        width: 14
        height: 14
        radius: 7
        color: slider.pressed ? Theme.controlPressed : Theme.control
        border.width: slider.visualFocus ? 2 : 1
        border.color: slider.visualFocus ? Theme.focusRing : Theme.border
    }

    // -- WP QP18 -- (QD41) Up and Down step a horizontal slider too, as Right
    // and Left do, and commit; so do assistive technology's increase and decrease
    function stepBy(up) {
        if (!enabled) return
        const before = value
        if (up) increase()
        else decrease()
        if (value !== before) {
            moved()
        }
    }
    Keys.onUpPressed: event => {
        if (orientation === Qt.Horizontal) stepBy(true)
        else event.accepted = false
    }
    Keys.onDownPressed: event => {
        if (orientation === Qt.Horizontal) stepBy(false)
        else event.accepted = false
    }

    Accessible.role: Accessible.Slider
    Accessible.name: label
    Accessible.onIncreaseAction: stepBy(true)
    Accessible.onDecreaseAction: stepBy(false)
}
