import QtQuick
import UConsole

// A GPIO picker: `choices` as HardwareController's pin lists give them
// ([{pin, text}]), `pin` the current one. `picked(pin)` on a person's choice.
ChoicePicker {
    id: picker
    property var pins: []
    property int pin: -1
    signal picked(int pin)

    implicitWidth: 92
    choices: pins.map(p => ({ value: p.pin, text: p.text }))
    value: pin
    missingText: pin >= 0 && pin < 255 ? qsTr("GPIO %1").arg(pin) : qsTr("Not set")
    onChosen: v => picker.picked(v)
}
