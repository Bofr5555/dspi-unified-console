import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A number with minus and plus steppers (the Mac's Stepper next to a value).
// Works in real units: `realValue` between `realFrom` and `realTo` in steps of
// `realStep`, shown with `decimals` places and `unit`. `committed(value)`
// fires on a step, a typed value or the wheel, not on binding changes.
SpinBox {
    id: control
    property string label
    property real realFrom: 0
    property real realTo: 100
    property real realStep: 1
    property int decimals: 0
    property string unit
    property real realValue: realFrom
    signal committed(real value)

    readonly property real scale: Math.pow(10, decimals)
    from: Math.round(realFrom * scale)
    to: Math.round(realTo * scale)
    stepSize: Math.max(1, Math.round(realStep * scale))
    value: Math.round(realValue * scale)
    editable: true
    wheelEnabled: true
    implicitHeight: Theme.controlHeight
    implicitWidth: 120
    font: Theme.fontMono
    focusPolicy: Qt.StrongFocus

    textFromValue: (v, locale) => Number(v / scale).toLocaleString(locale, "f", decimals) + (unit ? " " + unit : "")
    valueFromText: (text, locale) => {
        const t = unit ? text.replace(unit, "").trim() : text.trim()
        return Math.round(Number.fromLocaleString(locale, t) * scale)
    }
    validator: RegularExpressionValidator { regularExpression: /[-+]?[0-9]*[.,]?[0-9]*\s*\S*/ }
    onValueModified: {
        realValue = value / scale
        committed(realValue)
    }

    contentItem: TextInput {
        text: control.displayText
        font: control.font
        color: control.enabled ? Theme.text : Theme.textDisabled
        selectionColor: Theme.textSelection
        selectedTextColor: Theme.text
        horizontalAlignment: Qt.AlignHCenter
        verticalAlignment: Qt.AlignVCenter
        readOnly: !control.editable
        validator: control.validator
        inputMethodHints: Qt.ImhFormattedNumbersOnly
        // -- WP P1QT2 -- the field Tab reaches carries the spin box's name too
        Accessible.role: Accessible.EditableText
        Accessible.name: control.label
    }
    down.indicator: Rectangle {
        x: 0
        height: control.height
        width: height
        radius: Theme.radiusSmall
        color: control.down.pressed ? Theme.controlPressed : "transparent"
        Icon {
            anchors.centerIn: parent
            name: "minus"
            size: 12
            color: control.value > control.from && control.enabled ? Theme.text : Theme.textDisabled
        }
    }
    up.indicator: Rectangle {
        x: control.width - width
        height: control.height
        width: height
        radius: Theme.radiusSmall
        color: control.up.pressed ? Theme.controlPressed : "transparent"
        Icon {
            anchors.centerIn: parent
            name: "plus"
            size: 12
            color: control.value < control.to && control.enabled ? Theme.text : Theme.textDisabled
        }
    }
    background: Rectangle {
        radius: Theme.radiusSmall
        color: Theme.control
        border.width: control.activeFocus ? 2 : 1
        border.color: control.activeFocus ? Theme.focusRing : Theme.border
    }

    Accessible.role: Accessible.SpinBox
    Accessible.name: label
    Accessible.description: displayText
}
