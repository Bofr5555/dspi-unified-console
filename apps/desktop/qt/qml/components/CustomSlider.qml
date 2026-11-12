import QtQuick
import QtQuick.Controls.Basic
import UConsole

// The app's slider (the Mac's CustomSlider and ParameterRow behaviour): the
// thumb follows every event; `valueMoved` sends values live during a drag, at most
// 30 a second with only the latest kept; `committed` fires once on release, on
// each key step and on a reset. A right-click resets to `resetValue` when one
// is set (the desktop gesture for the Mac's reset, PARITY M51).
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
    onPressedChanged: {
        if (!pressed) {
            coalesce.stop()
            _pending = NaN
            committed(value)
        }
    }

    TapHandler {
        acceptedButtons: Qt.RightButton
        enabled: !isNaN(slider.resetValue)
        onTapped: {
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

    Accessible.role: Accessible.Slider
    Accessible.name: label
}
