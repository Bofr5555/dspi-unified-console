import QtQuick
import UConsole

// -- WP QP6 -- The matrix's compact number field (the Mac's CompactGainField
// and CompactDelayField): shows "-3 dB" or "1.5 ms", the bare number while
// typing (all of it selected), Enter or leaving commits, Escape restores, a
// text that is not a number puts the shown value back. The wheel steps the
// value while the field is not being typed in: gains by `wheelStep` dB,
// delays on whole milliseconds. A right-click resets to `resetValue`.
//
//     MatrixField { kind: "gain"; value: 0; onCommitted: v => ... }
BorderedField {
    id: field
    // "gain" (signed, two decimals at most) or "delay" (as the core keeps it)
    property string kind: "gain"
    property real value: 0
    property real minValue: kind === "gain" ? -60 : 0
    property real maxValue: kind === "gain" ? 12 : Infinity
    property real wheelStep: kind === "gain" ? 0.5 : 1
    property real resetValue: 0
    signal valueCommitted(real value)

    readonly property string unit: kind === "gain" ? qsTr("dB") : qsTr("ms")

    // "0", "+1.5", "-3" (gain); "0", "0.5", "12" (delay, four decimals at most)
    function format(v) {
        if (kind === "gain") {
            if (v === 0) return "0"
            let s = (v > 0 ? "+" : "") + Number(v).toFixed(2)
            s = s.replace(/0+$/, "")
            return s.endsWith(".") ? s.slice(0, -1) : s
        }
        let d = Number(v).toFixed(4).replace(/0+$/, "")
        return d.endsWith(".") ? d.slice(0, -1) : d
    }
    function shown() { return activeFocus ? format(value) : format(value) + " " + unit }
    function parse(t) {
        const cleaned = String(t).replace(unit, "").trim().replace(",", ".")
        const v = Number(cleaned)
        return cleaned.length && isFinite(v) ? v : NaN
    }
    // A delay step stays on the whole-millisecond grid from a fractional start
    function stepped(v, dir) {
        let n
        if (kind !== "delay") n = v + dir * wheelStep
        else if (v === Math.round(v)) n = Math.round(v + dir * wheelStep)
        else n = dir > 0 ? Math.max(Math.floor(v + wheelStep), Math.ceil(v)) : Math.min(Math.ceil(v - wheelStep), Math.floor(v))
        return Math.min(maxValue, Math.max(minValue, n))
    }
    function send(v) {
        if (Math.abs(v - value) > 1e-6) valueCommitted(v)
    }

    implicitWidth: kind === "gain" ? 62 : 56
    horizontalAlignment: TextInput.AlignHCenter
    font: Theme.fontMono
    color: activeFocus ? Theme.accentText : (enabled ? Theme.text : Theme.textDisabled)
    text: shown()
    Accessible.description: format(value) + " " + unit

    onActiveFocusChanged: {
        text = shown()
        if (activeFocus) selectAll()
    }
    onCommitted: t => {
        const v = parse(t)
        // A delay past the device's line is sent as typed: the core refuses it
        if (!isNaN(v)) send(kind === "gain" ? Math.min(maxValue, Math.max(minValue, v)) : Math.max(minValue, v))
        text = shown()
    }
    onValueChanged: if (!activeFocus) text = shown()

    WheelHandler {
        enabled: field.enabled && !field.activeFocus
        acceptedDevices: PointerDevice.Mouse | PointerDevice.TouchPad
        onWheel: event => {
            const dir = event.angleDelta.y > 0 ? 1 : event.angleDelta.y < 0 ? -1 : 0
            if (dir !== 0) field.send(field.stepped(field.value, dir))
        }
    }
    TapHandler {
        acceptedButtons: Qt.RightButton
        enabled: field.enabled
        onTapped: field.send(field.resetValue)
    }
}
