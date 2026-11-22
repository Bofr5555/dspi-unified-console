import QtQuick
import QtQuick.Layouts
import UConsole

// -- WP QP2 -- A number with its unit, edited by typing (the Mac's ValueField):
// Enter or leaving the field commits, Escape restores, a value that is not a
// number puts the shown one back. Ctrl+wheel steps by `scrollStep` (the
// Mac's Cmd-scroll; a plain wheel scrolls the list under it). `wholeSteps`
// keeps those steps on whole units (the delay field). Right-click resets to
// `resetValue` when it is set.
RowLayout {
    id: root
    property string label
    property string unit
    property real value: 0
    property int maxDecimals: 1
    property bool stripTrailingZeros: false
    property real scrollStep: 0.1
    property bool wholeSteps: false
    property real minValue: -Infinity
    property real maxValue: Infinity
    property real resetValue: NaN
    property int fieldWidth: 60
    // Shown instead of the value while set (a drag's live value)
    property real liveValue: NaN
    readonly property alias editing: field.activeFocus
    readonly property alias text: field.text
    signal committed(real value)

    spacing: 4

    // "1.50" -> "1.5", "1.000" -> "1.0" (or "1" with stripTrailingZeros)
    function format(v) {
        let s = Number(v).toFixed(maxDecimals)
        if (s.indexOf(".") < 0) return s
        s = s.replace(/0+$/, "")
        if (s.endsWith(".")) s = stripTrailingZeros ? s.slice(0, -1) : s + "0"
        return s === "-0" || s === "-0.0" ? s.slice(1) : s
    }
    function shown() { return format(isNaN(liveValue) ? value : liveValue) }
    function commitText(t) {
        const v = Number(String(t).trim().replace(",", "."))
        if (String(t).trim().length === 0 || !isFinite(v)) {
            settle()
            return
        }
        committed(v)
        settle()
    }
    // -- WP LAY4 -- The shown value, as what editing began with: a value that
    // arrives after Enter, with the focus still here, replaces it, and leaving
    // the field then has nothing typed to send
    function settle() {
        const all = field.text.length > 0 && field.selectedText === field.text
        field.text = shown()
        field._start = field.text
        if (all && field.activeFocus) field.selectAll()
    }
    // A Ctrl+wheel notch from `v` in direction `dir` (+1 or -1)
    function stepped(v, dir) {
        let n
        if (!wholeSteps) n = v + dir * scrollStep
        else if (v === Math.round(v)) n = Math.round(v + dir * scrollStep)
        else n = dir > 0 ? Math.max(Math.floor(v + scrollStep), Math.ceil(v)) : Math.min(Math.ceil(v - scrollStep), Math.floor(v))
        return Math.min(maxValue, Math.max(minValue, n))
    }

    BorderedField {
        id: field
        objectName: root.objectName.length ? root.objectName + ".field" : ""
        label: root.label
        selectAllOnFocus: true // -- WP QTTWEAK --
        Layout.preferredWidth: root.fieldWidth
        horizontalAlignment: TextInput.AlignRight
        font: Theme.fontMono
        color: activeFocus ? Theme.accentText : (enabled ? Theme.text : Theme.textDisabled)
        text: root.shown()
        onCommitted: t => root.commitText(t)
        Accessible.description: root.shown() + (root.unit.length ? " " + root.unit : "")

        WheelHandler {
            acceptedModifiers: Qt.ControlModifier
            enabled: field.enabled
            onWheel: event => {
                const dir = event.angleDelta.y > 0 ? 1 : event.angleDelta.y < 0 ? -1 : 0
                if (dir !== 0) root.committed(root.stepped(root.value, dir))
            }
        }
        TapHandler {
            acceptedButtons: Qt.RightButton
            enabled: !isNaN(root.resetValue) && field.enabled
            onTapped: root.committed(root.resetValue)
        }
    }
    Text {
        visible: root.unit.length > 0
        Layout.preferredWidth: 20
        text: root.unit
        font: Theme.fontSmall
        color: Theme.textMuted
    }
    // A new value from the device while not typing
    onValueChanged: {
        if (!field.activeFocus) field.text = shown()
        else if (field.text === field._start) settle() // -- WP LAY4 --
    }
    onLiveValueChanged: if (!field.activeFocus) field.text = shown()
}
