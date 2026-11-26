import QtQuick
import QtQuick.Controls.Basic
import UConsole

// A number typed in its unit (the Pro layer's field, docs/UI-MODES.md and
// docs/GLOSSARY.md §4). It reads what people type:
// - a plain number in the field's unit: "120", "-6", "0,5";
// - the unit or a known one after it: "120 Hz", "2.5 kHz", "-6 dB", "10 ms",
//   "0.01 s"; "k" multiplies by a thousand ("2k");
// - a change of the value: "+=3", "-=3"; and "+3", "-3" as well where the
//   value cannot be negative (a gain's "-3" is the gain);
// then clamps to from..to (the limit badge shows that it did) and rounds to
// `decimals`. Up and Down nudge by `step`, with Shift by `coarseStep`; Enter
// or leaving the field commits, Escape puts the value back, Ctrl+Backspace
// gives `resetValue`.
//
//     NumericField { unit: "Hz"; from: 10; to: 24000; value: band.freq; onCommitted: v => set(v) }
FocusScope {
    id: field
    property real value: 0
    property real from: 0
    property real to: 100
    property real step: 1
    property real coarseStep: step * 10
    property int decimals: 1
    // The glossary's unit ("Hz", "dB", "dBFS", "ms", "%"), or "" for none (Q)
    property string unit: ""
    property real resetValue: NaN
    property string label
    // The last entry was clamped to the range
    property bool limited: false
    readonly property alias editing: input.activeFocus
    signal committed(real value)

    // Hz shows as kHz from 1000 up, as the glossary writes it
    function format(v) {
        if (unit === "Hz" && Math.abs(v) >= 1000) return (v / 1000).toFixed(Math.max(decimals, 2)) + " " + qsTr("kHz")
        const text = v.toFixed(decimals)
        return unit.length ? text + " " + unit : text
    }

    // What `text` means for a field at `current`: { ok, value, limited }
    function parse(text, current) {
        let t = String(text).trim().toLowerCase().replace(/\s+/g, "")
        if (!t.length) return { ok: false }
        let relative = 0
        if (t.startsWith("+=")) { relative = 1; t = t.slice(2) }
        else if (t.startsWith("-=")) { relative = -1; t = t.slice(2) }
        else if (from >= 0 && (t[0] === "+" || t[0] === "-")) { relative = t[0] === "+" ? 1 : -1; t = t.slice(1) }
        t = t.replace(/[\u2212\u2012\u2013\ufe63\uff0d]/g, "-")   // -- WP USR1 -- the minus sign
        if (t.indexOf(",") >= 0 && t.indexOf(".") < 0) t = t.replace(",", ".")
        const m = /^([+-]?(?:\d+\.?\d*|\.\d+))(k)?(khz|hz|dbfs|db|ms|s|%)?$/.exec(t)
        if (!m) return { ok: false }
        let v = parseFloat(m[1])
        if (m[2]) v *= 1000
        const suffix = m[3] || ""
        if (suffix === "khz") v *= 1000
        else if (suffix === "s" && unit === "ms") v *= 1000
        else if (suffix.length && suffix !== unit.toLowerCase()) {   // "s" only in a ms field (QPI4)
            // dB and dBFS name the same number; anything else is another unit
            const db = suffix.startsWith("db") && unit.toLowerCase().startsWith("db")
            if (!db && !(suffix === "hz" && unit === "Hz")) return { ok: false }
        }
        if (relative !== 0) v = current + relative * v
        const clamped = Math.max(from, Math.min(to, v))
        const scale = Math.pow(10, decimals)
        return { ok: isFinite(clamped), value: Math.round(clamped * scale) / scale, limited: clamped !== v }
    }

    function commit(v, wasLimited) {
        limited = wasLimited
        input.text = format(v)
        if (v !== value) committed(v)
    }
    function nudge(by) {
        const r = parse(String(value + by), 0)
        if (r.ok) commit(r.value, r.limited)
    }

    implicitWidth: 96
    implicitHeight: Theme.controlHeight
    activeFocusOnTab: true

    TextInput {
        id: input
        objectName: "NumericField.input"
        anchors.fill: parent
        anchors.leftMargin: 6
        anchors.rightMargin: badge.visible ? badge.width + 8 : 6
        verticalAlignment: TextInput.AlignVCenter
        horizontalAlignment: TextInput.AlignRight
        font: Theme.fontMono
        color: field.enabled ? Theme.text : Theme.textDisabled
        selectionColor: Theme.textSelection
        selectedTextColor: Theme.text
        selectByMouse: true
        clip: true
        focus: true
        text: field.format(field.value)
        // -- WP USR1 -- leaving the field commits what was typed, as Enter does
        // (it went back to the old value); Escape still drops it
        property bool edited: false
        onTextEdited: edited = true
        onActiveFocusChanged: {
            if (activeFocus) {
                edited = false
                selectAll()
                return
            }
            const r = edited ? field.parse(text, field.value) : { ok: false }
            edited = false
            if (r.ok) field.commit(r.value, r.limited)
            else text = field.format(field.value)
        }
        onAccepted: {
            // -- WP INTE -- (REVE C1) only what was typed is written: Enter with
            // nothing typed shows the value again (it parsed the rounded text and
            // wrote that rounded value over the device's)
            const typed = edited
            edited = false   // -- WP USR1 --
            const r = typed ? field.parse(text, field.value) : { ok: false }
            if (r.ok) field.commit(r.value, r.limited)
            else text = field.format(field.value)
            selectAll()
        }
        Keys.onEscapePressed: event => {
            edited = false   // -- WP USR1 --
            text = field.format(field.value)
            field.limited = false
            focus = false
            event.accepted = true
        }
        Keys.onUpPressed: event => field.nudge(event.modifiers & Qt.ShiftModifier ? field.coarseStep : field.step)
        Keys.onDownPressed: event => field.nudge(-(event.modifiers & Qt.ShiftModifier ? field.coarseStep : field.step))
        Keys.onPressed: event => {
            if (event.key === Qt.Key_Backspace && (event.modifiers & Qt.ControlModifier) && !isNaN(field.resetValue)) {
                field.commit(field.resetValue, false)
                event.accepted = true
            }
        }
    }
    // The value moved under the field while it was not being typed in
    onValueChanged: if (!input.activeFocus) input.text = format(value)

    Rectangle {
        z: -1
        anchors.fill: parent
        // -- WP DL0 -- Modern: no outline but the focus ring and the held-to-range one
        radius: Theme.radiusControl
        color: Theme.control
        border.width: input.activeFocus ? Theme.focusRingWidth : field.limited ? 1 : Theme.lineWidth
        border.color: input.activeFocus ? Theme.focusRing : (field.limited ? Theme.warning : Theme.border)
    }

    // The limit badge: the entry was outside the range and was held to it
    Rectangle {
        id: badge
        objectName: "NumericField.limit"
        visible: field.limited
        anchors.right: parent.right
        anchors.rightMargin: 4
        anchors.verticalCenter: parent.verticalCenter
        width: 14
        height: 14
        radius: 7
        color: "transparent"
        border.color: Theme.warning
        Text {
            anchors.centerIn: parent
            text: "!"
            font: Theme.fontCaption
            color: Theme.warning
        }
        HelpTip {
            //: %1 and %2 are the range's ends with their unit
            text: qsTr("Held to the range %1 to %2").arg(field.format(field.from)).arg(field.format(field.to))
            visible: badgeHover.hovered
        }
        HoverHandler { id: badgeHover }
    }

    Accessible.role: Accessible.SpinBox
    Accessible.name: label
    Accessible.description: input.text
}
