import QtQuick 2.15

// Control Surfaces model helpers shared by the Control pages: the device's
// capability tables (`controlSurfaces.model`), the macOS Console's names and
// phrases, the wire encodings, and the rules for new and edited records.
// Records are plain objects with the core's field names (see core/src/cs.rs).
QtObject {
    id: cs

    readonly property var m: controlSurfaces.model
    // Changes only on connect / disconnect (bridge.connected notifies on
    // every status poll)
    readonly property bool connected: bridge.connected

    // ── Enums (firmware control_surfaces.h) ──
    readonly property int typeNone: 0
    readonly property int typeButton: 1
    readonly property int typeSwitch: 2
    readonly property int typePot: 3
    readonly property int typeEncoder: 4
    readonly property int typeLed: 5
    readonly property int typeLedPwm: 6
    readonly property int typeIr: 7
    readonly property int typeDisplay: 8
    readonly property int typeAuxOut: 9
    readonly property int typeAuxPwm: 10

    readonly property int actAdjust: 0
    readonly property int actStep: 1
    readonly property int actInc: 2
    readonly property int actDec: 3
    readonly property int actToggle: 4
    readonly property int actSet: 5
    readonly property int actFollow: 6
    readonly property int actTrigger: 7
    readonly property int actIndEquals: 8
    readonly property int actMomentary: 9
    readonly property int actIndAbove: 10
    readonly property int actIndLevel: 11

    readonly property int flagInvert: 0x01
    readonly property int flagReverse: 0x02
    readonly property int flagAccel: 0x08
    readonly property int flagGroup: 0x20
    readonly property int flagGroupAll: 0x80

    readonly property int kindBool: 1
    readonly property int kindEnum: 2

    readonly property int unitDb: 1
    readonly property int unitQ: 3
    readonly property int unitMs: 5

    readonly property int targetNone: 0
    readonly property int targetInput: 1
    readonly property int targetOutput: 2
    readonly property int targetDsp: 3
    readonly property int targetBand: 4
    readonly property int targetAux: 5

    readonly property int eventLong: 1

    readonly property int auxBootOn: 0x01
    readonly property int auxBootSaved: 0x02
    readonly property int auxLinear: 0x04

    readonly property int dmodeFixed: 0
    readonly property int dmodeCycleSelected: 1
    readonly property int dcfgOverlayAny: 0x01
    readonly property int dcfgEditGated: 0x02
    readonly property int dpageActive: 0x01
    readonly property int dpageGroup: 0x02
    readonly property int dpageLarge: 0x04
    readonly property int dpageBar: 0x08

    // Nouns used by name in the rules below
    readonly property int nounDisplayEdit: 55
    readonly property int nounPageValue: 56
    readonly property int nounAux: 68
    readonly property int nounAuxLevel: 69

    readonly property var adcPins: [26, 27, 28]
    readonly property var macroStepActions: [actSet, actToggle, actInc, actDec, actTrigger]
    readonly property int delayMaxSeconds: 6553       // the 0.1 s field, in whole seconds
    readonly property int displayMinDwell: 10         // 0.1 s units, cycle modes

    // ── Device tables ──
    readonly property bool supported: m.supported === true
    readonly property int slotCount: supported ? Math.min(16, m.maxBindings) : 0
    readonly property int irCount: supported ? Math.min(16, m.maxIr) : 0
    readonly property int groupCount: supported ? Math.min(8, m.maxGroups) : 0
    readonly property int macroCount: supported ? Math.min(8, m.maxMacros) : 0
    readonly property int stepCount: supported ? Math.min(8, m.maxSteps) : 0
    readonly property int pageCount: supported && m.display ? Math.min(16, m.display.maxPages) : 0
    readonly property int modelCount: supported && m.display ? m.display.modelCount : 0
    readonly property var types: supported ? m.types : []
    readonly property var nouns: supported ? m.nouns : []
    readonly property var bindings: supported ? m.bindings : []
    readonly property var names: supported ? m.names : []
    readonly property var irCommands: supported ? m.ir : []
    readonly property var groups: supported ? m.groups : []
    readonly property var macros: supported ? m.macros : []
    readonly property var status: supported ? m.status : ({})
    readonly property var ext: supported ? m.ext : ({})
    readonly property var display: supported ? m.display : ({})
    readonly property bool auxSupported: types.length > typeAuxPwm
    readonly property bool displaySupported: types.length > typeDisplay

    function nounDesc(n) { return n >= 0 && n < nouns.length ? nouns[n] : null }
    function isTargeted(nd) { return !!nd && nd.targetKind !== targetNone && nd.targetCount > 0 }
    function isSlotActive(slot) { return !!status.activeMask && ((status.activeMask >> slot) & 1) === 1 }
    function slotHealth(slot) { return status.slotStatus ? status.slotStatus[slot] : 0 }
    function isConfigured(b) { return !!b && b.type !== typeNone }
    function irConfigured(c) { return !!c && c.protocol !== 0 }
    function groupConfigured(g) { return !!g && g.targetKind !== targetNone && g.memberMask !== 0 }
    function isAuxType(t) { return t === typeAuxOut || t === typeAuxPwm }
    function isIndicatorType(t) { return t === typeLed || t === typeLedPwm }

    // ── Records ──
    readonly property var bindingKeys: ["type", "noun", "action", "flags", "gpio0", "gpio1", "event", "target",
        "index", "baseBright", "value", "step", "rangeMin", "rangeMax", "onDelay", "offDelay", "extras"]
    readonly property var irKeys: ["noun", "action", "flags", "target", "index", "protocol", "value", "step", "code"]
    readonly property var stepKeys: ["noun", "action", "flags", "target", "index", "value", "step", "preDelay"]

    function blank(keys) {
        var o = {}
        for (var i = 0; i < keys.length; i++) o[keys[i]] = 0
        return o
    }
    function emptyBinding() { var b = blank(bindingKeys); b.gpio1 = 0xFF; return b }
    function emptyIr() { return blank(irKeys) }
    function emptyStep() { return blank(stepKeys) }
    function copy(o) { return JSON.parse(JSON.stringify(o)) }
    function same(a, b, keys) {
        if (!a || !b) return a === b
        for (var i = 0; i < keys.length; i++) if (a[keys[i]] !== b[keys[i]]) return false
        return true
    }
    function sameBinding(a, b) { return same(a, b, bindingKeys) }
    function sameIr(a, b) { return same(a, b, irKeys) }
    function sameGroup(a, b) {
        return !!a && !!b && a.targetKind === b.targetKind && a.memberMask === b.memberMask && a.name === b.name
    }
    function sameMacro(a, b) {
        if (!a || !b || a.name !== b.name || a.steps.length !== b.steps.length) return false
        for (var i = 0; i < a.steps.length; i++) if (!same(a.steps[i], b.steps[i], stepKeys)) return false
        return true
    }
    function macroConfigured(mc) { return !!mc && (mc.steps.length > 0 || mc.name !== "") }

    // ── Names ──
    function typeName(t) {
        switch (t) {
        case typeNone: return qsTr("None")
        case typeButton: return qsTr("Push Button")
        case typeSwitch: return qsTr("Toggle Switch")
        case typePot: return qsTr("Potentiometer / Fader")
        case typeEncoder: return qsTr("Rotary Encoder")
        case typeLed: return qsTr("Indicator LED")
        case typeLedPwm: return qsTr("Dimmable LED")
        case typeIr: return qsTr("IR Remote")
        case typeDisplay: return qsTr("Display")
        case typeAuxOut: return qsTr("On/Off Output")
        case typeAuxPwm: return qsTr("Dimmable Output")
        default: return qsTr("Type %1").arg(t)
        }
    }
    function typeIcon(t) {
        switch (t) {
        case typeButton: return "cs-button"
        case typeSwitch: return "cs-switch"
        case typePot: return "cs-pot"
        case typeEncoder: return "cs-encoder"
        case typeLed: return "cs-led"
        case typeLedPwm: return "sun"
        case typeIr: return "cs-ir"
        case typeDisplay: return "cs-display"
        case typeAuxOut: return "power"
        case typeAuxPwm: return "sun"
        default: return "cs-pot"
        }
    }
    // Cool hues for inputs, warm for the LEDs (they are lights)
    function typeTint(t) {
        switch (t) {
        case typeButton: return Qt.rgba(0.20, 0.62, 0.74, 1)
        case typeSwitch: return Qt.rgba(0.15, 0.49, 0.62, 1)
        case typePot: return Qt.rgba(0.21, 0.49, 0.82, 1)
        case typeEncoder: return Qt.rgba(0.34, 0.37, 0.80, 1)
        case typeLed: return Qt.rgba(0.93, 0.63, 0.18, 1)
        case typeLedPwm: return Qt.rgba(0.90, 0.45, 0.20, 1)
        case typeIr: return Qt.rgba(0.55, 0.35, 0.72, 1)
        case typeDisplay: return Qt.rgba(0.016, 0.522, 0.435, 1)
        case typeAuxOut: return Qt.rgba(0.478, 0.353, 0.675, 1)
        case typeAuxPwm: return Qt.rgba(0.62, 0.40, 0.62, 1)
        default: return Qt.rgba(0.46, 0.53, 0.62, 1)
        }
    }

    // -- WP CG20 -- The core's names (cs_model::noun_name, in the snapshot): a few
    // read differently on a control than on an LED, a button clears clipping and
    // an LED shows it
    function nounName(n, type) {
        var nd = nounDesc(n)
        if (!nd || nd.name === undefined) return qsTr("Parameter %1").arg(n)
        return isIndicatorType(type) ? nd.name : nd.controlName
    }

    function actionName(a, noun) {
        var nd = nounDesc(noun)
        var isEnum = !!nd && nd.kind === kindEnum
        if (noun === nounPageValue) {
            if (a === actInc) return qsTr("Up")
            if (a === actDec) return qsTr("Down")
        }
        switch (a) {
        case actAdjust: return qsTr("Adjust")
        case actStep: return qsTr("Step")
        case actInc: return isEnum ? qsTr("Next") : qsTr("Increase")
        case actDec: return isEnum ? qsTr("Previous") : qsTr("Decrease")
        case actToggle: return qsTr("Toggle")
        case actSet: return qsTr("Set value")
        case actFollow: return qsTr("Follow position")
        case actTrigger: return qsTr("Trigger")
        case actIndEquals: return qsTr("Indicate")
        case actMomentary: return qsTr("Hold")
        case actIndAbove: return qsTr("Indicate above")
        case actIndLevel: return qsTr("Show level")
        default: return qsTr("Action %1").arg(a)
        }
    }

    function groupName(g) {
        var grp = groups[g]
        return grp && grp.name !== "" ? grp.name : qsTr("Group %1").arg(g + 1)
    }
    function macroName(i) {
        var mc = macros[i]
        return mc && mc.name !== "" ? mc.name : qsTr("Macro %1").arg(i + 1)
    }
    function auxName(slot) {
        return names[slot] ? names[slot] : qsTr("Aux %1").arg(slot + 1)
    }

    // ── Channels and targets ──
    function channelName(appId) {
        var n = appId >= 0 ? bridge.channelName(appId) : ""
        return n !== "" ? n : qsTr("Channel %1").arg(appId + 1)
    }
    // DSP channels are numbered inputs first, then outputs
    function dspName(w) {
        var ins = bridge.numInputChannels
        return w < ins ? channelName(bridge.inputAppId(w)) : channelName(2 + (w - ins))
    }
    function targetName(nd, t) {
        switch (nd.targetKind) {
        case targetInput: return channelName(bridge.inputAppId(t))
        case targetOutput: return channelName(2 + t)
        case targetAux: return auxName(t)
        default: return dspName(t)
        }
    }

    function memberList(mask) {
        var list = []
        for (var i = 0; i < 32; i++) if ((mask >>> i) & 1) list.push(i)
        return list
    }

    function groupChannelCount(kind) {
        for (var n = 0; n < nouns.length; n++) {
            var nd = nouns[n]
            if (nd.actions === 0 || nd.targetCount === 0) continue
            if (nd.targetKind === kind) return nd.targetCount
            if (kind === targetDsp && nd.targetKind === targetBand) return nd.targetCount
        }
        return 0
    }
    function groupChannelName(kind, ch) { return targetName({ targetKind: kind, targetCount: 255 }, ch) }

    // Macro step waits: 10 ms units
    function fmtSeconds(s) {
        if (s < 60) return qsTr("%1 s").arg(String(s < 10 ? Number(s.toPrecision(2)) : Math.round(s)))
        var mins = s / 60
        return qsTr("%1 min").arg(mins === Math.round(mins) ? mins.toFixed(0) : mins.toFixed(1))
    }

    // ── Rules: the core's (dspi_session::cs_rows, WP CG20) ──
    // A record's rows, and the record after a row changes. The bindings that call
    // these re-run when the model or the pin map changes.
    function describe(kind, slot, rec) {
        controlSurfaces.revision; bridge.hardware
        if (!rec) return { ok: false, rows: [] }
        var d = controlSurfaces.describe(kind, slot, rec)
        return d.ok ? d : { ok: false, rows: [], summary: "", title: "" }
    }
    function edit(kind, slot, rec, id, value) {
        var r = controlSurfaces.edit(kind, slot, rec, id, value)
        return r.ok ? r.draft : rec
    }
    function rowOf(d, id) {
        for (var i = 0; i < d.rows.length; i++) if (d.rows[i].id === id) return d.rows[i]
        return null
    }
    // A pick row's options for a CsPicker: section headings where they change
    function pickOptions(row) {
        var list = [], at = undefined
        if (!row) return list
        for (var i = 0; i < row.options.length; i++) {
            var o = row.options[i]
            if (o.disabled) continue
            if (o.section !== undefined && o.section !== at) { at = o.section; list.push({ header: at }) }
            list.push({ value: o.value, text: o.text })
        }
        return list
    }
    // The same as a CsPicker's families (a function picker)
    function pickCategories(row) {
        var out = []
        if (!row) return out
        for (var i = 0; i < row.options.length; i++) {
            var o = row.options[i]
            if (out.length === 0 || out[out.length - 1].text !== o.section) out.push({ text: o.section, options: [] })
            out[out.length - 1].options.push({ value: o.value, text: o.text })
        }
        return out
    }
    function makeBinding(type, slot) {
        var r = controlSurfaces.newBinding(slot, type)
        return r.ok ? r.draft : emptyBinding()
    }
    function newIrCommand() {
        var r = controlSurfaces.newRecord("ir")
        return r.ok ? r.draft : emptyIr()
    }
    function newStep() {
        var r = controlSurfaces.newRecord("step")
        return r.ok ? r.draft : emptyStep()
    }
    function newPage() {
        var r = controlSurfaces.newRecord("page")
        return r.ok ? r.draft : { noun: 0, target: 0, index: 0, flags: 0 }
    }
    function verbPhrase(b) { return describe("binding", 0, b).summary || "" }
    function irVerbPhrase(c) { return describe("ir", 0, c).summary || "" }
    function macroStepSummary(st) { return describe("step", 0, st).summary || "" }
    readonly property var realTypes: {
        var list = []
        for (var t = 1; t < types.length; t++) list.push(t)
        return list
    }

    // ── Wording ──
    function pinDetail(type) {
        switch (type) {
        case typePot: return qsTr("ADC pin (GPIO 26, 27, or 28), wiper to the pin.")
        case typeLed: case typeLedPwm: return qsTr("Output pin driving the LED.")
        case typeIr: return qsTr("GPIO wired to the receiver module's OUT (VCC to 3V3, GND to GND).")
        case typeAuxOut: return qsTr("Output pin driving the relay or MOSFET input.")
        case typeAuxPwm: return qsTr("PWM output pin driving the dimmer or fan input.")
        default: return qsTr("Wired between this GPIO and GND.")
        }
    }
    function invertTitle(type) {
        switch (type) {
        case typeLed: case typeLedPwm: return qsTr("Active-Low LED")
        case typePot: case typeEncoder: return qsTr("Pull-Down Wiring")
        case typeIr: return qsTr("Idle-Low Receiver")
        case typeAuxOut: case typeAuxPwm: return qsTr("Active-Low Output")
        default: return qsTr("Active-High Wiring")
        }
    }
    function invertDetail(type) {
        switch (type) {
        case typeLed: return qsTr("Drive the pin low to light the LED (LED wired to 3V3 through a resistor).")
        case typeLedPwm: return qsTr("Invert the PWM duty for an LED wired to 3V3 through a resistor.")
        case typePot: case typeEncoder: return qsTr("Wire the common terminal to 3V3 instead of GND (internal pull-down).")
        case typeIr: return qsTr("The receiver idles low and pulls high on a mark; default is the usual idle-high, active-low module.")
        case typeAuxOut: return qsTr("Drive the pin low to switch the load on, which is what most relay and opto-isolator boards expect.")
        case typeAuxPwm: return qsTr("Invert the PWM duty for a driver that switches on when the pin goes low.")
        default: return qsTr("Component wired to 3V3 with the internal pull-down; default is to GND with pull-up.")
        }
    }

    function displayModelName(model) {
        switch (model) {
        case 1: return qsTr("LCD 16x2 (HD44780)")
        case 2: return qsTr("LCD 20x4 (HD44780)")
        case 3: return qsTr("Character OLED 16x2")
        case 4: return qsTr("Character OLED 20x2")
        case 5: return qsTr("Character OLED 20x4")
        case 6: return qsTr("OLED 128x64 (SSD1306)")
        case 7: return qsTr("OLED 128x32 (SSD1306)")
        case 8: return qsTr("OLED 128x64 (SH1106)")
        default: return qsTr("Model %1").arg(model)
        }
    }
    function displayDefaultAddress(model) { return model === 1 || model === 2 ? 0x27 : 0x3C }
    function displayIsGraphic(model) { return model >= 6 }
    function hex(v, digits) { return ("00000000" + v.toString(16).toUpperCase()).slice(-digits) }
    function irProtocolName(p) {
        switch (p) {
        case 1: return "NEC"
        case 2: return "RC5"
        case 3: return "RC6"
        case 4: return qsTr("Generic")
        default: return qsTr("None")
        }
    }
    readonly property bool editGated: !!display.config && (display.config.flags & dcfgEditGated) !== 0

    function statusMessage(code) { return controlSurfaces.statusMessage(code) }
}
